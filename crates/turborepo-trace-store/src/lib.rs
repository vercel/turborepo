//! The whole-repo graph store of `turbo trace` .
//!
//! - [`build`]: walks the repository, partitions it into per-package shards
//!   (workspace packages, the root package, third-party packages), traces only
//!   the shards whose key changed, and writes them to a content-addressed store
//!   with a `graph.json` manifest.
//! - [`Graph::open`]: composes the shards of a manifest into one graph that
//!   implements [`turborepo_trace_graph::GraphView`], so the query engine and
//!   every exporter run on it.
//! - [`gc`]: deletes store files no manifest references.
//! - [`evict`]: `gc` first, then turbo's `cacheMaxAge` and `cacheMaxSize`
//!   applied to the rest of the graph directory.
//!
//! On-disk layout (`.turbo/trace/` by default):
//!
//! ```text
//! graph.json          the manifest: root, options, lockfile hashes, shard keys
//! store/<key>         one shard per package (versioned rkyv archive, zero-copy)
//! cache/hashes        content hashes keyed by (dev, inode, size, mtime_ns)
//! ```
//!
//! `cache/` is [`cache_dir`]: below the graph directory by default, and in a
//! per-machine directory when the host sets one ([`set_machine_cache`]).
//!
//! [`artifact`] packs the manifest and the shards into one file and installs it
//! elsewhere: a fresh checkout restores the graph of a nearby commit and
//! refreshes it.
//!
//! This crate lives outside `turborepo-trace-core` so that core stays a small
//! tracing library: the store pulls in `rkyv`, `ignore`, the lockfile parsers
//! and the query engine, none of which `trace` needs.
//!
//! Formats: [`shard`] (shard files), [`manifest`] (`graph.json`). Keys and
//! incremental rebuilds: [`mod@build`]. Probe compaction: [`normalize`].

// Every public item is documented.
#![warn(missing_docs)]

pub mod artifact;
pub mod build;
mod compose;
pub mod convert;
mod external;
mod filereuse;
mod follow;
pub mod fw;
mod glob;
pub mod hash;
pub mod identity;
mod keydump;
mod located;
pub mod manifest;
pub mod normalize;
mod once;
mod own;
pub mod paths;
pub mod shard;
pub mod smallfiles;
pub mod tasks;
mod treeindex;
pub mod walk;
pub mod workspace;

use std::path::{Path, PathBuf};

pub use build::{BuildReport, Externals, build};
pub use compose::{
    Graph, Kept, LoadStats,
    bind::{BoundDep, BoundStats, BoundTarget, ClosedSite, ClosedTable},
    cwd::CwdDropped,
    cwd_edge_site,
    env::{EnvAnswer, EnvDecl, EnvOracle, EnvStats},
    hints::{Hint, Hints, Loads, OTHER_TABLES},
};
pub use manifest::Manifest;

/// The default name of a graph directory. A directory of this name is never
/// part of the analyzed tree, whichever build it belongs to: the walk does not
/// enter it and the tracer's file system hides it (`DirCache::hide_named`), so
/// two builds of one tree agree wherever each keeps its graph.
pub const GRAPH_DIR: &str = ".turbotrace";

/// Where a graph directory is by default, relative to the repository root:
/// beside turbo's other state in `.turbo`. The walk and the tracer leave the
/// graph directory out of the tree wherever it is.
pub const DEFAULT_GRAPH_PATH: &str = ".turbo/trace";

/// Where the caches of every graph directory live: `None` (the default),
/// `<graph dir>/cache`; `Some(base)`, a directory per graph directory below
/// `base` ([`set_machine_cache`]).
static MACHINE_CACHE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Keeps the caches of every graph directory of this process below `base`, one
/// directory per graph directory (named by a digest of its real path), instead
/// of in `<graph dir>/cache`. `None` restores the default.
///
/// Some caches are keyed by this machine's file stamps (`hashes`, `manifests`,
/// `walk`, `obs`, and the names layer's `names-data`: device, inode, size and
/// mtime): on another machine or in another checkout a stamp can name other
/// bytes. A graph directory inside the repository travels with it (CI caches
/// `.turbo` with `actions/cache`), so a host that keeps its graph there keeps
/// its caches in a per-machine directory: the graph directory then holds only
/// the manifest, the store and the recorded language arguments, which are a
/// function of the tree. Every cache moves, not only the stamp-keyed ones:
/// what stays in the graph directory is what may travel, and that list stays
/// short enough to check ([`artifact`], "What travels").
pub fn set_machine_cache(base: Option<PathBuf>) {
    *MACHINE_CACHE.write().unwrap_or_else(|e| e.into_inner()) = base;
}

