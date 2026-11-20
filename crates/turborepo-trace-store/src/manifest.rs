//! `<graph dir>/graph.json`: the snapshot manifest.
//!
//! ```json
//! {
//!   "schema": "turbotrace-graph-manifest",
//!   "version": 1,
//!   "turbotrace": "0.1.0",
//!   "format": 1,
//!   "root": "/srv/repo",
//!   "options": {"externals": "opaque", "conditions": [], "all_conditions": false,
//!               "include_types": true, "max_glob": 1000, "target": null},
//!   "options_digest": "…",
//!   "lockfiles": [{"path": "pnpm-lock.yaml", "hash": "…"}],
//!   "shards": [{"key": "…", "kind": "first-party", "name": "web", "root": "apps/web"},
//!              {"key": "…", "kind": "opaque", "name": "react", "version": "18.3.1",
//!               "root": "node_modules/.pnpm/react@18.3.1/node_modules/react"}]
//! }
//! ```
//!
//! `kind` is `first-party`, `third-party` (traced with `--externals follow`) or
//! `opaque`. Shards are sorted by kind, root, name and version. Hashes are
//! xxh3-128 in hex.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, GraphOptions};

/// Manifest schema name.
pub const SCHEMA: &str = "turbotrace-graph-manifest";
/// Manifest schema version.
pub const VERSION: u32 = 1;

/// The manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Always [`SCHEMA`].
    pub schema: String,
    /// [`VERSION`].
    pub version: u32,
    /// The version of the crate that wrote it.
    pub turbotrace: String,
    /// Shard format version.
    pub format: u32,
    /// Absolute repository root.
    pub root: String,
    /// Build options.
    pub options: ManifestOptions,
    /// Digest of everything in the options that affects shards.
    pub options_digest: String,
    /// Lockfiles of every ecosystem (at the root and at package roots) with
    /// their content hashes.
    pub lockfiles: Vec<LockHash>,
    /// The shards of this snapshot.
    pub shards: Vec<ShardEntry>,
    /// Store file (`files-<xxh3-128>`) with the walked files of this snapshot,
    /// from which the composed graph expands membership hubs
    /// ([`crate::shard::HubRec`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<String>,
}

const FILES_MAGIC: &[u8; 8] = b"DTFILES1";

/// Whether `s` is a store key as builds write them: an xxh3-128 digest in 32
/// lowercase hex digits. Keys name files in the store directory; a graph
/// directory is checkout-controlled (and travels through caches), so a key
/// read from it is only joined to the store when it has this form.
pub fn is_shard_key(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `s` is the name of a walked-file list as [`write_files`] writes
/// it: `files-` and an xxh3-128 digest in 32 lowercase hex digits.
pub fn is_files_name(s: &str) -> bool {
    s.strip_prefix("files-").is_some_and(is_shard_key)
}

/// Writes the walked files (repo-relative, sorted) to `<store>/files-<digest>`
/// unless that file exists; returns its name.
pub(crate) fn write_files(store: &Path, files: &[String]) -> Option<String> {
    let mut b = Vec::with_capacity(16 + files.iter().map(|f| f.len() + 1).sum::<usize>());
    b.extend_from_slice(FILES_MAGIC);
    b.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for f in files {
        b.extend_from_slice(f.as_bytes());
        b.push(0);
    }
    let name = format!("files-{:032x}", xxhash_rust::xxh3::xxh3_128(&b));
    let path = store.join(&name);
    if !path.is_file() {
        crate::write_atomic(&path, &b).ok()?;
    }
    Some(name)
}

/// Reads a walked-file list written by [`write_files`].
pub(crate) fn read_files(store: &Path, name: &str) -> Option<Vec<String>> {
    if !is_files_name(name) {
        return None;
    }
    let b = std::fs::read(store.join(name)).ok()?;
    if b.len() < 16 || &b[..8] != FILES_MAGIC {
        return None;
    }
    if format!("files-{:032x}", xxhash_rust::xxh3::xxh3_128(&b)) != name {
        return None;
    }
    let n = u64::from_le_bytes(b[8..16].try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(n);
    for part in b[16..].split(|&c| c == 0) {
        if out.len() == n {
            break;
        }
        out.push(std::str::from_utf8(part).ok()?.to_owned());
    }
    (out.len() == n).then_some(out)
}

/// The bytes of a walked-file list written by [`write_files`], checked like
/// [`read_files`] does but not split into strings: iterate the paths with
/// [`files_in`].
pub(crate) fn read_files_raw(store: &Path, name: &str) -> Option<Vec<u8>> {
    if !is_files_name(name) {
        return None;
    }
    let b = std::fs::read(store.join(name)).ok()?;
    if b.len() < 16 || &b[..8] != FILES_MAGIC {
        return None;
    }
    if format!("files-{:032x}", xxhash_rust::xxh3::xxh3_128(&b)) != name {
        return None;
    }
    std::str::from_utf8(&b[16..]).ok()?;
    Some(b)
}

/// The paths of a list returned by [`read_files_raw`], in stored (sorted)
/// order.
pub(crate) fn files_in(raw: &[u8]) -> impl Iterator<Item = &str> + '_ {
    let n = u64::from_le_bytes(raw[8..16].try_into().unwrap_or([0; 8])) as usize;
    raw[16..]
        .split(|&c| c == 0)
        .take(n)
        .filter_map(|p| std::str::from_utf8(p).ok())
}

/// Recorded options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestOptions {
    /// `opaque` or `follow`.
    pub externals: String,
    /// Resolution conditions.
    pub conditions: Vec<String>,
    /// `--all-conditions`.
    pub all_conditions: bool,
    /// Type-only references included.
    pub include_types: bool,
    /// Glob cap.
    pub max_glob: usize,
    /// Build target.
    pub target: Option<String>,
    /// `--all-targets`.
    #[serde(default)]
    pub all_targets: bool,
}

