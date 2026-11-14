//! The composed graph of the previous snapshot, reused across an edit that
//! composition cannot see.
//!
//! The first graph open after any change of the manifest used to compose
//! everything again (nodes, edges, hubs, the probe and membership tables, task
//! links): most of what "edit a file, ask what is affected" costs on a large
//! repository. But most edits change nothing composition reads: a function
//! body, a test case, a comment. The package's shard gets a new key (its file's
//! hash is in it) and new line numbers, and the same files, references,
//! edges, probes, memberships, tasks and hubs.
//!
//! Composition ([`super::Graph::from_manifest`], the tables, task linking)
//! reads a shard's records, never a file's content hash, its cached extraction,
//! or the line and byte span of a reference, an edge or an uncertainty (those
//! are read from the shards when a query asks for them). So when the new
//! manifest differs from the one the cache was composed for only in the keys of
//! some shards, and each of those shards equals its predecessor with exactly
//! those fields left out ([`same_for_composition`]), the cached parts are the
//! parts a fresh composition would produce. They are taken as they are, and
//! both caches are written again under the new keys.
//!
//! Everything else composes from scratch: another walked-file list (a file
//! added or removed), another set of packages, a predecessor that the store no
//! longer holds, or any other difference between the two shards. The comparison
//! is on owned copies of the two shards with the unread fields cleared, so a
//! field added to a shard record later is compared unless someone decides
//! otherwise here.
//!
//! The derived layers (working directories, the environment registry, hints)
//! read more of a shard (the facts of its extractions) and keep their own cache
//! key, which still covers every shard key. A layer that knows which files'
//! contents it read can be carried over all the same: the graph says which
//! files changed between the two snapshots ([`super::Graph::carried`],
//! [`super::Graph::carried_patch`]).
//!
//! What a file does with the mock APIs of a test runner is stored as marker
//! edges of the file (`mock:…` specifiers of unresolved edges, `crate::fw`),
//! and the specifier of an edge is something composition reads: a marker more,
//! less or other composes again. Two kinds of marker name the line of their
//! call (`mock:replaces`, `mock:hazard`), so a call that moves in a test
//! composes again as well; the others name no line. The mock
//! layer ([`super::mocks`]) reads the composed graph only and is keyed like the
//! other derived layers: it is computed again for every new manifest.

use std::path::Path;

use super::cache;
use crate::{
    manifest::Manifest,
    shard::{self, LoadedShard, Shard},
};

/// The manifest the composed cache was written for, next to it.
const STAMP: &str = "composed.for";
/// Shards that may differ for a reuse to be tried, and their bytes: beyond
/// these, comparing costs what composing does.
const MAX_CHANGED: usize = 64;
const MAX_CHANGED_BYTES: usize = 48 << 20;

/// The key in the header of a composed cache file.
fn cache_key(path: &Path) -> Option<u128> {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    Some(u128::from_le_bytes(head[8..24].try_into().ok()?))
}

/// Records the manifest the composed cache now holds the parts of.
pub(super) fn stamp(out: &Path, manifest: &Manifest) {
    if let Ok(bytes) = serde_json::to_vec(manifest) {
        let _ = crate::write_atomic(&crate::cache_dir(out).join(STAMP), &bytes);
    }
}

/// `s` without what composition does not read: the key, each file's content
/// hash and cached extraction, and the positions (line, byte span) of
/// references, edges and uncertainties.
fn for_composition(mut s: Shard) -> Shard {
    s.key.clear();
    // The observation states a refresh validates files with: they hold content
    // hashes (of configs, of a test's own source), and composition reads none
    // of them.
    s.obs.states = Default::default();
    for f in &mut s.files {
        f.hash = None;
        f.extraction = None;
        for r in &mut f.refs {
            r.line = 0;
            for e in &mut r.edges {
                e.span = [0, 0];
            }
        }
        for u in &mut f.uncertainties {
            u.line = 0;
            u.span = [0, 0];
        }
    }
    s
}