/// The cache directory of the graph directory `out`: `<out>/cache`, or the
/// per-machine one ([`set_machine_cache`]).
pub fn cache_dir(out: &Path) -> PathBuf {
    let base = MACHINE_CACHE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    match base {
        None => out.join("cache"),
        Some(base) => {
            let real = real_path(out);
            let digest = xxhash_rust::xxh3::xxh3_128(real.as_os_str().as_encoded_bytes());
            base.join(format!("{digest:032x}"))
        }
    }
}

/// Creates [`cache_dir`] of `out` (best effort) and returns it. A per-machine
/// cache directory gets a `graph-dir` file naming the graph directory it
/// belongs to, for whoever wonders what it is.
pub(crate) fn create_cache_dir(out: &Path) -> PathBuf {
    let dir = cache_dir(out);
    let _ = std::fs::create_dir_all(&dir);
    let named = dir.join("graph-dir");
    if !dir.starts_with(out) && !named.exists() {
        let _ = std::fs::write(named, format!("{}\n", real_path(out).display()));
    }
    dir
}

/// `p` with its longest existing ancestor resolved (links, `..`), so that a
/// graph directory has one name before and after it is created.
fn real_path(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(real, |acc, c| acc.join(c));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                at = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Options for [`build`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphOptions {
    /// Repository root.
    pub root: PathBuf,
    /// Store directory; default `<root>/.turbo/trace` ([`DEFAULT_GRAPH_PATH`]).
    pub out: Option<PathBuf>,
    /// Third-party policy.
    pub externals: Externals,
    /// Resolution conditions (empty: the language defaults).
    pub conditions: Vec<String>,
    /// Also resolve every other `exports` condition.
    pub all_conditions: bool,
    /// Build target (`linux/x64`, ...).
    pub target: Option<String>,
    /// Include files for every target as conditional edges (Go, Rust).
    pub all_targets: bool,
    /// Glob expansion cap.
    pub max_glob: usize,
    /// Worker threads (0: one per core).
    pub threads: usize,
    /// Include type-only references (default true: typecheck and build steps
    /// read them).
    pub include_types: bool,
    /// Which directories are the JavaScript workspace members (default: what
    /// the repository declares, [`workspace::DeclaredMembers`]).
    pub members: workspace::Members,
}

impl Default for GraphOptions {
    fn default() -> Self {
        GraphOptions {
            root: PathBuf::from("."),
            out: None,
            externals: Externals::Opaque,
            conditions: Vec::new(),
            all_conditions: false,
            target: None,
            all_targets: false,
            max_glob: turborepo_trace_core::DEFAULT_MAX_GLOB,
            threads: 0,
            include_types: true,
            members: workspace::Members::default(),
        }
    }
}

impl GraphOptions {
    /// The store directory for an absolute `root`.
    pub fn out_dir(&self, root: &Path) -> PathBuf {
        match &self.out {
            Some(o) if o.is_absolute() => o.clone(),
            Some(o) => std::env::current_dir()
                .map(|c| c.join(o))
                .unwrap_or_else(|_| o.clone()),
            None => root.join(DEFAULT_GRAPH_PATH),
        }
    }
}

/// Errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// I/O on a path.
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    /// The tracer could not be built.
    #[error(transparent)]
    Trace(turborepo_trace_core::TraceError),
    /// A shard could not be read.
    #[error(transparent)]
    Shard(shard::ShardError),
    /// No graph has been built in this directory.
    #[error("no graph in {0}")]
    NotBuilt(PathBuf),
    /// A graph directory, or the store or caches below it, is a symbolic link.
    ///
    /// A graph directory is checkout-controlled (a commit can add
    /// `.turbo/trace`), so writing into it and cleaning it up refuse to follow
    /// one: the link would reach files outside the repository.
    #[error("{0}: not a directory (a symbolic link)")]
    Linked(PathBuf),
}

impl Error {
    pub(crate) fn io(path: &Path, e: std::io::Error) -> Error {
        Error::Io(path.to_path_buf(), e)
    }
}

/// Refuses a symbolic link at `path`, and at every component of `path` below
/// `base` when `path` lies there: that is the part of the path a checkout
/// controls, and following a link in it would write, or delete, outside the
/// repository. Without such a base (a graph directory or cache outside the
/// checkout) `path` itself is all that can be checked.
///
/// A `path` that does not exist is not a link: the caller creates it.
pub(crate) fn real_dir(path: &Path, base: Option<&Path>) -> Result<(), Error> {
    let stop = base.filter(|b| path.starts_with(b));
    if stop.is_none() {
        return not_link(path);
    }
    for at in path.ancestors() {
        not_link(at)?;
        if stop == Some(at) {
            break;
        }
    }
    Ok(())
}

/// `path` is not a symbolic link (it need not exist).
fn not_link(path: &Path) -> Result<(), Error> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::Linked(path.to_path_buf())),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Cache writes left running in the background, counted per graph directory
/// (see [`wait_for_cache_writes_in`]). A count, not a list of handles that a
/// waiter takes: the build of one graph must wait for that graph's writes
/// whoever else waits meanwhile (builds of other graphs in the same process, as
/// in parallel tests, used to take each other's handles, and a refresh then
/// read the observation cache before it was written).
static WRITES: std::sync::Mutex<std::collections::BTreeMap<PathBuf, usize>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());
/// Signalled whenever a write counted in [`WRITES`] ends.
static WRITE_DONE: std::sync::Condvar = std::sync::Condvar::new();

