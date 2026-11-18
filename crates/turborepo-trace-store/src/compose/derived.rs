//! A cache of derived layers of a graph, keyed by its manifest
//! (`<cache dir>/derived-<name>`): [`Graph::cached_patch`],
//! [`Graph::carried_patch`].
//!
//! The derived layers (per-test working directories, hints, the environment
//! registry) are recomputed whenever a graph is opened. Each is a pure function
//! of the composed graph and a few inputs of its own, so on an unchanged
//! repository the result of the last run can be read back, like the composed
//! graph itself ([`super::cache`]).
//!
//! # Across an edit
//!
//! Every edit gives the manifest another key, and with it every layer. But when
//! the graph took its composed parts from the snapshot before it
//! ([`super::reuse`]), the two graphs differ in nothing composition reads: the
//! same nodes, edges, references, uncertainty records, tables and walked files.
//! What does differ is a function of the contents of the files that changed:
//! their hashes, their extractions (the facts of a file), and the
//! lines and spans of their references, edges and uncertainty records. The
//! graph knows those files ([`Graph::carried`]: the reuse compared their
//! shards).
//!
//! So a layer that is a function of the composed parts and of the contents (in
//! that wide sense) of a set of files it can name is the same layer in both
//! graphs when none of those files changed. [`Graph::carried_patch`] offers the
//! stored layer of the snapshot before to its caller together with the changed
//! files; the caller, who knows what the layer read, says whether it carries
//! over. It is then stored again under this graph's key, bytes unchanged.

use std::{collections::HashSet, path::Path};

use turborepo_trace_graph::{NodeId, Patch};

use super::Graph;

const DERIVED_MAGIC: &[u8; 8] = b"DTDERIV1";

/// The snapshot a graph took its composed parts from across an edit
/// ([`super::reuse`]).
pub(super) struct Carried {
    /// [`super::cache::key`] of that snapshot's manifest.
    pub key: u128,
    /// The files whose contents differ between the two, as (shard, file)
    /// indexes.
    pub changed: Vec<(u32, u32)>,
}

/// How [`Graph::carried_patch`] got a layer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// Stored for this graph.
    Cached,
    /// Stored for the snapshot before the edit, and the same for this graph.
    Carried,
    /// Computed now.
    Computed,
}

/// The key of a stored layer: the graph's identity and the layer's own inputs.
fn full_key(manifest: u128, key: u128) -> [u8; 16] {
    let mut h = xxhash_rust::xxh3::Xxh3::new();
    h.update(DERIVED_MAGIC);
    h.update(&manifest.to_le_bytes());
    h.update(&key.to_le_bytes());
    h.digest128().to_le_bytes()
}

/// A stored layer: its key, its numbers, and the bytes of its patch.
fn parse(bytes: &[u8]) -> Option<([u8; 16], Vec<u64>, &[u8])> {
    let rest = bytes.strip_prefix(DERIVED_MAGIC)?;
    let (k, rest) = rest.split_first_chunk::<16>()?;
    let (n, mut rest) = rest.split_first_chunk::<4>()?;
    let n = u32::from_le_bytes(*n) as usize;
    let mut meta = Vec::with_capacity(n.min(1 << 16));
    for _ in 0..n {
        let (v, r) = rest.split_first_chunk::<8>()?;
        meta.push(u64::from_le_bytes(*v));
        rest = r;
    }
    Some((*k, meta, rest))
}