/// Whether two shards are equal in everything composition reads.
pub(super) fn same_for_composition(a: &LoadedShard, b: &LoadedShard) -> bool {
    // Cheap first: a file or a table record more or less.
    let (x, y) = (a.get(), b.get());
    if x.files.len() != y.files.len()
        || x.probes.len() != y.probes.len()
        || x.memberships.len() != y.memberships.len()
        || x.configs.len() != y.configs.len()
        || x.tasks.len() != y.tasks.len()
        || x.hubs.len() != y.hubs.len()
    {
        return false;
    }
    for_composition(a.to_owned()) == for_composition(b.to_owned())
}

/// What a snapshot takes from the one before it ([`across_edit`]).
pub(super) struct Taken {
    /// The composed parts.
    pub composed: cache::Composed,
    /// The manifest they were composed for.
    pub old: Manifest,
    /// The files whose contents differ between the two snapshots, as (shard,
    /// file) indexes. Everything else a file's record holds is either equal
    /// (what composition reads) or a function of its contents (its hash,
    /// its extraction, lines and spans).
    pub changed: Vec<(u32, u32)>,
}

/// The previous snapshot's composed parts and its manifest, when `manifest`
/// differs from it only in shards that are the same for composition (see the
/// module docs). `shards` are `manifest`'s; `pack` is the store's pack as it is
/// now (it still holds the shards that were replaced: call this before the pack
/// is compacted).
pub(super) fn across_edit(
    out: &Path,
    manifest: &Manifest,
    shards: &[LoadedShard],
    pack: Option<&shard::Pack>,
) -> Option<Taken> {
    if turborepo_trace_core::debug::var_os("NO_COMPOSED_CACHE").is_some()
        || turborepo_trace_core::debug::var_os("NO_COMPOSE_REUSE").is_some()
    {
        return None;
    }
    // The cache is this manifest's already: the usual case, decided from 24 bytes.
    if cache_key(&crate::cache_dir(out).join("composed")) == Some(cache::key(manifest)) {
        return None;
    }
    let old: Manifest =
        serde_json::from_slice(&std::fs::read(crate::cache_dir(out).join(STAMP)).ok()?).ok()?;
    if old.files != manifest.files
        || old.shards.len() != manifest.shards.len()
        || !old.names_are_canonical()
    {
        return None;
    }
    let mut changed = Vec::new();
    for (i, (a, b)) in old.shards.iter().zip(&manifest.shards).enumerate() {
        if (&a.kind, &a.name, &a.version, &a.root) != (&b.kind, &b.name, &b.version, &b.root) {
            return None;
        }
        if a.key != b.key {
            changed.push(i);
        }
    }
    if changed.is_empty()
        || changed.len() > MAX_CHANGED
        || changed.iter().map(|&i| shards[i].len()).sum::<usize>() > MAX_CHANGED_BYTES
    {
        return None;
    }
    let store = out.join("store");
    let mut files = Vec::new();
    for &i in &changed {
        let before = shard::load_from(&store, pack, &old.shards[i].key).ok()?;
        if !same_for_composition(&before, &shards[i]) {
            return None;
        }
        // The same files in the same order (they were just compared): the ones with
        // other contents. A file without a recorded hash counts as changed.
        let hash = |f: &shard::ArchivedFileRec| {
            f.hash
                .as_ref()
                .map(|h| [h[0].to_native(), h[1].to_native()])
        };
        for (j, (a, b)) in before
            .get()
            .files
            .iter()
            .zip(shards[i].get().files.iter())
            .enumerate()
        {
            if hash(a).is_none() || hash(a) != hash(b) {
                files.push((i as u32, j as u32));
            }
        }
    }
    let shard_files: Vec<usize> = shards.iter().map(|s| s.get().files.len()).collect();
    let bytes = std::fs::read(crate::cache_dir(out).join("composed")).ok()?;
    let composed = cache::Composed::decode(&bytes, cache::key(&old), &shard_files)?;
    Some(Taken {
        composed,
        old,
        changed: files,
    })
}