/// The key of a graph directory in [`WRITES`]: its real path, so that two
/// spellings of one directory wait for each other.
fn writes_key(out: &Path) -> PathBuf {
    std::fs::canonicalize(out).unwrap_or_else(|_| out.to_path_buf())
}

/// One background cache write below a graph directory: counted from its
/// creation until it is dropped (when the write is done, when it panicked, or
/// when its thread never started).
struct Writing(PathBuf);

impl Writing {
    fn begin(out: &Path) -> Writing {
        let key = writes_key(out);
        *WRITES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_default() += 1;
        Writing(key)
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        let mut w = WRITES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = w.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                w.remove(&self.0);
            }
        }
        drop(w);
        WRITE_DONE.notify_all();
    }
}

/// Runs `f` (a cache write below the graph directory `out`) on a background
/// thread, then `then` (freeing state) on the same thread after the write is
/// accounted done. [`wait_for_cache_writes_in`] (so the next [`build`] of that
/// graph) and [`wait_for_cache_writes`] wait for `f` only.
pub(crate) fn spawn_cache_write(
    out: &Path,
    f: impl FnOnce() + Send + 'static,
    then: impl FnOnce() + Send + 'static,
) {
    let writing = Writing::begin(out);
    // A thread that cannot be spawned drops the closure, and `writing` with it.
    let _ = std::thread::Builder::new()
        .name("turbo-trace-cache-write".into())
        .spawn(move || {
            f();
            drop(writing);
            then();
        });
}

fn wait_while(pending: impl Fn(&std::collections::BTreeMap<PathBuf, usize>) -> bool) {
    let mut w = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    while pending(&w) {
        w = WRITE_DONE.wait(w).unwrap_or_else(|e| e.into_inner());
    }
}

/// Waits until the cache writes that builds and graph loads of this process
/// left running below the graph directory `out` are on disk. [`build`] does
/// this before it reads them.
pub fn wait_for_cache_writes_in(out: &Path) {
    let key = writes_key(out);
    wait_while(|w| w.contains_key(&key));
}

/// Waits until no cache write of this process is running, for any graph
/// directory. Call before the process exits after a [`build`] (a write cut
/// short leaves the previous cache, plus a temporary file).
pub fn wait_for_cache_writes() {
    wait_while(|w| !w.is_empty());
}