/// A lockfile hash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockHash {
    /// Path relative to the root (`pnpm-lock.yaml`, `services/svc-a/go.sum`).
    pub path: String,
    /// xxh3-128 hex.
    pub hash: String,
}

/// One shard of the snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardEntry {
    /// Store key (file name in `store/`).
    pub key: String,
    /// `first-party`, `third-party` or `opaque`.
    pub kind: String,
    /// Package name.
    pub name: String,
    /// Version (third-party only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Package root (repo-relative; real path for third-party packages).
    pub root: String,
}

impl Manifest {
    /// A manifest for a finished build.
    pub fn new(
        root: &Path,
        opts: &GraphOptions,
        options_digest: String,
        lockfiles: Vec<(String, String)>,
        shards: Vec<ShardEntry>,
    ) -> Self {
        Manifest {
            schema: SCHEMA.to_owned(),
            version: VERSION,
            turbotrace: env!("CARGO_PKG_VERSION").to_owned(),
            format: crate::shard::FORMAT_VERSION,
            root: root.to_string_lossy().into_owned(),
            options: ManifestOptions {
                externals: opts.externals.as_str().to_owned(),
                conditions: opts.conditions.clone(),
                all_conditions: opts.all_conditions,
                include_types: opts.include_types,
                max_glob: opts.max_glob,
                target: opts.target.clone(),
                all_targets: opts.all_targets,
            },
            options_digest,
            lockfiles: lockfiles
                .into_iter()
                .map(|(path, hash)| LockHash { path, hash })
                .collect(),
            shards,
            files: None,
        }
    }

    /// Reads `<out>/graph.json`. `None` if missing, unreadable, of another
    /// version, or naming a store file in another form than builds write
    /// ([`Manifest::names_are_canonical`]).
    pub fn read(out: &Path) -> Option<Manifest> {
        let m: Manifest =
            serde_json::from_slice(&std::fs::read(out.join("graph.json")).ok()?).ok()?;
        (m.schema == SCHEMA
            && m.version == VERSION
            && m.format == crate::shard::FORMAT_VERSION
            && m.names_are_canonical())
        .then_some(m)
    }

    /// Whether every store file the manifest names has the form builds write:
    /// shard keys [`is_shard_key`], the walked-file list [`is_files_name`].
    /// The names are joined to the store directory to read, rewrite and
    /// delete files there, and `graph.json` is checkout-controlled: a name
    /// with a separator, `..` or a root would reach outside the store.
    pub fn names_are_canonical(&self) -> bool {
        self.shards.iter().all(|s| is_shard_key(&s.key))
            && self.files.as_deref().is_none_or(is_files_name)
    }

    /// Writes `<out>/graph.json` atomically.
    // Invariant: our own structs (string keys, derived impls) serialize to a Vec.
    #[allow(clippy::expect_used)]
    pub fn write(&self, out: &Path) -> Result<bool, Error> {
        let path = out.join("graph.json");
        let mut bytes = serde_json::to_vec_pretty(self).expect("manifest serializes");
        bytes.push(b'\n');
        crate::write_if_changed(&path, &bytes).map_err(|e| Error::io(&path, e))
    }
}
