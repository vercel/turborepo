//! The graph through turbo's cache: CI puts the graph of a commit into turbo's
//! cache after a build, and a checkout without a graph fetches the graph of the
//! nearest commit before its first one. Everything turbo's cache does applies:
//! the local cache (`.turbo/cache`, `--cache-dir`), the remote cache,
//! `--cache=` and artifact signing.
//!
//! - **What travels** is one packed graph
//!   ([`turborepo_trace_store::artifact`]): the manifest, the shards and the
//!   recorded language arguments, under checksums. Never a cache keyed by file
//!   stamps (those live in a per-machine directory).
//! - **The key** is a digest of the trace build id (turbo's version and the
//!   format versions of everything that decides shard contents), the operating
//!   system, the architecture and the commit SHA ([`graph_key`]).
//! - **Which commit**: turbo's cache matches keys exactly, so a fetch tries, in
//!   order, `HEAD`, the merge base of the base ref and `HEAD`, and then the
//!   merge base's first-parent ancestors, up to [`MAX_ANCESTORS`] of them, with
//!   one batched existence check ([`candidates`]).
//! - **Trust**: a restored graph is refreshed against the working tree before
//!   it answers anything: every shard whose inputs differ from the restored
//!   one's is traced again, and shards that name the checkout's location are
//!   left out. So which graph is restored only changes how long the refresh
//!   takes, never the selection, and a graph is trusted as far as the build
//!   cache it comes from.
//! - **Who writes**: CI only (`turborepo_ci::is_ci`), when the cache takes
//!   writes and holds no graph of `HEAD` yet. A developer's machine fetches
//!   what CI wrote and never uploads a graph.
//!
//! Nothing here fails a command: a graph that cannot be fetched, installed or
//! put is reported on stderr, and the command goes on with a normal build.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_cache::AsyncCache;
use turborepo_scm::SCM;
use turborepo_trace_store::{
    Manifest,
    artifact::{self, ArtifactError, Header, PackOptions},
};

use crate::{
    Changes,
    adapter::resolve_base,
    langs::{self, LangArgs},
    lockdiff::git,
};

/// First-parent ancestors of the merge base a fetch tries.
pub const MAX_ANCESTORS: usize = 20;

/// The packed graph's name in the graph directory, while it is fetched or put.
const FILE: &str = "transport.graph";

/// The graph directory of the repository at `root`.
fn graph_dir(root: &Path) -> PathBuf {
    root.join(turborepo_trace_store::DEFAULT_GRAPH_PATH)
}

/// The cache key of the graph of `sha` built by this binary on this platform.
pub fn graph_key(sha: &str) -> String {
    let id = format!(
        "turbo-trace\n{}\n{}\n{}\n{sha}",
        turborepo_trace_store::build::build_id(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    format!("{:016x}", xxhash_rust::xxh3::xxh3_64(id.as_bytes()))
}

fn rev(root: &Path, args: &[&str]) -> Option<String> {
    let out = git(root, args).ok()?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// The commits whose graph a build at `root` can start from, best first:
/// `HEAD`, the merge base of the base ref (as `changes` resolve it, through
/// turbo's SCM layer) and
/// `HEAD`, and the merge base's first-parent ancestors (at most
/// [`MAX_ANCESTORS`]). Without a base (`--changed`, or one that does not
/// resolve), `HEAD`'s first-parent ancestors. Empty outside a git repository.
pub fn candidates(root: &AbsoluteSystemPath, scm: &SCM, changes: &Changes) -> Vec<String> {
    let root = root.as_std_path();
    let Some(head) = rev(root, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]) else {
        return Vec::new();
    };
    let base = match changes {
        Changes::Scm { base, .. } => resolve_base(scm, base.as_deref()).ok(),
        Changes::Files(_) => None,
    };
    let start = base
        .and_then(|b| rev(root, &["merge-base", "--end-of-options", &b, "HEAD"]))
        .unwrap_or_else(|| head.clone());
    let count = format!("--max-count={}", MAX_ANCESTORS + 1);
    let mut out = vec![head];
    if let Ok(list) = git(
        root,
        &[
            "rev-list",
            "--first-parent",
            &count,
            "--end-of-options",
            &start,
        ],
    ) {
        out.extend(
            list.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned),
        );
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|sha| seen.insert(sha.clone()));
    out
}

/// A graph installed from turbo's cache.
#[derive(Clone, Debug)]
pub struct Restored {
    /// The commit it was built at.
    pub sha: String,
    /// Its place among the candidates (0: `HEAD`).
    pub distance: usize,
    /// Shards installed.
    pub shards: usize,
    /// Shards left out because they name the directory they were built in.
    pub relocated: usize,
    /// Bytes fetched.
    pub bytes: u64,
    /// Milliseconds: the existence check, the fetch, the install.
    pub timings_ms: [u64; 3],
}

impl std::fmt::Display for Restored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let short = self.sha.get(..12).unwrap_or(&self.sha);
        write!(
            f,
            "restored the graph of {short} ({} shard(s), {:.1} MiB, {} ms)",
            self.shards,
            self.bytes as f64 / f64::from(1 << 20),
            self.timings_ms.iter().sum::<u64>()
        )?;
        if self.relocated > 0 {
            write!(
                f,
                "; {} name the checkout they were built in and are traced again",
                self.relocated
            )?;
        }
        Ok(())
    }
}