/// Writes `bytes` to `path` (as [`write_atomic`] does) unless the file already
/// holds exactly these bytes; returns whether it wrote. A refresh that changes
/// nothing leaves the graph directory as it is: no file is rewritten and no
/// directory's mtime moves.
pub fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<bool> {
    let same = std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64)
        && std::fs::read(path).is_ok_and(|old| old == bytes);
    if same {
        return Ok(false);
    }
    write_atomic(path, bytes)?;
    Ok(true)
}

/// Writes `bytes` to `path` through a temporary file and a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "tmp.{}.{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// What [`gc`] removed.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Shard files kept (referenced by the manifest).
    pub kept: usize,
    /// Shard files deleted.
    pub removed: usize,
    /// Bytes freed.
    pub bytes: u64,
}

/// Deletes from `<out>/store` every shard the current manifest does not
/// reference: rewrites the pack with exactly the referenced shards (dropping
/// its log) and removes every other file but the snapshot's walked-file list.
///
/// Only files a build writes there are deleted (see [`is_store_file`]), and
/// only when they are regular files: a graph directory is checkout-controlled,
/// so a symbolic link at it, above it, or at the store is refused
/// ([`Error::Linked`]) and anything else in the store is left alone.
pub fn gc(out: &Path) -> Result<GcReport, Error> {
    let manifest = Manifest::read(out).ok_or_else(|| Error::NotBuilt(out.to_path_buf()))?;
    let store = out.join("store");
    // Before anything is read through them or deleted below them: a commit can
    // add `.turbo/trace`, or the store, as a link to anywhere.
    let base = PathBuf::from(&manifest.root);
    real_dir(out, Some(&base))?;
    real_dir(&store, Some(&base))?;
    let pack = shard::Pack::open(&store);
    let mut r = GcReport::default();
    let shards: Vec<(&str, shard::LoadedShard)> = manifest
        .shards
        .iter()
        .filter_map(|e| {
            shard::load_from(&store, pack.as_ref(), &e.key)
                .ok()
                .map(|s| (e.key.as_str(), s))
        })
        .collect();
    let before = pack.as_ref().map_or(0, |p| p.sizes()[0] + p.sizes()[1]);
    let dropped = pack.as_ref().map_or(0, |p| p.len()).saturating_sub(
        shards
            .iter()
            .filter(|(k, _)| pack.as_ref().is_some_and(|p| p.contains(k)))
            .count(),
    );
    drop(pack);
    let list: Vec<(&str, &shard::LoadedShard)> = shards.iter().map(|(k, s)| (*k, s)).collect();
    shard::Pack::compact(&store, &list).map_err(|e| Error::io(&store, e))?;
    let after = std::fs::metadata(store.join(shard::PACK_FILE)).map_or(0, |m| m.len());
    r.bytes += before.saturating_sub(after);
    r.removed += dropped;
    r.kept = shards.len();
    let keep: std::collections::HashSet<&str> = [shard::PACK_FILE]
        .into_iter()
        .chain(manifest.files.as_deref())
        .collect();
    for e in std::fs::read_dir(&store).map_err(|e| Error::io(&store, e))? {
        let e = e.map_err(|e| Error::io(&store, e))?;
        let name = e.file_name().to_string_lossy().into_owned();
        // What a build did not write there is not the cleanup's to delete, and a
        // link is not a file of ours whatever it is called.
        let regular = e
            .file_type()
            .map_err(|err| Error::io(&e.path(), err))?
            .is_file();
        if !regular || !is_store_file(&name) {
            continue;
        }
        if keep.contains(name.as_str()) {
            if name != shard::PACK_FILE {
                r.kept += 1;
            }
            continue;
        }
        r.bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        std::fs::remove_file(e.path()).map_err(|err| Error::io(&e.path(), err))?;
        r.removed += 1;
    }
    Ok(r)
}

/// Whether `name` is a file a build writes in the store: the pack, its log, a
/// shard key, a walked-file list, or the temporary file an interrupted
/// `write_atomic` left (`<name>.tmp.<pid>.<thread>`). A store also holds what
/// a checkout put there, which [`gc`] leaves alone.
pub fn is_store_file(name: &str) -> bool {
    let name = name.split(".tmp.").next().unwrap_or(name);
    name == shard::PACK_FILE
        || name == shard::LOG_FILE
        || manifest::is_shard_key(name)
        || manifest::is_files_name(name)
}