/// Stores a layer (best effort: a cache that cannot be written is computed
/// again).
fn store(path: &Path, full: &[u8; 16], meta: &[u64], patch: &[u8]) {
    let mut out = Vec::with_capacity(8 + 16 + 4 + meta.len() * 8 + patch.len());
    out.extend_from_slice(DERIVED_MAGIC);
    out.extend_from_slice(full);
    out.extend_from_slice(&(meta.len() as u32).to_le_bytes());
    for v in meta {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(patch);
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&tmp, &out).is_err() || std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

impl Graph {
    /// Where a caller keeps a fact it derives from this graph and keys by
    /// itself (`<cache dir>/<name>`), next to the derived layers.
    /// `None` for a graph without a store directory and with
    /// `TURBO_TRACE_DEBUG_DERIVED_CACHE=0`.
    pub fn derived_cache_path(&self, name: &str) -> Option<std::path::PathBuf> {
        let on = turborepo_trace_core::debug::var_os("DERIVED_CACHE").is_none_or(|v| v != "0");
        self.tables_cache
            .as_deref()
            .and_then(|p| p.parent())
            .filter(|_| on)
            .map(|dir| dir.join(name))
    }

    /// A derived layer of this graph ([`turborepo_trace_graph::Patch`]) from
    /// the store's cache (`<cache dir>/derived-<name>`), or computed
    /// with `compute` and stored there.
    ///
    /// A derived layer is a pure function of the composed graph and a few
    /// inputs of its own (the hints file, a switch): `key` identifies
    /// those; the graph's identity (the binary, the manifest's shards and
    /// walked-files list, as for `cache/composed`) is added here. An
    /// unchanged repository then pays a file read instead of the
    /// computation on every query. `compute` also returns numbers to keep with
    /// the patch (statistics). The third value says whether the cache
    /// answered.
    ///
    /// `TURBO_TRACE_DEBUG_DERIVED_CACHE=0` always computes.
    pub fn cached_patch(
        &self,
        name: &str,
        key: u128,
        compute: impl FnOnce() -> (Patch, Vec<u64>),
    ) -> (Patch, Vec<u64>, bool) {
        let (patch, meta, kept) = self.carried_patch(name, key, |_, _| false, compute);
        (patch, meta, kept == Kept::Cached)
    }

    /// The files whose contents changed since the snapshot whose composed parts
    /// this graph took over across an edit (see the module documentation).
    /// `None`: the graph was composed from its shards, or read the composed
    /// parts of its own manifest.
    pub fn carried(&self) -> Option<Vec<NodeId>> {
        let c = self.carried.as_ref()?;
        let changed: HashSet<(u32, u32)> = c.changed.iter().copied().collect();
        Some(
            self.nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| changed.contains(&(n.shard, n.file)))
                .map(|(i, _)| NodeId(i as u32))
                .collect(),
        )
    }

    /// [`Graph::cached_patch`], which can also take the layer over from the
    /// snapshot before an edit (see the module documentation).
    ///
    /// When the cache holds the layer `name` of the snapshot this graph took
    /// its composed parts from, stored with the same `key`, `same` is given
    /// that layer's numbers and the files whose contents changed since
    /// ([`Graph::carried`]). It must return true only if the layer read the
    /// contents of none of them: nothing of their text, their hashes, their
    /// extractions, or the lines and spans of their references, edges and
    /// uncertainty records. Everything else the layer can have read from the
    /// graph is the same in both. The layer is then this graph's: it is
    /// returned and stored under this graph's key.
    ///
    /// `TURBO_TRACE_DEBUG_DERIVED_CACHE=0` always computes.
    pub fn carried_patch(
        &self,
        name: &str,
        key: u128,
        same: impl FnOnce(&[u64], &[NodeId]) -> bool,
        compute: impl FnOnce() -> (Patch, Vec<u64>),
    ) -> (Patch, Vec<u64>, Kept) {
        let on = turborepo_trace_core::debug::var_os("DERIVED_CACHE").is_none_or(|v| v != "0");
        let path = self
            .tables_cache
            .as_deref()
            .and_then(|p| p.parent())
            .filter(|_| on)
            .map(|dir| dir.join(format!("derived-{name}")));
        let full = full_key(super::cache::key(&self.manifest), key);
        if let Some(p) = &path
            && let Ok(bytes) = std::fs::read(p)
            && let Some((k, meta, body)) = parse(&bytes)
        {
            if k == full {
                if let Some(patch) = Patch::from_bytes(body) {
                    return (patch, meta, Kept::Cached);
                }
            } else if let Some(c) = &self.carried
                && k == full_key(c.key, key)
                && let Some(changed) = self.carried()
                && same(&meta, &changed)
                && let Some(patch) = Patch::from_bytes(body)
            {
                store(p, &full, &meta, body);
                return (patch, meta, Kept::Carried);
            }
        }
        let (patch, meta) = compute();
        if let Some(p) = &path {
            store(p, &full, &meta, &patch.to_bytes());
        }
        (patch, meta, Kept::Computed)
    }
}
