//! Derived facts of the end-to-end selection from source, kept between queries
//! in the graph directory (`<cache dir>/e2e-*`): what a spec visits, and
//! the invocation relation of an app. Each is a function of file contents and
//! nothing else, so it is stored under a key made of those contents; a query
//! that finds the key reads the fact instead of computing it. The store changes
//! how long a query takes, never its answer: a fact is used only when
//! the key computed from the current tree is the one it was stored under.
//!
//! The keys ([`Key`]) hold the format version, the binary
//! ([`turborepo_trace_store::build`]), and per fact:
//!
//! | Fact | Key |
//! |---|---|
//! | the visits of a spec (`crate::e2e_static`: the routes its URLs match) | the spec, its Playwright config directory and hosts; the content of every file its code can load ([`Closures`]); every loaded app (its directory and everything `NextApp::load` read: routes, config, middleware, microfrontends) |
//! | the relation of an app | the app; the content of every file its routes and middleware can load; the URL-API tables of `turbotrace.toml` (where it is read) |
//! | what following adds to a spec in one app under one rule | the two keys above and the rule |
//!
//! `TURBO_TRACE_DEBUG_E2E_CACHE=0` (and `TURBO_TRACE_DEBUG_DERIVED_CACHE=0`,
//! the switch of the graph's derived layers) neither reads nor writes:
//! everything is computed, by the same code.

use std::path::PathBuf;

use serde::{Serialize, de::DeserializeOwned};
use turborepo_trace_graph::{GraphView, NodeId, RefFlags, RefKind};
use turborepo_trace_store::Graph;

/// Format of the stored facts and of what their keys cover.
pub(crate) const VERSION: u32 = 1;

/// Whether facts are read and written.
pub(crate) fn enabled() -> bool {
    let on = |name: &str| turborepo_trace_core::debug::var_os(name).is_none_or(|v| v != "0");
    on("E2E_CACHE") && on("DERIVED_CACHE")
}

/// A key under construction.
pub(crate) struct Key(xxhash_rust::xxh3::Xxh3);

impl Key {
    /// A key of the kind `tag`, for this binary and format.
    pub(crate) fn new(tag: &str) -> Key {
        let mut k = Key(xxhash_rust::xxh3::Xxh3::new());
        k.0.update(&VERSION.to_le_bytes());
        k.str(turborepo_trace_store::build::build_id());
        k.str(tag);
        k
    }

    pub(crate) fn str(&mut self, s: &str) -> &mut Key {
        self.bytes(s.as_bytes())
    }

    pub(crate) fn bytes(&mut self, b: &[u8]) -> &mut Key {
        self.0.update(&(b.len() as u64).to_le_bytes());
        self.0.update(b);
        self
    }

    pub(crate) fn num(&mut self, n: u128) -> &mut Key {
        self.0.update(&n.to_le_bytes());
        self
    }

    pub(crate) fn finish(&self) -> u128 {
        self.0.digest128()
    }
}

/// A key as it is stored.
pub(crate) fn hex(k: u128) -> String {
    format!("{k:032x}")
}

/// The digest of a serializable value (its JSON).
pub(crate) fn digest_of<T: Serialize>(v: &T) -> u128 {
    struct Sink(xxhash_rust::xxh3::Xxh3);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.update(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut s = Sink(xxhash_rust::xxh3::Xxh3::new());
    // A value that cannot be written has no stable digest: a different one every
    // time.
    if serde_json::to_writer(&mut s, v).is_err() {
        s.0.update(
            &std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
                .to_le_bytes(),
        );
    }
    s.0.digest128()
}

/// The content of the files that code can load, as digests: the files reached
/// from seed files over module edges (imports, `require`, `import()`,
/// re-exports, type-only ones included, so that the set is no smaller than what
/// any reader of the code follows).
pub(crate) struct Closures<'g> {
    g: &'g Graph,
    /// Per node: the walk that saw it last.
    seen: Vec<u32>,
    walk: u32,
}

impl<'g> Closures<'g> {
    pub(crate) fn new(g: &'g Graph) -> Self {
        Closures {
            g,
            seen: vec![0; g.node_count() as usize],
            walk: 0,
        }
    }

    /// The digest of the first-party files `seeds` can load, themselves
    /// included: of each its path, its content and the files its module
    /// edges lead to (what an import resolves to can change without the
    /// importer changing). It does not depend on the order of the walk. A
    /// file without a recorded content hash (it is not on disk)
    /// counts by its path.
    pub(crate) fn digest(&mut self, seeds: &[NodeId]) -> u128 {
        self.walk += 1;
        let walk = self.walk;
        let mut stack: Vec<NodeId> = Vec::new();
        for &s in seeds {
            if std::mem::replace(&mut self.seen[s.0 as usize], walk) != walk {
                stack.push(s);
            }
        }
        let (mut sum, mut files) = (0u128, 0u64);
        while let Some(n) = stack.pop() {
            if self.g.is_external(n) {
                continue;
            }
            let mut h = xxhash_rust::xxh3::Xxh3::new();
            h.update(self.g.path(n).as_bytes());
            match self.g.file_hash(n) {
                Some(c) => h.update(&c.to_le_bytes()),
                None => h.update(b"\0no content hash"),
            }
            files += 1;
            for e in self.g.out_edges(n) {
                if let Some(to) = e.to
                    && matches!(
                        e.kind(),
                        RefKind::Import
                            | RefKind::Require
                            | RefKind::DynamicImport
                            | RefKind::ReExport
                    )
                {
                    h.update(&[
                        0,
                        e.kind() as u8,
                        u8::from(e.flags().contains(RefFlags::TYPE_ONLY)),
                    ]);
                    h.update(self.g.path(to).as_bytes());
                    if std::mem::replace(&mut self.seen[to.0 as usize], walk) != walk {
                        stack.push(to);
                    }
                }
            }
            sum = sum.wrapping_add(h.digest128());
        }
        let mut k = xxhash_rust::xxh3::Xxh3::new();
        k.update(&sum.to_le_bytes());
        k.update(&files.to_le_bytes());
        k.digest128()
    }
}

fn path_of(g: &Graph, name: &str) -> Option<PathBuf> {
    if !enabled() {
        return None;
    }
    g.derived_cache_path(name)
}

/// The stored value `name` of this graph, if there is one that can be read.
pub(crate) fn read<T: DeserializeOwned>(g: &Graph, name: &str) -> Option<T> {
    let bytes = std::fs::read(path_of(g, name)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Stores `v` as `name` (written beside and renamed, so that a reader never
/// sees a part).
pub(crate) fn write<T: Serialize>(g: &Graph, name: &str, v: &T) {
    let Some(p) = path_of(g, name) else { return };
    let Ok(bytes) = serde_json::to_vec(v) else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &bytes).is_err() || std::fs::rename(&tmp, &p).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}