/// Bytes the current snapshot occupies in the store: the pack and its log, the
/// walked-file list, and referenced shards still in loose files.
pub fn store_size(out: &Path) -> u64 {
    let Some(m) = Manifest::read(out) else {
        return 0;
    };
    let store = out.join("store");
    let (packed, [main, log]) = shard::Pack::index_only(&store);
    main + log
        + m.shards
            .iter()
            .filter(|s| !packed.contains(&s.key))
            .map(|s| s.key.as_str())
            .chain(m.files.as_deref())
            .filter_map(|k| std::fs::metadata(store.join(k)).ok())
            .map(|m| m.len())
            .sum::<u64>()
}

/// What [`evict`] removed.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EvictReport {
    /// Store files kept: the shards of the current snapshot and its walked-file
    /// list ([`GcReport::kept`]).
    pub kept: usize,
    /// Store files removed by [`gc`]: superseded shard versions, walked-file
    /// lists no snapshot names ([`GcReport::removed`]).
    pub removed: usize,
    /// Cache files of the graph directory removed.
    pub cached: usize,
    /// Bytes freed.
    pub bytes: u64,
}

/// Trims the graph directory `out` the way turbo's `cacheMaxAge` and
/// `cacheMaxSize` trim the task cache in `.turbo/cache`: [`gc`] runs first, and
/// the limits then apply to what it leaves that the next command does not need.
///
/// The shards the current `graph.json` references are never eviction
/// candidates, whatever their age: they are what the next command opens, and
/// [`gc`] (which goes first) drops every store file the manifest does not
/// reference, superseded shard versions and a log that grew among them. Nothing
/// is evicted one shard at a time: the store is a pack and its log
/// ([`shard::Pack`]), not a file per shard, and after [`gc`] holds no shard the
/// manifest leaves out.
///
/// The limits apply to the rest of the graph directory, the caches below
/// [`cache_dir`] (the per-machine directory when the host sets one): a cache
/// file last written more than `max_age` ago is removed, then the least
/// recently used ones are removed while the graph directory is over
/// `max_size`. A snapshot that is over `max_size` on its own keeps the store
/// and loses the caches. Every cache file is state a build recomputes, so
/// eviction costs a slower next build and changes no result.
///
/// Only the cache files a build writes ([`CACHE_FILES`]) are removed, and only
/// when they are regular files: a symbolic link at the graph directory, above
/// it, or at a cache directory is refused ([`Error::Linked`]), and anything
/// else there is left alone.
///
/// Best effort, like the task cache's eviction: a file that cannot be removed
/// is left for the next run.
pub fn evict(
    out: &Path,
    max_age: Option<std::time::Duration>,
    max_size: Option<u64>,
) -> Result<EvictReport, Error> {
    let manifest = Manifest::read(out);
    // Before anything below them is deleted: a commit can add `.turbo/trace`, or
    // its cache directory, as a link to anywhere.
    let base = manifest.as_ref().map(|m| PathBuf::from(&m.root));
    let cache = cache_dir(out);
    real_dir(out, base.as_deref())?;
    real_dir(&cache, base.as_deref())?;
    let mut report = EvictReport::default();
    match &manifest {
        // The store already holds exactly the snapshot: there is nothing to
        // reclaim, and a pack of the whole snapshot is not rewritten for nothing.
        Some(manifest) if store_is_current(out, manifest) => {
            report.kept = manifest.shards.len() + usize::from(manifest.files.is_some());
        }
        _ => match gc(out) {
            Ok(cleaned) => {
                report.kept = cleaned.kept;
                report.removed = cleaned.removed;
                report.bytes = cleaned.bytes;
            }
            // No snapshot: nothing is referenced, and a store a build is writing is
            // not the cleanup's to touch. Only the caches are trimmed.
            Err(Error::NotBuilt(_)) => {}
            Err(e) => return Err(e),
        },
    }
    // What the next command needs, counted and never evicted: the store after
    // `gc`, and the manifest.
    let snapshot = files_below(&out.join("store"))
        .iter()
        .map(|f| f.bytes)
        .sum::<u64>()
        + std::fs::metadata(out.join("graph.json")).map_or(0, |m| m.len());
    let mut cached = 0u64;
    let mut files = Vec::new();
    for file in files_below(&cache) {
        // Only the caches a build writes are the cleanup's to delete: a graph
        // directory is checkout-controlled and may hold anything.
        if !CACHE_FILES.contains(&file.name.as_str()) {
            continue;
        }
        cached += file.bytes;
        files.push(file);
    }
    // Phase 1: age. A cache file's mtime is when its contents last changed, so a
    // file that an unchanged tree left alone is the one that goes first.
    if let Some(max_age) = max_age {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(max_age)
            .unwrap_or(std::time::UNIX_EPOCH);
        for file in &files {
            if file.used >= cutoff || std::fs::remove_file(&file.path).is_err() {
                continue;
            }
            report.cached += 1;
            report.bytes += file.bytes;
            cached = cached.saturating_sub(file.bytes);
        }
    }
    // Phase 2: size, least recently used first. The snapshot is not evictable
    // but its bytes count: a graph directory over the limit with it alone keeps
    // no cache at all.
    if let Some(max_size) = max_size {
        let mut total = cached + snapshot;
        if total > max_size {
            files.sort_by_key(|f| f.used);
            for file in &files {
                if total <= max_size {
                    break;
                }
                if std::fs::remove_file(&file.path).is_err() {
                    continue;
                }
                report.cached += 1;
                report.bytes += file.bytes;
                total = total.saturating_sub(file.bytes);
            }
        }
    }
    Ok(report)
}