/// Installs the packed graph `file` into `out` for the repository at `root`:
/// only when it was built by this binary with the options this host computes
/// the same digest for (otherwise no shard of it could be reused).
fn install(file: &Path, root: &Path, out: &Path) -> Result<artifact::UnpackReport, ArtifactError> {
    let expect = |h: &Header| -> Result<String, String> {
        let lang: LangArgs = match h.extra.get(LangArgs::FILE) {
            Some(text) => serde_json::from_str(text)
                .map_err(|e| format!("its language flags do not parse: {e}"))?,
            None => LangArgs::default(),
        };
        turborepo_trace_store::build::options_digest_of(
            &crate::graph::graph_options(root, out, &h.options),
            &langs::all(&lang),
        )
        .map_err(|e| e.to_string())
    };
    artifact::unpack(file, out, root, &expect, &[LangArgs::FILE], false)
}

/// Fetches and installs the graph of the first of `candidates` that turbo's
/// cache holds, when the repository at `root` has no graph yet. `Ok(None)`:
/// there is a graph already, or the cache holds none of the candidates.
pub async fn restore(
    root: &AbsoluteSystemPath,
    cache: &AsyncCache,
    candidates: &[String],
) -> Result<Option<Restored>, String> {
    let out = graph_dir(root.as_std_path());
    if candidates.is_empty() || Manifest::read(&out).is_some() {
        return Ok(None);
    }
    let t0 = Instant::now();
    let keys: Vec<String> = candidates.iter().map(|sha| graph_key(sha)).collect();
    let hits = cache.batch_exists(&keys).await;
    let Some(distance) = hits.iter().position(Option::is_some) else {
        return Ok(None);
    };
    let t1 = Instant::now();
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let anchor = AbsoluteSystemPathBuf::try_from(out.as_path()).map_err(|e| e.to_string())?;
    let file = out.join(FILE);
    let fetched = cache
        .fetch(&anchor, &keys[distance])
        .await
        .map_err(|e| e.to_string());
    let fetched = match fetched {
        Ok(Some(_)) if file.is_file() => true,
        Ok(_) => false,
        Err(e) => {
            let _ = std::fs::remove_file(&file);
            return Err(format!("cannot fetch the graph: {e}"));
        }
    };
    if !fetched {
        let _ = std::fs::remove_file(&file);
        return Ok(None);
    }
    let t2 = Instant::now();
    let bytes = std::fs::metadata(&file).map_or(0, |m| m.len());
    let installed =
        turborepo_rayon_compat::block_in_place(|| install(&file, root.as_std_path(), &out));
    let _ = std::fs::remove_file(&file);
    let r = installed.map_err(|e| format!("the fetched graph is not used ({e})"))?;
    Ok(Some(Restored {
        sha: candidates[distance].clone(),
        distance,
        shards: r.header.shards,
        relocated: r.relocated,
        bytes,
        timings_ms: [
            (t1 - t0).as_millis() as u64,
            (t2 - t1).as_millis() as u64,
            t2.elapsed().as_millis() as u64,
        ],
    }))
}

/// The commit a graph built now is put under: `HEAD` (the graph is built from
/// the working tree, whose commit it is).
pub fn head(root: &AbsoluteSystemPath) -> Option<String> {
    rev(
        root.as_std_path(),
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )
}

/// Whether a graph built now is put into turbo's cache: in CI only.
pub fn should_save() -> bool {
    turborepo_ci::is_ci()
}

/// Puts the graph of the repository at `root` into turbo's cache as the graph
/// of `sha`, unless the cache holds one already; waits until it is written.
/// `duration_ms`: what the build took (turbo's cache records the time saved).
/// Returns whether it put one.
pub async fn save(
    root: &AbsoluteSystemPath,
    cache: &AsyncCache,
    sha: &str,
    duration_ms: u64,
) -> Result<bool, String> {
    let key = graph_key(sha);
    if matches!(cache.exists(&key).await, Ok(Some(_))) {
        return Ok(false);
    }
    let out = graph_dir(root.as_std_path());
    if Manifest::read(&out).is_none() {
        return Ok(false);
    }
    let file = out.join(FILE);
    let opts = PackOptions {
        label: Some(sha.to_owned()),
        extra: vec![LangArgs::FILE.to_owned()],
        ..PackOptions::default()
    };
    turborepo_rayon_compat::block_in_place(|| artifact::pack(&out, &file, &opts))
        .map_err(|e| format!("cannot pack the graph: {e}"))?;
    let anchor = AbsoluteSystemPathBuf::try_from(out.as_path()).map_err(|e| e.to_string())?;
    let files = vec![AnchoredSystemPathBuf::from_raw(FILE).map_err(|e| e.to_string())?];
    let put = cache.put(anchor, key, files, duration_ms).await;
    let written = match put {
        Ok(()) => cache.wait().await,
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&file);
    written.map_err(|e| format!("cannot put the graph: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_differ_by_commit_and_are_hex() {
        let a = graph_key("0123456789abcdef0123456789abcdef01234567");
        let b = graph_key("1123456789abcdef0123456789abcdef01234567");
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, graph_key("0123456789abcdef0123456789abcdef01234567"));
    }
}