/// The names of the files a build or a query keeps in the caches below
/// [`cache_dir`], one constant per file in [`cache_file`], and [`CACHE_FILES`]:
/// a cache is named once, so a writer and the eviction cannot list different
/// names.
macro_rules! cache_files {
    ($($(#[$doc:meta])* $name:ident = $file:literal;)*) => {
        /// The names of the files a build or a query keeps in the caches below
        /// [`crate::cache_dir`], one constant per file: the writer and
        /// [`crate::CACHE_FILES`] use the same constant.
        pub mod cache_file {
            $($(#[$doc])* pub const $name: &str = $file;)*
        }

        /// The files in the caches below [`cache_dir`] that [`evict`] may
        /// delete, and the only ones: the cache directory is checkout-controlled
        /// when it sits in the graph directory, so a file there that no build or
        /// query wrote is not the cleanup's to delete. Every name of
        /// [`cache_file`] is here; the note naming the graph directory a
        /// per-machine cache belongs to (`graph-dir`) is deliberately not.
        pub const CACHE_FILES: &[&str] = &[$(cache_file::$name),*];
    };
}

cache_files! {
    /// Content hashes keyed by file stamp ([`crate::hash::HashCache`]).
    HASHES = "hashes";
    /// Small setup files keyed by file stamp ([`crate::smallfiles::SmallFiles`]).
    MANIFESTS = "manifests";
    /// The previous walk with its directory stamps ([`crate::walk::walk_cached`]).
    WALK = "walk";
    /// The tree parts and listings of the last build (`build.rs`).
    OBS = "obs";
    /// The composed parts of the last composition (`compose.rs`).
    COMPOSED = "composed";
    /// The snapshot the composed parts were composed for (`compose/reuse.rs`).
    COMPOSED_FOR = "composed.for";
    /// The lookup tables of the last composition (`compose/tables.rs`).
    TABLES = "tables";
    /// The derived environment layer (`compose/env.rs`).
    DERIVED_ENV = "derived-env";
    /// The derived manual-mock layer (`compose/mocks.rs`).
    DERIVED_MOCKS = "derived-mocks";
    /// The derived cross-file binding layer (`turborepo-trace/src/xfile.rs`).
    DERIVED_BOUND = "derived-bound";
    /// The names layer's data (`turborepo-trace/src/names.rs`).
    NAMES_DATA = "names-data";
    /// Cross-file facts keyed by content hash and file name
    /// (`turborepo-trace/src/xfile.rs`).
    XFILE_FACTS = "xfile-facts";
    /// The recorded run of cross-file binding (`turborepo-trace/src/xfile.rs`).
    XFILE_TRACE = "xfile-trace";
    /// A pack in the pre-log location; a build removes it.
    PACK = "pack";
}

/// A file of a graph directory, for [`evict`].
struct FileUse {
    name: String,
    path: PathBuf,
    bytes: u64,
    /// When the file last changed, which is when it was last written: a build
    /// only rewrites a cache file whose contents differ, and every read of a
    /// changed file writes it again.
    used: std::time::SystemTime,
}

/// Whether the store of `out` already holds exactly what [`gc`] would leave:
/// the manifest's shards, in the pack, and its walked-file list. No log (its
/// records are replaced versions until a compaction), no other shard, no
/// walked-file list no snapshot names.
fn store_is_current(out: &Path, manifest: &Manifest) -> bool {
    let store = out.join("store");
    let (packed, _) = shard::Pack::index_only(&store);
    if packed.len() != manifest.shards.len()
        || !manifest.shards.iter().all(|s| packed.contains(&s.key))
    {
        return false;
    }
    let keep: std::collections::HashSet<&str> = [shard::PACK_FILE]
        .into_iter()
        .chain(manifest.files.as_deref())
        .collect();
    files_below(&store)
        .iter()
        .all(|f| keep.contains(f.name.as_str()))
}

/// The regular files directly below `dir`. Symbolic links and directories are
/// not files a build wrote, and are never followed or removed.
fn files_below(dir: &Path) -> Vec<FileUse> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        files.push(FileUse {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            bytes: meta.len(),
            used: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
        });
    }
    files
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    /// A wait for one graph directory's cache writes ends when they are on
    /// disk, also while another thread waits (for every graph, as the
    /// process does before it exits, or for another graph, as a parallel
    /// build does). The waits used to share one list of handles, and the
    /// first waiter took them all: the other returned at once.
    #[test]
    fn a_wait_for_a_graphs_cache_writes_is_not_taken_by_another_waiter() {
        let a = Path::new("/nonexistent/turbo-trace-writes-test/a");
        let b = Path::new("/nonexistent/turbo-trace-writes-test/b");
        let written = Arc::new(AtomicBool::new(false));
        let (release, released) = std::sync::mpsc::channel::<()>();
        let w = Arc::clone(&written);
        spawn_cache_write(
            a,
            move || {
                let _ = released.recv();
                std::thread::sleep(std::time::Duration::from_millis(100));
                w.store(true, Ordering::SeqCst);
            },
            || {},
        );
        // Nothing is being written below `b`.
        wait_for_cache_writes_in(b);
        assert!(!written.load(Ordering::SeqCst));
        // Another waiter, for every graph, is already waiting when this thread starts
        // to.
        let other = std::thread::spawn(wait_for_cache_writes);
        std::thread::sleep(std::time::Duration::from_millis(50));
        release.send(()).unwrap();
        wait_for_cache_writes_in(a);
        assert!(written.load(Ordering::SeqCst));
        other.join().unwrap();
    }

    /// A write that panics is not waited for forever, nor one that is never
    /// started.
    #[test]
    fn a_failed_cache_write_ends_the_wait() {
        let a = Path::new("/nonexistent/turbo-trace-writes-test/fails");
        spawn_cache_write(
            a,
            || panic!("a cache write fails (expected in this test)"),
            || {},
        );
        wait_for_cache_writes_in(a);
        drop(Writing::begin(a));
        wait_for_cache_writes_in(a);
    }

    #[test]
    fn an_unchanged_file_is_not_rewritten() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("f");
        assert!(write_if_changed(&p, b"one").unwrap());
        let file = std::fs::metadata(&p).unwrap().modified().unwrap();
        let dir = std::fs::metadata(t.path()).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!write_if_changed(&p, b"one").unwrap());
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), file);
        assert_eq!(
            std::fs::metadata(t.path()).unwrap().modified().unwrap(),
            dir
        );
        assert!(write_if_changed(&p, b"two").unwrap());
        assert!(write_if_changed(&p, b"owt").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), b"owt");
    }
}
