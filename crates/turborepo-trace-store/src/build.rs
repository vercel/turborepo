//! The graph build: whole-repo, incremental.
//!
//! 1. Walk the repository ([`crate::walk`]) and discover workspace packages
//!    ([`crate::workspace`]). Files with a registered extension are entries of
//!    their package.
//! 2. For each first-party package with a shard in the previous manifest,
//!    recompute its key from the current inputs and the previous shard's
//!    observations ([`Key`]); an equal key means the shard is reused as is.
//! 3. In every other package, find the files that read nothing that changed:
//!    they keep their records from the previous shard ([`crate::filereuse`]).
//!    Analyze the rest in one parallel [`Tracer::analyze`] call, reusing
//!    per-file extractions whose content hash is unchanged.
//! 4. Assemble a shard per traced package from the fresh and the kept files:
//!    edges in symbolic form, normalized observations ([`crate::normalize`]),
//!    the observations the key validates ([`ObsRec`]) and their state per table
//!    entry.
//! 5. Third-party packages: one opaque shard per `name@version` the first-party
//!    shards reach, or, with [`Externals::Follow`], traced shards for every
//!    package reached transitively.
//! 6. Write new shards to `store/<key>` and the manifest `graph.json`.
//!
//! # Keys
//!
//! A key is the xxh3-128 of the shard's inputs:
//!
//! - first party: format and crate version, the trace options, the package
//!   (name, root, manifest contents), every entry's path and content hash, and
//!   the current state of everything the previous trace observed: each probe
//!   still missing, each membership's matching file list (for a listing that
//!   only membership hubs reference: what the hubs store of it,
//!   [`crate::tasks::Tasks::hub_state`]), each config input's contents (only
//!   its path and existence for path-only inputs), each dependency closure's
//!   package roots (turbo `^task`), each reached file still present in the same
//!   package, and each package link still resolving to the same `name@version`
//!   and integrity (a name the lockfile does not resolve: still unresolved).
//!   The last item is what "the resolved versions of only the packages it
//!   references" means here: a lockfile bump re-keys exactly the shards whose
//!   links now resolve differently.
//! - opaque third party: format and crate version, the options (including the
//!   platform), name, version and integrity.
//! - traced third party: the same plus the observation state, so a package
//!   whose own dependencies resolve differently (pnpm peer variants, a
//!   transitive bump) is re-traced.
//!
//! A reused shard is byte-identical to what a fresh trace of the same tree
//! would produce, as long as the observations are complete; the property test
//! in `tests/compose.rs` checks this across random edits.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use rayon::prelude::*;
use turborepo_trace_core::{
    AnalyzeInput, DirCache, FileAnalysis, Language, PackageBoundary, Tracer, Uncertainty,
    timing::{Span, mark},
};
use turborepo_trace_graph::glob::Glob;
use xxhash_rust::xxh3::Xxh3;

use crate::{
    Error, GraphOptions,
    convert::{cached_extraction, edge_attr, extraction_rec_in, unc_rec},
    hash::{HashCache, join, split},
    identity::{Identities, load_lockfiles_deferred},
    manifest::{Manifest, ShardEntry},
    normalize::{NormCx, Tables},
    paths::{abs, rel_to},
    shard::{
        self, ArchivedShard, CrossRec, EdgeRec, FILE_LEVEL, FileRec, LinkRec, LoadedShard, ObsRec,
        PackageRec, RefRec, Shard, TargetRec, UncRec, kind,
    },
    workspace::{PackageDef, Workspace},
    write_atomic,
};

/// Third-party policy for graph builds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Externals {
    /// One opaque node per `name@version` (default).
    #[default]
    Opaque,
    /// Trace into third-party packages, one shard per `name@version`.
    Follow,
}

impl Externals {
    /// Stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            Externals::Opaque => "opaque",
            Externals::Follow => "follow",
        }
    }
}

/// What a build did.
#[derive(Clone, Debug, Default)]
pub struct BuildReport {
    /// Files walked.
    pub files: usize,
    /// First-party packages.
    pub first_party: usize,
    /// Third-party packages (opaque or traced).
    pub third_party: usize,
    /// Shards reused from the store without tracing.
    pub reused: usize,
    /// Shards traced in this build.
    pub traced: usize,
    /// Files analyzed in this build.
    pub analyzed_files: usize,
    /// Of those, files whose extraction was reused (unchanged content hash).
    pub reused_extractions: usize,
    /// Files of traced shards that were not analyzed: their records were taken
    /// from the package's previous shard, because nothing they observed
    /// changed.
    pub kept_files: usize,
    /// Shard files newly written to the store.
    pub written: usize,
    /// Milliseconds per phase: walk, validate, trace, assemble+write,
    /// third-party, total.
    pub timings_ms: [u64; 6],
    /// The manifest written.
    pub manifest: Manifest,
    /// Whether the walk's file list came from the walk cache.
    pub walk_cached: bool,
    /// Shards whose observed tree state was reused from the previous build
    /// (unchanged walked tree and outside directories), and shards whose
    /// observations were re-checked.
    pub tree_reused: [usize; 2],
    /// Whether `graph.json` was written (it is left alone when it already says
    /// the same).
    pub manifest_written: bool,
    /// Whether the observation cache is written (in the background): not when
    /// every tree part and listing was served from it and nothing changed.
    pub obs_written: bool,
    /// The files the walk found (repo-relative, `.gitignore` respected,
    /// sorted), for callers that need the tree's file list (untracked files
    /// for `affected --since`).
    pub walked: Vec<String>,
    /// The symbolic links the walk passed (repo-relative, `.gitignore`
    /// respected, sorted; [`crate::walk::Walk::links`]): entries of the
    /// tree that are no files of it.
    pub walked_links: Vec<String>,
}

/// The build identity, part of the options digest and of packed graphs: turbo's
/// version (`version.txt`) and the format versions of everything that decides
/// shard contents. A change to what a shard, a task record or a framework
/// integration holds bumps one of these constants, so a binary of another
/// release or format reuses nothing.
pub fn build_id() -> &'static str {
    static ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            "turbo-{}/shard-{}/{}/{}/xfile-{}/manifest-{}/artifact-{}",
            turbo_version(),
            shard::FORMAT_VERSION,
            crate::tasks::TASKS_VERSION,
            crate::fw::INTEGRATION_VERSION,
            turborepo_trace_lang_js::xfile::codec::VERSION,
            crate::manifest::VERSION,
            crate::artifact::VERSION,
        )
    });
    &ID
}

/// turbo's version: the first line of the repository's `version.txt` (the
/// release channel follows on the second line).
fn turbo_version() -> &'static str {
    let v = include_str!("../../../version.txt");
    v.split_once('\n')
        .map_or(v, |(version, _)| version)
        .trim_end()
}

/// The digest of the options that affect shard contents.
pub fn options_digest(opts: &GraphOptions, tracer: &Tracer) -> String {
    options_digest_for(opts, tracer, build_id())
}

/// [`options_digest`] for the build identity `build_id`.
pub fn options_digest_for(opts: &GraphOptions, tracer: &Tracer, build_id: &str) -> String {
    let o = tracer.options();
    let mut h = Xxh3::new();
    let mut put = |s: &str| {
        h.update(&(s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    put("turbotrace-graph");
    put(env!("CARGO_PKG_VERSION"));
    put(build_id);
    put(&shard::FORMAT_VERSION.to_string());
    put(opts.externals.as_str());
    put(&o.conditions.join(","));
    put(&o.all_conditions.to_string());
    put(&o.include_types.to_string());
    put(&o.max_glob.to_string());
    put(&o.all_targets.to_string());
    put(crate::fw::INTEGRATION_VERSION);
    put(crate::tasks::TASKS_VERSION);
    put("generated-headers-1");
    put(&o.platform().os_arch());
    put(&format!("{:?}", o.platform()));
    let mut langs: Vec<String> = tracer
        .languages()
        .iter()
        .map(|l| format!("{}{}", l.id().0, l.config_key()))
        .collect();
    langs.sort_unstable();
    put(&langs.join(","));
    format!("{:032x}", h.digest128())
}

/// The tracer a build of `repo` with `opts` and `langs` uses.
fn tracer_for(
    repo: &Path,
    opts: &GraphOptions,
    langs: &[Arc<dyn Language>],
    graph_in_tree: Option<&Path>,
) -> Result<Tracer, turborepo_trace_core::TraceError> {
    let mut b = Tracer::builder()
        .base(repo)
        .cwd(repo)
        // A graph is a function of the repository tree: paths outside it are never
        // looked up on the build machine.
        .confined(true)
        // Nor is the graph directory part of the tree, wherever it is and whatever its
        // name: a build writes into it, so whatever observed it (a listing, a glob, a
        // probe below it) would differ after every build.
        .hide(graph_in_tree)
        // A graph that another build keeps in the tree under the default name is not
        // part of it either: a build with its graph elsewhere sees the same tree.
        .hide_named(crate::GRAPH_DIR)
        .record(true)
        .include_types(opts.include_types)
        .conditions(opts.conditions.iter().cloned())
        .all_conditions(opts.all_conditions)
        .max_glob(opts.max_glob)
        .all_targets(opts.all_targets)
        .threads(opts.threads);
    if let Some(t) = &opts.target {
        b = b.target(t);
    }
    for l in langs {
        b = b.language_arc(Arc::clone(l));
    }
    b.build()
}

/// The options digest [`build`] would record for `opts` and `langs` with this
/// binary on this host, without building: what a stored graph or a graph
/// artifact ([`crate::artifact`]) must carry to be reused here.
pub fn options_digest_of(
    opts: &GraphOptions,
    langs: &[Arc<dyn Language>],
) -> Result<String, Error> {
    let repo = std::fs::canonicalize(&opts.root).map_err(|e| Error::io(&opts.root, e))?;
    let repo = turborepo_trace_core::fs::normalize(&repo);
    // One thread: only the options and the languages are read.
    let opts = GraphOptions {
        threads: 1,
        ..opts.clone()
    };
    // What is hidden from the tracer does not enter the digest.
    let tracer = tracer_for(&repo, &opts, langs, None).map_err(Error::Trace)?;
    Ok(options_digest(&opts, &tracer))
}

struct Hasher(Xxh3);

impl Hasher {
    fn new(tag: &str) -> Self {
        let mut h = Hasher(Xxh3::new());
        h.str(tag);
        h
    }
    fn str(&mut self, s: &str) {
        self.0.update(&(s.len() as u64).to_le_bytes());
        self.0.update(s.as_bytes());
    }
    fn u128(&mut self, x: u128) {
        self.0.update(&x.to_le_bytes());
    }
    fn opt(&mut self, s: Option<&str>) {
        match s {
            Some(s) => {
                self.0.update(&[1]);
                self.str(s);
            }
            None => self.0.update(&[0]),
        }
    }
    fn hex(self) -> String {
        format!("{:032x}", self.0.digest128())
    }
}

/// Shared state of one build.
pub(crate) struct Cx {
    pub repo: PathBuf,
    pub tracer: Arc<Tracer>,
    /// The tracer's environment roots outside the repository.
    pub env_roots: Vec<PathBuf>,
    pub ws: Workspace,
    pub ids: Identities,
    pub hashes: HashCache,
    pub opts_digest: String,
    pub store: PathBuf,
    /// Framework config files of the walked tree (per-package key part,
    /// [`crate::fw::ConfigPaths::digest_for`]).
    pub fw_configs: crate::fw::ConfigPaths,
    /// Membership listing digests of this build, by directory and pattern, with
    /// whether the listing stayed inside walked directories.
    pub members: MembershipMemo,
    /// Directories the repository walk entered (absolute).
    pub walked_dirs: std::collections::HashSet<PathBuf>,
    /// The tree index for deep listings (started when the tree changed).
    tree_index: crate::treeindex::Slot,
    /// `Some` when tree parts of the previous build can be reused: the walked
    /// directories that changed since they were computed (empty when the
    /// walked tree is unchanged).
    pub tree_changed: Option<Changed>,
    /// Tree-part digests of shards from the previous build (see
    /// `tree_changed`), by key.
    pub obs_prev: HashMap<String, TreePart>,
    /// Tree-part digests of this build's shards, by key.
    pub obs_new: std::sync::Mutex<HashMap<String, TreePart>>,
    /// When the build started (ns since the epoch): outside directories
    /// modified less than two seconds before are not trusted for reuse.
    pub start_ns: i128,
    /// Tree parts reused from the previous build, and computed.
    pub tree_stats: [std::sync::atomic::AtomicUsize; 2],
    /// Membership listings computed by this build (not served from the previous
    /// build's).
    pub fresh_listings: std::sync::atomic::AtomicUsize,
    /// Per-build memos of the observation checks every shard repeats (the same
    /// configs, package links and dependency closures appear in hundreds of
    /// shards): digest of a link location's current identity, whether a
    /// path-only config is a file, digest of a package's dependency
    /// closure.
    pub link_memo: dashmap::DashMap<PathBuf, u128>,
    /// [`seen_digest`] per place, per build.
    pub seen_memo: dashmap::DashMap<PathBuf, Arc<Seen>>,
    /// Digest of a manifest's resolution view ([`view_digest`]), per build.
    pub view_memo: dashmap::DashMap<PathBuf, Option<u128>>,
    /// Memo for [`view_digest`] of a loader view.
    pub loader_view_memo: dashmap::DashMap<PathBuf, Option<u128>>,
    pub is_file_memo: dashmap::DashMap<PathBuf, bool>,
    pub closure_memo: dashmap::DashMap<String, u128>,
    /// The states of the shards' location records ([`crate::located`]), per
    /// build.
    pub places: crate::located::Places,
    /// Directory mtimes looked up during this build.
    pub dir_mtimes: std::sync::Mutex<HashMap<PathBuf, Option<i128>>>,
    /// Deep listings of the previous build that entered more outside
    /// directories than a shard's tree part keeps (see `tree_changed`), see
    /// [`Listings`].
    pub listings_prev: Listings,
    /// The task workspace (dependency closures for `^task`).
    pub task_graph: crate::tasks::TaskGraph,
    /// The store's pack and log as they were when the build started
    /// ([`shard::Pack`]).
    pub pack: Option<shard::Pack>,
    /// Shards written by this build, appended to the pack's log in batches.
    pub sink: std::sync::Mutex<ShardSink>,
}

/// Shards waiting to be appended to the store's log ([`write_shard`]).
#[derive(Default)]
pub(crate) struct ShardSink {
    staged: Vec<(String, Vec<u8>)>,
    bytes: usize,
    keys: std::collections::HashSet<String>,
    /// End of the log's last complete record, when known.
    log_end: Option<usize>,
}

/// Staged shard bytes that trigger an append during the build.
const SINK_FLUSH_BYTES: usize = 64 << 20;

/// Files analyzed per batch before their packages are assembled (bounds the
/// analyses held at once in a cold build; a refresh of a few packages is one
/// batch).
const ANALYZE_BATCH_FILES: usize = 16_384;

impl ShardSink {
    fn flush(&mut self, store: &Path) -> Result<(), Error> {
        if self.staged.is_empty() {
            return Ok(());
        }
        let log = store.join(shard::LOG_FILE);
        shard::Pack::append(store, &self.staged, self.log_end).map_err(|e| Error::io(&log, e))?;
        self.log_end = std::fs::metadata(&log).ok().map(|m| m.len() as usize);
        self.staged.clear();
        self.bytes = 0;
        Ok(())
    }
}

/// Diagnostics (`TURBO_TRACE_DEBUG_TIMING`): time and count of fresh membership
/// listings, shallow and deep.
static LISTING_US: [std::sync::atomic::AtomicUsize; 2] = [
    std::sync::atomic::AtomicUsize::new(0),
    std::sync::atomic::AtomicUsize::new(0),
];
static LISTING_N: [std::sync::atomic::AtomicUsize; 2] = [
    std::sync::atomic::AtomicUsize::new(0),
    std::sync::atomic::AtomicUsize::new(0),
];

/// Deep membership listings stored once per build instead of once per shard: a
/// listing that enters more than [`MAX_OUTSIDE_DIRS`] directories outside the
/// walk (gitignored `dist/`, `.next/` trees below a root `**/__mocks__/**` or
/// task-discovery listing) is reusable while the walked tree and each of those
/// directories keep their mtimes. Shards reference such a listing by key
/// ([`TreePart::listings`]) instead of becoming unreusable.
#[derive(Default)]
pub(crate) struct Listings {
    /// Outside directories with the mtimes they had when listed.
    dirs: Vec<(PathBuf, i128)>,
    /// Digest and indexes into `dirs`, by listed directory and pattern.
    entries: rustc_hash::FxHashMap<(PathBuf, String), (u128, Vec<u32>)>,
    /// Per entry of `dirs`: whether its mtime is unchanged (checked in parallel
    /// on first use).
    valid: std::sync::OnceLock<Vec<bool>>,
}

impl Listings {
    /// Drops the listings that read a changed walked directory (a deep listing
    /// reads every walked directory below its own).
    fn retain_unchanged(&mut self, repo: &Path, changed: &Changed) {
        if changed.dirs.is_empty() {
            return;
        }
        self.entries.retain(|(d, _), _| {
            d.strip_prefix(repo)
                .map_or(true, |r| !changed.under(&r.to_string_lossy()))
        });
    }

    fn valid(&self) -> &[bool] {
        // Nested rayon work: never inside `get_or_init`, which blocks the workers that
        // ask meanwhile (see `crate::once`; this deadlocked untouched refreshes
        // of a large monorepo). The build computes it once up front, off the
        // pool ([`Listings::warm`]).
        crate::once::init_once(&self.valid, || {
            self.dirs
                .par_iter()
                .map(|(p, m)| dir_mtime(p) == Some(*m))
                .collect()
        })
    }

    /// Checks the outside directories' mtimes now (see [`Listings::valid`]).
    fn warm(&self) {
        if !self.dirs.is_empty() {
            let _ = self.valid();
        }
    }

    /// Whether the listing of `dir` with `pattern` is known and still valid
    /// now.
    fn is_valid(&self, dir: &Path, pattern: &str) -> bool {
        self.entries
            .get(&(dir.to_path_buf(), pattern.to_owned()))
            .is_some_and(|(_, ix)| {
                let valid = self.valid();
                ix.iter().all(|&i| valid[i as usize])
            })
    }

    /// Every listing still valid now, with its outside directories' (unchanged)
    /// mtimes.
    fn still_valid(&self) -> DeepListings {
        if self.entries.is_empty() {
            return DeepListings::default();
        }
        let valid = self.valid();
        self.entries
            .iter()
            .filter(|(_, (_, ix))| ix.iter().all(|&i| valid[i as usize]))
            .map(|(k, (d, ix))| {
                let dirs: Vec<Stamped> = ix
                    .iter()
                    .map(|&i| {
                        let (p, m) = &self.dirs[i as usize];
                        (p.clone(), Some(*m))
                    })
                    .collect();
                (k.clone(), (*d, Arc::new(dirs)))
            })
            .collect()
    }

    /// The digest and outside directories (with their unchanged mtimes) of a
    /// listing still valid now.
    fn get(&self, dir: &Path, pattern: &str) -> Option<(u128, Vec<Stamped>)> {
        let (d, ix) = self.entries.get(&(dir.to_path_buf(), pattern.to_owned()))?;
        let valid = self.valid();
        ix.iter().all(|&i| valid[i as usize]).then(|| {
            (
                *d,
                ix.iter()
                    .map(|&i| {
                        let (p, m) = &self.dirs[i as usize];
                        (p.clone(), Some(*m))
                    })
                    .collect(),
            )
        })
    }
}

/// The tree part of a shard's observation digest and what it can be reused
/// under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreePart {
    pub digest: u128,
    /// `None`: never reused (a deep listing left the walked directories, or an
    /// outside directory was modified too recently to trust its mtime).
    /// `Some(dirs)`: reusable while the walked tree is unchanged and each
    /// of these directories outside it (the nearest existing ancestors of
    /// outside observations) keeps its mtime.
    pub reuse: Option<Vec<(PathBuf, i128)>>,
    /// Deep listings (directory, pattern) the tree part also depends on, valid
    /// through the build's [`Listings`].
    pub listings: Vec<(PathBuf, String)>,
    /// The walked directories the tree part depends on: it is reusable after a
    /// tree change that touched none of them ([`Changed`]).
    pub walked: WalkedDeps,
}

/// Walked directories (repo-relative) a tree part depends on: a probe, a
/// reached file or a shallow listing depends on the listing of one directory
/// (`exact`, as hashes); a deep listing on every walked directory below its own
/// (`deep`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WalkedDeps {
    /// [`dir_hash`] of each directory, sorted and deduplicated.
    pub exact: Vec<u64>,
    /// Directories whose whole walked subtree counts, sorted and deduplicated.
    pub deep: Vec<String>,
}

/// The hash of a repo-relative directory in [`WalkedDeps::exact`].
fn dir_hash(rel: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(rel.as_bytes())
}

/// Walked directories whose listing changed since the previous build's tree
/// parts were computed ([`crate::walk::Walk::changed`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Changed {
    exact: std::collections::HashSet<u64>,
    dirs: Vec<String>,
    /// The obs cache was written for exactly the current walk (nothing
    /// changed).
    same_walk: bool,
}

/// Digest of a directory listing: entry names and kinds (entries are sorted by
/// name).
fn listing_digest(l: &turborepo_trace_core::fs::DirListing) -> u64 {
    let mut h = Xxh3::new();
    for e in l.iter() {
        h.update(e.name_bytes());
        h.update(&[0, e.kind() as u8]);
    }
    h.digest()
}

impl Changed {
    /// Drops the directories whose listing digest is the one recorded by the
    /// previous build (`old`, by [`dir_hash`]): their mtime changed but not
    /// their entries (a file rewritten through a rename, as `git checkout`
    /// does).
    fn refine(self, old: &HashMap<u64, u64>, now: impl Fn(&str) -> Option<u64> + Sync) -> Self {
        let same_walk = self.same_walk;
        let dirs: Vec<String> = self
            .dirs
            .into_par_iter()
            .filter(|d| match old.get(&dir_hash(d)) {
                Some(o) => now(d) != Some(*o),
                None => true,
            })
            .collect();
        Changed {
            same_walk,
            ..Changed::new(dirs)
        }
    }

    fn new(dirs: Vec<String>) -> Self {
        Changed {
            exact: dirs.iter().map(|d| dir_hash(d)).collect(),
            dirs,
            same_walk: false,
        }
    }

    /// Whether a changed directory is `d` or below it (repo-relative, `""` the
    /// root).
    fn under(&self, d: &str) -> bool {
        self.dirs.iter().any(|c| {
            d.is_empty()
                || (c.starts_with(d) && (c.len() == d.len() || c.as_bytes()[d.len()] == b'/'))
        })
    }

    /// Whether a change touches what `w` depends on.
    fn touches(&self, w: &WalkedDeps) -> bool {
        if self.dirs.is_empty() {
            return false;
        }
        w.exact.iter().any(|h| self.exact.contains(h)) || w.deep.iter().any(|d| self.under(d))
    }
}

/// An outside directory with its mtime when already checked.
type Stamped = (PathBuf, Option<i128>);
/// Deep listings of a build by (directory, pattern): digest and outside
/// directories.
type DeepListings = rustc_hash::FxHashMap<(PathBuf, String), (u128, Arc<Vec<Stamped>>)>;

/// How a membership listing is covered for reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ListCover {
    /// By the walked tree and these outside directories' mtimes.
    Dirs(Vec<PathBuf>),
    /// By the walked tree and many outside directories' mtimes, kept once per
    /// build in [`Listings`] (with the mtimes already checked, for a
    /// listing of the previous build).
    Listing(Arc<Vec<(PathBuf, Option<i128>)>>),
    /// Not coverable.
    No,
}

/// How an observed path is covered for reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Cover {
    /// By this walked directory's mtime (listings of walked directories are
    /// compared between walks, [`Changed`]).
    Walked(PathBuf),
    /// By this directory's mtime (its nearest existing ancestor outside the
    /// walk).
    Dir(PathBuf),
    /// Not coverable.
    No,
}

/// Membership digests computed once per build.
pub(crate) type MembershipMemo =
    std::sync::Mutex<HashMap<(PathBuf, String), Arc<std::sync::OnceLock<(u128, ListCover)>>>>;

/// Whether a membership pattern lists a subtree (not one directory's entries).
fn is_deep_pattern(pattern: &str) -> bool {
    pattern.contains('/') || pattern.contains("**")
}

/// Outside directories a shard's tree part keeps for one listing; a listing
/// entering more is kept in [`Listings`].
const MAX_OUTSIDE_DIRS: usize = 256;
/// Outside directories a deep listing may enter and still be reusable through
/// their mtimes.
const MAX_LISTING_OUTSIDE_DIRS: usize = 100_000;

fn dir_mtime(p: &Path) -> Option<i128> {
    use turborepo_trace_core::os::MetadataExt;
    let m = std::fs::metadata(p).ok()?;
    Some(m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128)
}

fn now_ns() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

const OBS_MAGIC: &[u8; 8] = b"DTOBS05\n";

/// A little-endian reader over a cache file body.
struct Rd<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
    fn i128(&mut self) -> Option<i128> {
        Some(i128::from_le_bytes(self.take(16)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        Some(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

fn put_u32(b: &mut Vec<u8>, x: usize) {
    b.extend_from_slice(&(x as u32).to_le_bytes());
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    put_u32(b, s.len());
    b.extend_from_slice(s.as_bytes());
}

/// Reads `<cache dir>/obs` if it was written for `token` (workspace and
/// lockfiles) and for the walk the current walk is unchanged from (all tree
/// parts are candidates) or was compared with (`walk.prev`: tree parts
/// untouched by `walk.changed` are). Returns the tree parts by shard key, the
/// deep listings they reference, and the changed walked directories
/// (`None`: nothing is reusable).
#[allow(clippy::type_complexity)]
// Invariant: a fixed-width range of the slice converts to an array of that width.
#[allow(clippy::expect_used)]
fn read_obs_cache(
    out: &Path,
    token: u128,
    walk: (Option<u128>, Option<u128>, &[String]),
) -> (
    HashMap<String, TreePart>,
    Listings,
    Option<Changed>,
    HashMap<u64, u64>,
) {
    let (unchanged, prev, walk_changed) = walk;
    let mut m = HashMap::new();
    let mut l = Listings::default();
    let Ok(b) = std::fs::read(crate::cache_dir(out).join("obs")) else {
        return (m, l, None, HashMap::new());
    };
    if b.len() < 48 || &b[..8] != OBS_MAGIC || b[8..24] != token.to_le_bytes() {
        return (m, l, None, HashMap::new());
    }
    let written_for = u128::from_le_bytes(b[24..40].try_into().expect("16 bytes"));
    let changed = if unchanged == Some(written_for) {
        Changed {
            same_walk: true,
            ..Changed::default()
        }
    } else if prev == Some(written_for) {
        Changed::new(walk_changed.to_vec())
    } else {
        return (m, l, None, HashMap::new());
    };
    let (body, sum) = b.split_at(b.len() - 8);
    if xxhash_rust::xxh3::xxh3_64(body).to_le_bytes() != sum {
        return (m, l, None, HashMap::new());
    }
    let mut r = Rd { b: body, pos: 40 };
    let mut digests: HashMap<u64, u64> = HashMap::new();
    let parsed = (|| -> Option<()> {
        for _ in 0..r.u32()? {
            let k = r.str()?;
            let digest = r.u128()?;
            let n = r.u32()?;
            let reuse = if n == u32::MAX {
                None
            } else {
                let mut dirs = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    let p = r.str()?;
                    dirs.push((PathBuf::from(p), r.i128()?));
                }
                Some(dirs)
            };
            let mut listings = Vec::new();
            for _ in 0..r.u32()? {
                let d = r.str()?;
                listings.push((PathBuf::from(d), r.str()?));
            }
            let mut walked = WalkedDeps::default();
            for _ in 0..r.u32()? {
                walked.exact.push(r.u64()?);
            }
            for _ in 0..r.u32()? {
                walked.deep.push(r.str()?);
            }
            m.insert(
                k,
                TreePart {
                    digest,
                    reuse,
                    listings,
                    walked,
                },
            );
        }
        for _ in 0..r.u32()? {
            let p = r.str()?;
            l.dirs.push((PathBuf::from(p), r.i128()?));
        }
        for _ in 0..r.u32()? {
            let d = r.str()?;
            let pat = r.str()?;
            let digest = r.u128()?;
            let n = r.u32()? as usize;
            let mut ix = Vec::with_capacity(n);
            for _ in 0..n {
                let i = r.u32()?;
                if i as usize >= l.dirs.len() {
                    return None;
                }
                ix.push(i);
            }
            l.entries.insert((PathBuf::from(d), pat), (digest, ix));
        }
        for _ in 0..r.u32()? {
            let k = r.u64()?;
            digests.insert(k, r.u64()?);
        }
        Some(())
    })();
    if parsed.is_none() {
        return (HashMap::new(), Listings::default(), None, HashMap::new());
    }
    (m, l, Some(changed), digests)
}

/// The obs cache token: the workspace's packages (ownership of reached files is
/// part of the tree digest and follows manifests, not directories). The walk
/// the cache was written for is stored next to it ([`read_obs_cache`]).
fn obs_token(ws: &Workspace) -> u128 {
    let mut h = Hasher::new("obs-token");
    for p in &ws.packages {
        h.str(&p.name);
        h.str(&p.root);
    }
    h.0.digest128()
}

/// Writes `<cache dir>/obs` for the current walk cache (removes it without
/// one): the tree parts, and the deep listings (every one a tree part
/// references, and the others the build knows), stamped with the outside
/// directories' mtimes (a listing with a directory modified since `racy` is
/// left out, and so the shards referencing it are not reused next time).
fn write_obs_cache(
    out: &Path,
    ws: &Workspace,
    entries: &HashMap<String, TreePart>,
    dir_digests: &[(u64, u64)],
    listings: &DeepListings,
    mtime: &dyn Fn(&Path) -> Option<i128>,
    racy: i128,
) {
    let path = crate::cache_dir(out).join("obs");
    let Some(walk) = crate::walk::cache_token(out) else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    let mut b = OBS_MAGIC.to_vec();
    b.extend_from_slice(&obs_token(ws).to_le_bytes());
    b.extend_from_slice(&walk.to_le_bytes());
    let mut keys: Vec<&String> = entries.keys().collect();
    keys.sort_unstable();
    put_u32(&mut b, keys.len());
    let mut used: std::collections::BTreeSet<&(PathBuf, String)> = Default::default();
    for k in keys {
        let t = &entries[k];
        put_str(&mut b, k);
        b.extend_from_slice(&t.digest.to_le_bytes());
        match &t.reuse {
            None => b.extend_from_slice(&u32::MAX.to_le_bytes()),
            Some(dirs) => {
                put_u32(&mut b, dirs.len());
                for (p, m) in dirs {
                    put_str(&mut b, &p.to_string_lossy());
                    b.extend_from_slice(&m.to_le_bytes());
                }
            }
        }
        put_u32(&mut b, t.listings.len());
        for l in &t.listings {
            put_str(&mut b, &l.0.to_string_lossy());
            put_str(&mut b, &l.1);
            used.insert(l);
        }
        put_u32(&mut b, t.walked.exact.len());
        for x in &t.walked.exact {
            b.extend_from_slice(&x.to_le_bytes());
        }
        put_u32(&mut b, t.walked.deep.len());
        for d in &t.walked.deep {
            put_str(&mut b, d);
        }
    }
    // Listings: interned outside directories, then the entries.
    let mut dir_ix: rustc_hash::FxHashMap<&Path, u32> = Default::default();
    let mut dirs: Vec<(&Path, i128)> = Vec::new();
    let mut recs: Vec<(&(PathBuf, String), u128, Vec<u32>)> = Vec::new();
    used.extend(listings.keys());
    for key in used {
        let Some((digest, outside)) = listings.get(key) else {
            continue;
        };
        let mut ix = Vec::with_capacity(outside.len());
        let ok = outside.iter().all(|(d, known)| {
            if let Some(&i) = dir_ix.get(d.as_path()) {
                ix.push(i);
                return true;
            }
            let Some(m) = known.or_else(|| mtime(d)).filter(|m| *m < racy) else {
                return false;
            };
            let i = dirs.len() as u32;
            dirs.push((d, m));
            dir_ix.insert(d, i);
            ix.push(i);
            true
        });
        if ok {
            recs.push((key, *digest, ix));
        }
    }
    put_u32(&mut b, dirs.len());
    for (p, m) in &dirs {
        put_str(&mut b, &p.to_string_lossy());
        b.extend_from_slice(&m.to_le_bytes());
    }
    put_u32(&mut b, recs.len());
    for ((d, pat), digest, ix) in recs {
        put_str(&mut b, &d.to_string_lossy());
        put_str(&mut b, pat);
        b.extend_from_slice(&digest.to_le_bytes());
        put_u32(&mut b, ix.len());
        for i in ix {
            b.extend_from_slice(&i.to_le_bytes());
        }
    }
    // Listing digests of the walked directories this build listed.
    put_u32(&mut b, dir_digests.len());
    for (k, d) in dir_digests {
        b.extend_from_slice(&k.to_le_bytes());
        b.extend_from_slice(&d.to_le_bytes());
    }
    let sum = xxhash_rust::xxh3::xxh3_64(&b);
    b.extend_from_slice(&sum.to_le_bytes());
    crate::create_cache_dir(out);
    let _ = write_atomic(&path, &b);
}

impl Cx {
    fn fs(&self) -> &DirCache {
        self.tracer.fs()
    }

    /// xxh3-128 of [`membership_listing`] for `dir` and `pattern`, memoized for
    /// the build, and how it is covered for reuse ([`ListCover`]). A deep
    /// listing of the previous build still valid ([`Listings`]) is not
    /// listed again.
    pub(crate) fn membership_digest(&self, dir: &Path, pattern: &str) -> (u128, ListCover) {
        let cell = {
            let mut m = self.members.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(
                m.entry((dir.to_path_buf(), pattern.to_owned()))
                    .or_default(),
            )
        };
        // The listing runs nested rayon work: never inside `get_or_init` (see
        // `crate::once`).
        crate::once::init_once(&cell, || {
            if let Some((d, dirs)) = self.listings_prev.get(dir, pattern) {
                return (d, ListCover::Listing(Arc::new(dirs)));
            }
            self.fresh_listings
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tl = Instant::now();
            let mut h = Hasher::new("listing");
            let indexed = is_deep_pattern(pattern)
                .then(|| self.tree_index.get())
                .flatten()
                .and_then(|ix| {
                    ix.listing(
                        dir,
                        Glob::new(pattern).ok().as_ref(),
                        required_literal(pattern),
                        LISTING_BUDGET,
                        MAX_LISTING_OUTSIDE_DIRS,
                    )
                });
            let (list, outside) = indexed.unwrap_or_else(|| {
                membership_listing_in(self.fs(), dir, pattern, Some(&self.walked_dirs))
            });
            if turborepo_trace_core::timing::enabled() {
                let us = tl.elapsed().as_micros() as usize;
                let i = usize::from(is_deep_pattern(pattern));
                LISTING_US[i].fetch_add(us, std::sync::atomic::Ordering::Relaxed);
                LISTING_N[i].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if us > 50_000 {
                    mark(&format!(
                        "  listing {} {pattern:?}: {} ms, {} entries",
                        dir.display(),
                        us / 1000,
                        list.len()
                    ));
                }
            }
            for f in list {
                h.str(&f);
            }
            let base = if self.walked_dirs.contains(dir) {
                Cover::Walked(dir.to_path_buf())
            } else if self.fs().is_dir(dir) {
                Cover::Dir(dir.to_path_buf())
            } else {
                self.cover(dir)
            };
            let dirs = match base {
                Cover::No => None,
                Cover::Walked(_) => outside,
                Cover::Dir(d) => outside.map(|mut v| {
                    v.push(d);
                    v
                }),
            };
            let cover = match dirs {
                None => ListCover::No,
                Some(v) if v.len() <= MAX_OUTSIDE_DIRS => ListCover::Dirs(v),
                Some(v) => ListCover::Listing(Arc::new(v.into_iter().map(|d| (d, None)).collect())),
            };
            (h.0.digest128(), cover)
        })
        .clone()
    }

    /// Whether a deep listing referenced by a previous tree part is still
    /// valid.
    fn listing_valid(&self, dir: &Path, pattern: &str) -> bool {
        if !self.listings_prev.is_valid(dir, pattern) {
            return false;
        }
        // Served from the previous build without listing; recorded for the next obs
        // cache.
        let _ = self.membership_digest(dir, pattern);
        true
    }

    /// The deep listings of this build, for the obs cache: every coverable
    /// listing of a subtree (pattern with `/` or `**`), and the previous
    /// build's that are still valid. The next build reuses their digests
    /// while no walked directory below them changed and their outside
    /// directories keep their mtimes, so a refresh lists only the
    /// subtrees that changed ([`Listings`]).
    fn deep_listings(&self) -> DeepListings {
        let mut out: DeepListings = self.listings_prev.still_valid();
        for (k, cell) in self
            .members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            match cell.get() {
                Some((d, ListCover::Listing(dirs))) => {
                    out.insert(k.clone(), (*d, Arc::clone(dirs)));
                }
                Some((d, ListCover::Dirs(dirs))) if is_deep_pattern(&k.1) => {
                    out.insert(
                        k.clone(),
                        (
                            *d,
                            Arc::new(dirs.iter().map(|p| (p.clone(), None)).collect()),
                        ),
                    );
                }
                _ => {}
            }
        }
        out
    }

    /// The mtime of directory `p`, memoized for the build.
    pub(crate) fn dir_mtime(&self, p: &Path) -> Option<i128> {
        if let Some(m) = self
            .dir_mtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(p)
        {
            return *m;
        }
        let m = dir_mtime(p);
        self.dir_mtimes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(p.to_path_buf(), m);
        m
    }

    /// How adding or removing `p` (or a missing directory on the way to it)
    /// shows: in a walked directory's mtime, or in the mtime of `p`'s
    /// nearest existing ancestor.
    pub(crate) fn cover(&self, p: &Path) -> Cover {
        let mut d = p.parent();
        while let Some(x) = d {
            if self.walked_dirs.contains(x) {
                return Cover::Walked(x.to_path_buf());
            }
            if self.fs().is_dir(x) {
                return Cover::Dir(x.to_path_buf());
            }
            d = x.parent();
        }
        Cover::No
    }

    pub(crate) fn pkg_root_abs(&self, root: &str) -> PathBuf {
        if root.starts_with('/') {
            PathBuf::from(root)
        } else if root.is_empty() {
            self.repo.clone()
        } else {
            self.repo.join(root)
        }
    }

    fn normcx<'b>(&'b self, pkg_root: &'b Path, follow: bool) -> NormCx<'b> {
        NormCx {
            repo: &self.repo,
            pkg_root,
            follow,
            ids: &self.ids,
            fs: self.tracer.fs(),
            env_roots: &self.env_roots,
        }
    }
}

/// Probes that exist now, membership listings and reached files of `s`; and
/// whether all of them lie inside walked directories.
fn tree_part(
    cx: &Cx,
    s: &ArchivedShard,
    root: &Path,
    tasks: Option<&crate::tasks::Tasks<'_>>,
) -> TreePart {
    let fs = cx.fs();
    let root = root.to_path_buf();
    let mut h = Hasher::new("tree");
    let mut covers: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut reusable = true;
    let mut walked: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut deep: Vec<PathBuf> = Vec::new();
    let mut add = |c: Cover, covers: &mut std::collections::BTreeSet<PathBuf>| match c {
        Cover::Walked(_) => {}
        Cover::Dir(d) => {
            covers.insert(d);
        }
        Cover::No => reusable = false,
    };
    let tp0 = Instant::now();
    // Probes. Large tables are checked in parallel chunks (one big shard must not
    // be the long pole of a refresh); chunk digests are combined in order.
    type Chunk = (u128, Vec<Cover>, std::collections::HashSet<PathBuf>);
    let chunk_digests: Vec<Chunk> = s
        .probes
        .as_slice()
        .par_chunks(512)
        .map(|chunk| {
            let mut h = Hasher::new("probes");
            let mut cov: Vec<Cover> = Vec::new();
            let mut walked: std::collections::HashSet<PathBuf> = Default::default();
            let mut paths = Vec::new();
            // The paths a record probed, then the directories those probes were conditional
            // on (`expand_guards`: one that appears changes what a trace records), then
            // where a walk-up went on (`expand_above`: found there, or not).
            for p in chunk {
                for (tag, expand) in crate::normalize::PRESENCE {
                    paths.clear();
                    expand(p, &mut paths);
                    for rel in &paths {
                        let a = abs(&root, rel);
                        if tag == crate::normalize::ABOVE && !a.starts_with(&cx.repo) {
                            continue;
                        }
                        if let Some(kind) = crate::normalize::presence(fs, &a) {
                            h.str(tag);
                            h.str(rel);
                            h.str(&kind);
                        }
                        match cx.cover(&a) {
                            Cover::Walked(d) => {
                                walked.insert(d);
                            }
                            c => {
                                if !cov.contains(&c) {
                                    cov.push(c);
                                }
                            }
                        }
                    }
                }
            }
            (h.0.digest128(), cov, walked)
        })
        .collect();
    for (d, cov, w) in chunk_digests {
        h.u128(d);
        walked.extend(w);
        for c in cov {
            add(c, &mut covers);
        }
    }
    // The directory each probed path lies in: there or not. A lookup that stops at
    // a missing directory records the directory; once it is there, the lookup
    // goes on and records paths inside it, none of which exists. Two shards
    // that differ in exactly that had the same key (no probe of either is
    // present), and the store kept the first one's bytes under it.
    for rel in crate::normalize::probe_dirs(s.probes.as_slice()) {
        let a = abs(&root, &rel);
        if fs.is_dir(&a) {
            h.str("probe-dir-present");
            h.str(&rel);
        } else {
            // Not there: from which directory on (`DIR_ABSENT`).
            h.str(crate::normalize::DIR_ABSENT);
            h.str(&rel);
            h.str(&crate::normalize::nearest_dir_on(fs, &root, &rel));
        }
        // The repository's root is there for every build of it; what would cover it is
        // its parent directory, which is anyone's.
        if a == cx.repo {
            continue;
        }
        match cx.cover(&a) {
            Cover::Walked(d) => {
                walked.insert(d);
            }
            c => add(c, &mut covers),
        }
    }
    let tp1 = Instant::now();
    // Membership listings (one digest per directory and pattern per build: the same
    // membership, such as a jest project's `**/__mocks__/**`, appears in many
    // shards).
    let mut listings: Vec<(PathBuf, String)> = Vec::new();
    // A listing that only membership hubs reference is not listed: the shard stores
    // the hubs' states of it (below), nothing else
    // ([`crate::shard::Shard::hub_members`]).
    let hub_only =
        |i: usize| tasks.is_some() && s.hub_members.binary_search(&(i as u32).into()).is_ok();
    // A listing only files' key lists reference is not part of the tree part at
    // all: the key takes its digest directly
    // ([`crate::shard::Shard::key_only`], `obs_digest_keyed`).
    let key_only = |i: usize| s.key_only.binary_search(&(i as u32).into()).is_ok();
    let member_digests: Vec<Option<(u128, ListCover)>> = s
        .memberships
        .as_slice()
        .par_iter()
        .enumerate()
        .map(|(i, m)| {
            (!hub_only(i) && !key_only(i))
                .then(|| cx.membership_digest(&abs(&root, m.dir.as_str()), m.pattern.as_str()))
        })
        .collect();
    for (i, (m, dc)) in s.memberships.iter().zip(member_digests).enumerate() {
        if key_only(i) {
            continue;
        }
        // The walked directories the listing reads: the directory (and with a deep
        // pattern its whole walked subtree), or for a missing directory its
        // walked cover.
        let base = abs(&root, m.dir.as_str());
        if cx.walked_dirs.contains(&base) {
            let p = m.pattern.as_str();
            if p.contains('/') || p.contains("**") {
                deep.push(base);
            } else {
                walked.insert(base);
            }
        } else if let Cover::Walked(w) = cx.cover(&base) {
            walked.insert(w);
        }
        h.str("member");
        h.str(m.dir.as_str());
        h.str(m.pattern.as_str());
        let Some((d, c)) = dc else {
            h.str("hub");
            continue;
        };
        h.u128(d);
        match c {
            ListCover::Dirs(dirs) => covers.extend(dirs),
            ListCover::Listing(_) => {
                listings.push((abs(&root, m.dir.as_str()), m.pattern.to_string()));
            }
            ListCover::No => add(Cover::No, &mut covers),
        }
    }
    // Membership hubs: what the shard keeps of each hub's member list
    // ([`crate::tasks::Tasks::hub_state`]). It is a function of the walked files
    // below the hub's directory, like the listing it stands for.
    if let Some(tasks) = tasks {
        for hub in s.hubs.iter() {
            let base = if hub.dir.is_empty() {
                cx.repo.clone()
            } else {
                cx.repo.join(hub.dir.as_str())
            };
            if cx.walked_dirs.contains(&base) {
                deep.push(base);
            } else {
                // A directory the walk does not enter (ignored, or not there): no walked
                // file is below it. Covered like any observation outside the walk, by its
                // nearest walked or existing ancestor.
                match cx.cover(&base) {
                    Cover::Walked(w) => {
                        walked.insert(w);
                    }
                    c => add(c, &mut covers),
                }
            }
            let exclude: Vec<String> = hub.exclude.iter().map(|e| e.to_string()).collect();
            h.str("hub");
            h.str(hub.dir.as_str());
            h.str(hub.pattern.as_str());
            for e in &exclude {
                h.str("exclude");
                h.str(e);
            }
            h.str(if hub.own { "own" } else { "all" });
            h.u128(tasks.hub_state(hub.dir.as_str(), hub.pattern.as_str(), &exclude, hub.own));
        }
    }
    listings.sort();
    listings.dedup();
    let tp2 = Instant::now();
    // Reached files: still present, still owned by the same package.
    let found_digests: Vec<Chunk> = s
        .obs
        .found
        .as_slice()
        .par_chunks(512)
        .map(|chunk| {
            let mut h = Hasher::new("found");
            let mut cov: Vec<Cover> = Vec::new();
            let mut walked: std::collections::HashSet<PathBuf> = Default::default();
            for f in chunk {
                let p = abs(&root, f.as_str());
                h.str("found");
                h.str(f.as_str());
                h.u128(found_digest(cx, &p));
                match cx.cover(&p) {
                    Cover::Walked(d) => {
                        walked.insert(d);
                    }
                    c => {
                        if !cov.contains(&c) {
                            cov.push(c);
                        }
                    }
                }
            }
            (h.0.digest128(), cov, walked)
        })
        .collect();
    for (d, cov, w) in found_digests {
        h.u128(d);
        walked.extend(w);
        for c in cov {
            add(c, &mut covers);
        }
    }
    // Places seen that led into the tree or nowhere ([`SEEN_LINK`], no package
    // name): where each leads now. Covered like a probe at the place, plus the
    // directory of every symbolic link crossed (by its mtime also where the
    // walk lists it: a link that leads elsewhere is the same entry of the same
    // kind) and the real path's.
    for l in s.obs.links.iter() {
        let Some(rest) = l.location.strip_prefix(SEEN_LINK) else {
            continue;
        };
        if !l.name.is_empty() {
            continue;
        }
        let loc = abs(&root, rest);
        let st = seen_state(cx, &loc);
        h.str("seen");
        h.str(rest);
        h.u128(st.digest);
        let mut cover = |c: Cover, covers: &mut std::collections::BTreeSet<PathBuf>| match c {
            Cover::Walked(d) => {
                walked.insert(d);
            }
            c => add(c, covers),
        };
        cover(cx.cover(&loc), &mut covers);
        for t in &st.would_lead {
            cover(cx.cover(t), &mut covers);
        }
        for d in &st.link_dirs {
            cover(Cover::Dir(d.clone()), &mut covers);
        }
        if let Some(real) = &st.real {
            cover(cx.cover(real), &mut covers);
        }
    }
    let tp3 = Instant::now();
    if turborepo_trace_core::timing::enabled() && (tp3 - tp0).as_millis() >= 100 {
        mark(&format!(
            "  tree part {:?}: probes {} ms ({}), memberships {} ms ({}), found {} ms ({}), \
             reusable {reusable}, {} covers",
            s.package.root.as_str(),
            (tp1 - tp0).as_millis(),
            s.probes.len(),
            (tp2 - tp1).as_millis(),
            s.memberships.len(),
            (tp3 - tp2).as_millis(),
            s.obs.found.len(),
            covers.len()
        ));
    }
    // Stamp the outside directories; one modified shortly before the build started
    // (or during it) cannot be trusted: the observations above may predate the
    // modification.
    let racy = cx.start_ns - 2_000_000_000;
    let covers_dbg: Vec<PathBuf> = if turborepo_trace_core::timing::enabled() {
        covers.iter().cloned().collect()
    } else {
        Vec::new()
    };
    let reuse = reusable
        .then(|| {
            covers
                .into_iter()
                .map(|d| dir_mtime(&d).filter(|m| *m < racy).map(|m| (d, m)))
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    if turborepo_trace_core::timing::enabled() && reuse.is_none() {
        mark(&format!(
            "  tree part {:?} not reusable (coverable {reusable}; racy: {:?})",
            s.package.root.as_str(),
            covers_dbg
                .iter()
                .filter(|d| dir_mtime(d).is_none_or(|m| m >= racy))
                .take(3)
                .collect::<Vec<_>>()
        ));
    }
    let rel = |p: &Path| -> Option<String> {
        p.strip_prefix(&cx.repo)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
    };
    let mut exact: Vec<u64> = walked
        .iter()
        .filter_map(|p| rel(p))
        .map(|r| dir_hash(&r))
        .collect();
    exact.sort_unstable();
    exact.dedup();
    let mut deep: Vec<String> = deep.iter().filter_map(|p| rel(p)).collect();
    deep.sort_unstable();
    deep.dedup();
    TreePart {
        digest: h.0.digest128(),
        reuse,
        listings,
        walked: WalkedDeps { exact, deep },
    }
}

/// The digest of the current state of everything `s` observed (see the module
/// docs).
pub(crate) fn obs_digest(cx: &Cx, s: &ArchivedShard) -> u128 {
    obs_digest_keyed(cx, s, None, None).0
}

/// [`obs_digest`], plus its tree part and whether that part only observed
/// walked directories. With the shard's `key` and an unchanged walked tree, the
/// tree part of an in-tree shard is taken from the previous build
/// ([`Cx::obs_prev`]): probes, memberships and reached files inside walked
/// directories can only change when a walked directory does. Config contents
/// and package links are always checked.
///
/// `tasks`: the build's task discovery, for a first-party shard (its membership
/// hubs are keyed by their state, [`crate::tasks::Tasks::hub_state`]); `None`
/// for a third-party shard, which has no tasks.
pub(crate) fn obs_digest_keyed(
    cx: &Cx,
    s: &ArchivedShard,
    key: Option<&str>,
    tasks: Option<&crate::tasks::Tasks<'_>>,
) -> (u128, TreePart) {
    let root = cx.pkg_root_abs(s.package.root.as_str());
    let cached = key
        .and_then(|k| Some((cx.tree_changed.as_ref()?, cx.obs_prev.get(k)?)))
        .filter(|(changed, t)| !changed.touches(&t.walked))
        .map(|(_, t)| t)
        .filter(|t| {
            t.reuse
                .as_ref()
                .is_some_and(|dirs| dirs.iter().all(|(d, m)| cx.dir_mtime(d) == Some(*m)))
                && t.listings.iter().all(|(d, p)| cx.listing_valid(d, p))
        })
        .cloned();
    let tree = match cached {
        Some(t) => {
            cx.tree_stats[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            t
        }
        None => {
            cx.tree_stats[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tree_part(cx, s, &root, tasks)
        }
    };
    let mut h = Hasher::new("obs");
    h.u128(tree.digest);
    // Config contents; path and existence only for path-only inputs.
    let path_only = |i: usize| s.path_configs.binary_search(&(i as u32).into()).is_ok();
    let view = |c: &str| crate::normalize::view_config(c).map(|p| abs(&root, p));
    let configs: Vec<PathBuf> = s
        .configs
        .iter()
        .enumerate()
        .filter(|(i, c)| !path_only(*i) && view(c.as_str()).is_none())
        .map(|(_, c)| abs(&root, c.as_str()))
        .collect();
    let mut hashes = cx.hashes.hash_many(&configs).into_iter();
    for (i, c) in s.configs.iter().enumerate() {
        // A manifest read through its resolution view: the view's bytes.
        if let Some(p) = view(c.as_str()) {
            h.str("config-view");
            h.str(c.as_str());
            match view_digest(cx, c.as_str(), &p) {
                Some(x) => h.u128(x),
                None => h.str("missing"),
            }
            continue;
        }
        if path_only(i) {
            h.str("config-path");
            h.str(c.as_str());
            let p = abs(&root, c.as_str());
            let is_file = match cx.is_file_memo.get(&p) {
                Some(v) => *v,
                None => {
                    let v = cx.fs().is_file(&p);
                    cx.is_file_memo.insert(p, v);
                    v
                }
            };
            h.str(if is_file { "file" } else { "missing" });
            continue;
        }
        h.str("config");
        h.str(c.as_str());
        match hashes.next().flatten() {
            Some(x) => h.u128(x),
            None => h.str("missing"),
        }
    }
    // Whether the checkout lies below `node_modules`, for a shard with a file whose
    // governing tsconfig has aliases ([`crate::fw::place_state`]).
    if let Some(below) = crate::fw::place_state(cx, s) {
        h.str("aliased");
        h.str(crate::fw::place_word(below));
    }
    // The place of the checkout as an input of a project's test discovery: the
    // state its stored discovery has at the root of this build
    // ([`crate::located`]). Empty at every place for an ordinary project.
    for l in s.located.iter() {
        h.str("located");
        h.str(l.config.as_str());
        h.str(l.spec.as_str());
        h.str(&cx.places.state(l.spec.as_str()));
    }
    // Dependency closures resolved by `^task`: the closure's package roots now.
    for r in s.dep_closures.iter() {
        h.str("closure");
        h.str(r.as_str());
        let d = match cx.closure_memo.get(r.as_str()) {
            Some(d) => *d,
            None => {
                let mut c = Hasher::new("closure");
                match cx.task_graph.closure(r.as_str()) {
                    Some(ix) => {
                        for i in ix {
                            c.str(&cx.task_graph.ws.packages[i].rel);
                        }
                    }
                    None => c.str("absent"),
                }
                let d = c.0.digest128();
                cx.closure_memo.insert(r.to_string(), d);
                d
            }
        };
        h.u128(d);
    }
    // Third-party links: the package found at each location now, or for links
    // resolved from the lockfile alone, what the lockfile resolves them to now
    // (not part of the tree part, so a lockfile change keeps every tree part
    // reusable).
    for l in s.obs.links.iter() {
        if let Some(rest) = l.location.strip_prefix(LOCK_LINK) {
            h.str("lock-link");
            h.str(l.location.as_str());
            let (importer, name) = rest.split_once(':').unwrap_or((rest, ""));
            match cx.ids.lock_resolve(importer, name) {
                Some(crate::identity::LockTarget::Package(id)) => {
                    h.str(&id.name);
                    h.str(&id.version);
                    h.opt(id.integrity.as_deref());
                }
                Some(crate::identity::LockTarget::Workspace(r)) => h.str(&r),
                None => h.str("absent"),
            }
            continue;
        }
        if let Some(rest) = l.location.strip_prefix(SEEN_LINK) {
            // A place that led into the tree or nowhere is in the tree part. One that led
            // into an installed package is checked here like a package link: the package's
            // identity is its manifest's contents, which no directory's mtime covers.
            if !l.name.is_empty() {
                h.str("seen-link");
                h.str(l.location.as_str());
                h.u128(seen_digest(cx, &abs(&root, rest)));
            }
            continue;
        }
        h.str("link");
        h.str(l.location.as_str());
        h.u128(link_digest(cx, &abs(&root, l.location.as_str())));
    }
    // Listings that only files' key lists reference: one digest per directory and
    // pattern per build, taken from the previous build while the listed tree is
    // unchanged ([`Cx::membership_digest`]).
    for i in s.key_only.iter() {
        let m = &s.memberships[i.to_native() as usize];
        h.str("key-member");
        h.str(m.dir.as_str());
        h.str(m.pattern.as_str());
        h.u128(
            cx.membership_digest(&abs(&root, m.dir.as_str()), m.pattern.as_str())
                .0,
        );
    }
    (h.0.digest128(), tree)
}

/// The digest of everything a trace takes from a reached file `p`: whether it
/// is there, and what target classification reads of it
/// ([`TargetCx::classify`]): whether a language's or an installed package owns
/// it, which workspace package it lies in and at which root (an edge names its
/// target by the package and the path below the package's root). A data file
/// that a vendored module comes to contain, or a package that moves and
/// keeps its name, is another target for a new trace. So is a file that comes
/// to be a link to a file: a resolution finds what the link leads to, and the
/// link as one it crossed (found by the random edits that make a file a link,
/// [`crate::normalize::presence`]).
pub(crate) fn found_digest(cx: &Cx, p: &Path) -> u128 {
    use turborepo_trace_core::fs::FileKind;
    let fs = cx.fs();
    let mut h = Hasher::new("found-state");
    h.str(match fs.stat(p).filter(|st| st.kind == FileKind::File) {
        Some(st) if st.is_symlink => "link",
        Some(_) => "file",
        None => "absent",
    });
    h.str(if cx.ids.language(p, fs).is_some() {
        "language package"
    } else if cx.ids.is_third_party(p) && cx.ids.package_root(p).is_some() {
        "installed package"
    } else {
        "first party"
    });
    if let Ok(rel) = p.strip_prefix(&cx.repo) {
        let def = &cx.ws.packages[cx.ws.owner(&rel.to_string_lossy())];
        h.str(&def.name);
        h.str(&def.root);
    }
    h.0.digest128()
}

/// Where a place that was seen ([`SEEN_LINK`]) leads now, and what decides it.
#[derive(Debug)]
pub(crate) struct Seen {
    /// The digest of where it leads: nowhere, or its real path, with the
    /// identity of the installed package the path lies in.
    pub digest: u128,
    /// The name of that installed package (`-` for one without a name).
    pub package: Option<String>,
    /// The directory of every symbolic link crossed on the way.
    pub link_dirs: Vec<PathBuf>,
    /// The real path.
    pub real: Option<PathBuf>,
    /// For a place that is a symbolic link leading nowhere: where it points
    /// (and where that points, for a chain). What it would lead to appears
    /// there, and no directory on the way to the link changes when it does.
    pub would_lead: Vec<PathBuf>,
}

/// [`Seen`] of the place `loc`, memoized for the build.
pub(crate) fn seen_state(cx: &Cx, loc: &Path) -> Arc<Seen> {
    if let Some(s) = cx.seen_memo.get(loc) {
        return Arc::clone(&s);
    }
    let fs = cx.fs();
    let mut h = Hasher::new("seen");
    let mut seen = Seen {
        digest: 0,
        package: None,
        link_dirs: Vec::new(),
        real: None,
        would_lead: Vec::new(),
    };
    match fs.stat(loc) {
        None => {
            h.str("absent");
            let mut at = loc.to_path_buf();
            for _ in 0..8 {
                let (Ok(target), Some(dir)) = (std::fs::read_link(&at), at.parent()) else {
                    break;
                };
                // The link's directory, by its mtime: a link that comes to lead elsewhere
                // is the same entry of the same kind in a listing of it.
                seen.link_dirs.push(dir.to_path_buf());
                at = abs(dir, &target.to_string_lossy());
                seen.would_lead.push(at.clone());
            }
            // Where it would lead is part of what was seen: a lookup goes on there, and a
            // link that comes to lead to another place where nothing is, is followed to
            // that place by a new trace. (A config's walk that listed the link's directory
            // used to leave a record of the link by accident, while it led nowhere:
            // `turborepo_trace_frameworks::fsx`, `entries`.)
            for place in &seen.would_lead {
                h.str(&rel_to(&cx.repo, &cx.repo, place));
            }
        }
        Some(_) => {
            let rp = fs.canonicalize(loc);
            let real = rp.path;
            seen.link_dirs = rp
                .links
                .iter()
                .filter_map(|(link, _)| link.parent().map(Path::to_path_buf))
                .collect();
            h.str("at");
            h.str(&rel_to(&cx.repo, &cx.repo, &real));
            // An installed package: observations inside it are not recorded (its identity
            // stands for its files), so the identity is part of what was seen. A workspace
            // package's files are recorded where they are.
            if cx.ids.is_third_party(&real.join("x"))
                && let Some((id, _)) = link_identity(cx, loc)
            {
                h.str(&id.name);
                h.str(&id.version);
                h.opt(id.integrity.as_deref());
                seen.package = Some(if id.name.is_empty() {
                    "-".to_owned()
                } else {
                    id.name
                });
            }
            seen.real = Some(real);
        }
    }
    seen.digest = h.0.digest128();
    let seen = Arc::new(seen);
    cx.seen_memo.insert(loc.to_path_buf(), Arc::clone(&seen));
    seen
}

/// The digest of where the place `loc` (a link or a directory a lookup went
/// through, [`SEEN_LINK`]) leads now: nowhere, or its real path.
pub(crate) fn seen_digest(cx: &Cx, loc: &Path) -> u128 {
    seen_state(cx, loc).digest
}

/// The digest of the view of the manifest at `abs(root, path of c)` now that
/// the config input `c` stands for: the resolution view
/// ([`turborepo_trace_core::DirCache::read_manifest`]) or the loader's
/// ([`turborepo_trace_core::DirCache::read_manifest_loader`]; of the file as it
/// is when it has no view), `None` when there is no such file. Memoized for the
/// build.
pub(crate) fn view_digest(cx: &Cx, c: &str, p: &Path) -> Option<u128> {
    let loader = crate::normalize::loader_config(c).is_some();
    let memo = if loader {
        &cx.loader_view_memo
    } else {
        &cx.view_memo
    };
    if let Some(d) = memo.get(p) {
        return *d;
    }
    let bytes = if loader {
        cx.fs().read_manifest_loader(p)
    } else {
        cx.fs().read_manifest(p)
    };
    let d = bytes.ok().map(|b| xxhash_rust::xxh3::xxh3_128(&b));
    memo.insert(p.to_path_buf(), d);
    d
}

/// The digest of the package identity found at the link location `loc` now
/// (memoized for the build).
pub(crate) fn link_digest(cx: &Cx, loc: &Path) -> u128 {
    if let Some(d) = cx.link_memo.get(loc) {
        return *d;
    }
    let mut lh = Hasher::new("link-id");
    if let Some(li) = cx.ids.language(loc, cx.fs()) {
        lh.str("language");
        lh.str(&li.id.name);
        lh.str(&li.id.version);
        lh.opt(li.id.integrity.as_deref());
        lh.str(&rel_to(&cx.repo, &cx.repo, &li.root));
    } else {
        match link_identity(cx, loc) {
            Some((id, real)) => {
                lh.str(&id.name);
                lh.str(&id.version);
                lh.opt(id.integrity.as_deref());
                lh.str(&rel_to(&cx.repo, &cx.repo, &real));
            }
            None => lh.str("absent"),
        }
    }
    let d = lh.0.digest128();
    cx.link_memo.insert(loc.to_path_buf(), d);
    d
}

/// The digest of what the lockfile resolves the lock link `rest`
/// (`<importer>:<name>`, a link location without its [`LOCK_LINK`] prefix) to
/// now.
pub(crate) fn lock_link_digest(cx: &Cx, rest: &str) -> u128 {
    let mut h = Hasher::new("lock-link");
    let (importer, name) = rest.split_once(':').unwrap_or((rest, ""));
    match cx.ids.lock_resolve(importer, name) {
        Some(crate::identity::LockTarget::Package(id)) => {
            h.str(&id.name);
            h.str(&id.version);
            h.opt(id.integrity.as_deref());
        }
        Some(crate::identity::LockTarget::Workspace(r)) => h.str(&r),
        None => h.str("absent"),
    }
    h.0.digest128()
}

/// The repo-relative scope of a covered uncertainty. A scope outside the
/// repository (or a scope-less record) is `None`: the record stays open, which
/// is always sound.
fn scope_rec(repo: &Path, u: &Uncertainty) -> Option<crate::shard::ScopeRec> {
    let s = u.scope.as_ref()?;
    let dir = rel_to(repo, repo, &s.dir);
    (!dir.starts_with('/') && !dir.starts_with("..")).then(|| crate::shard::ScopeRec {
        dir,
        pattern: s.pattern.to_string(),
    })
}

/// The DefinitelyTyped package of an npm package name: `retry` →
/// `@types/retry`, `@scope/pkg` → `@types/scope__pkg`.
fn types_package(name: &str) -> String {
    match name.strip_prefix('@').and_then(|n| n.split_once('/')) {
        Some((scope, pkg)) => format!("@types/{scope}__{pkg}"),
        None => format!("@types/{name}"),
    }
}

/// Link location prefix for dependencies resolved from the lockfile alone:
/// `lockfile:<importer>:<name>`.
pub(crate) const LOCK_LINK: &str = "lockfile:";

/// Link location prefix for a place a lookup went through without it being a
/// package an edge leads into: a symbolic link crossed on the way to a
/// first-party file, or the package directory of a file that was found and not
/// made an edge target ([`TargetCx::reach`]). `seen:<location>`; such a link
/// has no identity (it adds no package to the graph), its state is where the
/// place leads ([`seen_digest`]). Its name is empty, or the name of the
/// installed package the place led into: the first kind is part of the shard's
/// tree part (`tree_part`), the second is checked with the package
/// links (`obs_digest_keyed`).
pub(crate) const SEEN_LINK: &str = "seen:";

/// The identity of the package at a link location (a `node_modules/<name>`
/// path).
pub(crate) fn link_identity(
    cx: &Cx,
    location: &Path,
) -> Option<(crate::identity::Identity, PathBuf)> {
    let fs = cx.fs();
    if !fs.is_dir(location) {
        return None;
    }
    let real = fs.canonicalize(location).path;
    let name = cx
        .ids
        .package_root(&real.join("package.json"))
        .map(|(_, n)| n)
        .unwrap_or_default();
    Some((cx.ids.of_root(fs, &real, &name), real))
}

/// Files below `dir` matching `pattern`, relative to `dir`, sorted (directories
/// that match a pattern segment end in `/`). Symlinked directories and
/// `node_modules` are not entered. Deep patterns list subdirectories in
/// parallel. A listing that visits more than 200k entries is replaced by a
/// single truncation marker (deterministic regardless of order).
///
/// The graph directory is not part of the tree, whatever its name: `fs` hides
/// it ([`DirCache::hide`]; a build writes into it, so a listing that saw it
/// would differ after every build, and a refresh would never find the tree
/// unchanged).
pub fn membership_listing(fs: &DirCache, dir: &Path, pattern: &str) -> Vec<String> {
    membership_listing_in(fs, dir, pattern, None).0
}

/// [`membership_listing`], also returning the directories outside `walked` it
/// entered (`None` beyond [`MAX_LISTING_OUTSIDE_DIRS`]).
pub(crate) fn membership_listing_in(
    fs: &DirCache,
    dir: &Path,
    pattern: &str,
    walked: Option<&std::collections::HashSet<PathBuf>>,
) -> (Vec<String>, Option<Vec<PathBuf>>) {
    let cx = ListCx {
        walked,
        outside: std::sync::Mutex::new(Some(Vec::new())),
        fs,
        glob: Glob::new(pattern).ok(),
        deep: pattern.contains('/') || pattern.contains("**"),
        // A literal path-component fragment every match must contain (`__mocks__` in
        // `**/__mocks__/**`, `.snap` in `**/*.snap`): entries whose name and directory
        // both lack it are skipped without building their path or running the glob.
        lit: required_literal(pattern),
        visited: std::sync::atomic::AtomicUsize::new(0),
    };
    let mut out = cx.list(dir, "");
    let outside = cx
        .outside
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .map(|mut v| {
            v.sort();
            v
        });
    if cx.visited.load(std::sync::atomic::Ordering::Relaxed) > LISTING_BUDGET {
        return (vec!["\0truncated".to_owned()], outside);
    }
    out.sort_unstable();
    (out, outside)
}

const LISTING_BUDGET: usize = 200_000;

struct ListCx<'a> {
    walked: Option<&'a std::collections::HashSet<PathBuf>>,
    outside: std::sync::Mutex<Option<Vec<PathBuf>>>,
    fs: &'a DirCache,
    glob: Option<Glob>,
    deep: bool,
    lit: Option<&'a str>,
    visited: std::sync::atomic::AtomicUsize,
}

impl ListCx<'_> {
    fn list(&self, d: &Path, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Some(listing) = self.fs.list(d) else {
            return out;
        };
        let n = self
            .visited
            .fetch_add(listing.len(), std::sync::atomic::Ordering::Relaxed);
        if n > LISTING_BUDGET {
            return out;
        }
        let prefix_hit = self.lit.is_none_or(|l| prefix.contains(l));
        let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
        for e in listing.iter() {
            let name = e.name().to_string_lossy();
            let is_dir = matches!(e.kind(), turborepo_trace_core::fs::EntryKind::Dir);
            let descend =
                is_dir && self.deep && !matches!(&*name, "node_modules" | ".git" | ".turbotrace");
            let may_match = prefix_hit || self.lit.is_some_and(|l| name.contains(l));
            if !may_match && !descend {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{prefix}/{name}")
            };
            let matches = may_match && self.glob.as_ref().is_none_or(|g| g.matches(&rel));
            if is_dir {
                // A directory entry can itself satisfy a single-segment pattern.
                if matches {
                    out.push(format!("{rel}/"));
                }
                if descend {
                    let sub = d.join(e.name());
                    if self.walked.is_some_and(|w| !w.contains(&sub)) {
                        let mut o = self.outside.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(v) = o.as_mut() {
                            if v.len() >= MAX_LISTING_OUTSIDE_DIRS {
                                *o = None;
                            } else {
                                v.push(sub.clone());
                            }
                        }
                    }
                    subdirs.push((sub, rel));
                }
            } else if matches {
                out.push(rel);
            }
        }
        if subdirs.len() > 1 {
            let nested: Vec<Vec<String>> = subdirs
                .par_iter()
                .map(|(p, rel)| self.list(p, rel))
                .collect();
            out.extend(nested.into_iter().flatten());
        } else if let Some((p, rel)) = subdirs.first() {
            out.extend(self.list(p, rel));
        }
        out
    }
}

/// The longest wildcard-free fragment of one path component of `pattern`, if
/// any. Patterns with alternations, classes, negations or escapes have none (a
/// fragment there is not required).
pub(crate) fn required_literal(pattern: &str) -> Option<&str> {
    if pattern.contains(['{', '}', '[', ']', '!', '\\', '(', ')', '+', '@']) {
        return None;
    }
    pattern
        .split('/')
        .flat_map(|seg| seg.split(['*', '?']))
        .filter(|s| !s.is_empty())
        .max_by_key(|s| s.len())
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    #[test]
    fn task_pseudo_paths() {
        // A membership hub named after a `.dockerignore` exclude, a task, a part.
        for p in [
            "/r/services/svc-a/#members:{services/svc-a/app,services/svc-a/app/**} \
             !{**/node_modules,**/node_modules/**}",
            "/r/#members:**",
            "/r/apps/web/#task:apps/web#build",
            "/r/apps/web/#docker:apps/web/Dockerfile#builder",
            "/r/packages/ui/#tsc:packages/ui/tsconfig.json",
        ] {
            assert!(is_task_pseudo_path(Path::new(p)), "{p}");
        }
        // Real files, also below node_modules and with a `#` in a name.
        for p in [
            "/r/node_modules/pkg/index.js",
            "/r/src/#private/x.js",
            "/r/docs/c#/notes.md",
            "/r/src/#",
        ] {
            assert!(!is_task_pseudo_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn literals() {
        assert_eq!(required_literal("**/__mocks__/**"), Some("__mocks__"));
        assert_eq!(required_literal("**/*.snap"), Some(".snap"));
        assert_eq!(required_literal("*.{js,ts}"), None);
        assert_eq!(required_literal("**"), None);
        assert_eq!(required_literal("a.test.ts.snap"), Some("a.test.ts.snap"));
    }

    #[test]
    fn listing_with_prefilter() {
        let d = tempfile::tempdir().unwrap();
        for f in [
            "a/__mocks__/x.js",
            "a/b/__mocks__/y.js",
            "a/b/z.js",
            "__mocks__/r.js",
            "c.snap",
            "a/d.snap",
        ] {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let fs = DirCache::new();
        assert_eq!(
            membership_listing(&fs, d.path(), "**/__mocks__/**"),
            ["__mocks__/r.js", "a/__mocks__/x.js", "a/b/__mocks__/y.js"]
        );
        assert_eq!(
            membership_listing(&fs, d.path(), "**/*.snap"),
            ["a/d.snap", "c.snap"]
        );
        assert_eq!(membership_listing(&fs, d.path(), "*.snap"), ["c.snap"]);
    }
}

/// One first-party package's state during a build.
struct PkgState {
    def: PackageDef,
    /// Entries: repo-relative paths.
    entries: Vec<String>,
    old: Option<(String, LoadedShard)>,
    /// Content hashes of the entries, when computed before tracing.
    hashes: Option<Vec<Option<u128>>>,
    manifest_hash: Option<u128>,
}

fn first_party_key(
    cx: &Cx,
    def: &PackageDef,
    manifest_hash: Option<u128>,
    entries: &[(String, Option<u128>)],
    obs: u128,
) -> String {
    let mut h = Hasher::new("first-party");
    h.str(&cx.opts_digest);
    h.u128(cx.fw_configs.digest_for(&def.root));
    h.str(&def.name);
    h.str(&def.root);
    h.u128(manifest_hash.unwrap_or(0));
    for (p, x) in entries {
        h.str(p);
        h.u128(x.unwrap_or(u128::MAX));
    }
    h.u128(obs);
    h.hex()
}

fn opaque_key(cx: &Cx, name: &str, version: &str, integrity: Option<&str>) -> String {
    let mut h = Hasher::new("opaque");
    h.str(&cx.opts_digest);
    h.str(name);
    h.str(version);
    h.opt(integrity);
    h.hex()
}

pub(crate) fn third_party_key(cx: &Cx, p: &PackageRec, obs: u128) -> String {
    let mut h = Hasher::new("third-party");
    h.str(&cx.opts_digest);
    h.str(&p.name);
    h.opt(p.version.as_deref());
    h.opt(p.integrity.as_deref());
    h.str(&p.root);
    h.u128(obs);
    h.hex()
}

/// Runs a build (see the module docs).
// Invariant: every package of a batch was inserted into `per_pkg` above; it decodes what this
// function just encoded.
#[allow(clippy::expect_used)]
pub fn build(opts: &GraphOptions, langs: &[Arc<dyn Language>]) -> Result<BuildReport, Error> {
    let t0 = Instant::now();
    let mut report = BuildReport::default();
    let repo = std::fs::canonicalize(&opts.root).map_err(|e| Error::io(&opts.root, e))?;
    let repo = turborepo_trace_core::fs::normalize(&repo);
    let out = opts.out_dir(&repo);
    let store = out.join("store");
    std::fs::create_dir_all(&store).map_err(|e| Error::io(&store, e))?;
    // The graph directory by its real path, as the walk and the tracer meet it
    // below the (canonical) root: they leave it out by path.
    let out = std::fs::canonicalize(&out)
        .map(|p| turborepo_trace_core::fs::normalize(&p))
        .unwrap_or(out);
    let store = out.join("store");
    let graph_in_tree = (out != repo && out.starts_with(&repo)).then(|| out.clone());
    // The cache writes (obs cache) that the previous build of this graph left
    // running land before this build reads them.
    crate::wait_for_cache_writes_in(&out);

    // 1. Walk and discover.
    let start_ns = now_ns();
    mark("build: start");
    let walk = crate::walk::walk_cached(&repo, &out, opts.threads);
    mark("build: walk");
    report.walk_cached = walk.unchanged.is_some();
    report.walked_links = walk.links;
    let files = Arc::new(walk.files);
    let walked_dirs: std::collections::HashSet<PathBuf> = walk
        .dirs
        .iter()
        .map(|d| {
            if d.is_empty() {
                repo.clone()
            } else {
                repo.join(d)
            }
        })
        .collect();
    let walk_state = (walk.unchanged, walk.prev, walk.changed);
    mark("build: walked dir set");
    report.files = files.len();
    // Independent setup steps in parallel: workspace discovery, lockfiles, the hash
    // cache and the tracer.
    let make_tracer = || tracer_for(&repo, opts, langs, graph_in_tree.as_deref());
    // The task workspace (every manifest read again) right after discovery,
    // overlapping the tracer and the hash cache.
    let manifests = crate::smallfiles::SmallFiles::open(&crate::cache_dir(&out).join("manifests"));
    let (((ws, task_graph), (lock_loader, locks, lock_hashes)), (hashes, tracer)) = rayon::join(
        || {
            rayon::join(
                || {
                    static WS: Span = Span::new("setup: workspace discovery");
                    static TG: Span = Span::new("setup: task workspace");
                    let t = WS.enter();
                    let ws = Workspace::discover_with(&repo, &files, &manifests, &*opts.members.0);
                    drop(t);
                    let _t = TG.enter();
                    let tg = crate::tasks::TaskGraph::new(&repo, &ws, &manifests);
                    (ws, tg)
                },
                || {
                    static LOCK: Span = Span::new("setup: lockfiles");
                    let _t = LOCK.enter();
                    load_lockfiles_deferred(&repo, &files)
                },
            )
        },
        || {
            rayon::join(
                || {
                    static HASHES: Span = Span::new("setup: hash cache open");
                    let _t = HASHES.enter();
                    HashCache::open(&crate::cache_dir(&out).join("hashes"))
                },
                || {
                    static TRACER: Span = Span::new("setup: tracer");
                    let _t = TRACER.enter();
                    make_tracer()
                },
            )
        },
    );
    mark("build: setup (workspace, lockfiles, hashes, tracer)");
    let tracer = Arc::new(tracer.map_err(Error::Trace)?);
    // With a changed tree, task analysis of dirty packages will resolve npm
    // binaries: build the workspace bin index (a workspace discovery) in the
    // background meanwhile instead of stalling every resolving task in the
    // assemble phase. Same shared dir cache, so the index is the one lookups
    // use.
    if walk_state.0.is_none() {
        let fs = Arc::clone(tracer.fs_arc());
        let root = repo.clone();
        let _ = std::thread::Builder::new()
            .name("turbotrace-binindex".into())
            .spawn(move || {
                turborepo_trace_frameworks::fsx::with_fs(&fs, || {
                    turborepo_trace_tasks::recognizers::bins::npm::prewarm_index(&root)
                })
            });
    }
    let env_roots: Vec<PathBuf> = tracer
        .environment_roots()
        .iter()
        .map(|r| r.path.clone())
        .filter(|p| !p.starts_with(&repo))
        .collect();
    let (obs_prev, mut listings_prev, tree_changed, old_digests) = read_obs_cache(
        &out,
        obs_token(&ws),
        (walk_state.0, walk_state.1, &walk_state.2),
    );
    let tree_changed = tree_changed.map(|c| {
        c.refine(&old_digests, |d| {
            let p = if d.is_empty() {
                repo.clone()
            } else {
                repo.join(d)
            };
            tracer.fs().list(&p).map(|l| listing_digest(&l))
        })
    });
    if let Some(c) = &tree_changed {
        listings_prev.retain_unchanged(&repo, c);
    }
    listings_prev.warm();
    if turborepo_trace_core::debug::var_os("DEBUG_WALK").is_some() {
        eprintln!(
            "obs cache: {} tree parts, changed walked dirs: {:?}",
            obs_prev.len(),
            tree_changed
                .as_ref()
                .map(|c| &c.dirs[..c.dirs.len().min(8)])
        );
    }
    mark("build: obs cache");
    // The store's shards: the pack and its log (a pack in the pre-log location is
    // gone).
    let pack = shard::Pack::open(&store);
    let _ = std::fs::remove_file(crate::cache_dir(&out).join("pack"));
    let cx = Cx {
        task_graph,
        opts_digest: options_digest(opts, &tracer),
        ids: Identities::with_lazy_lock(vec![PackageBoundary::NODE_MODULES], lock_loader)
            .with_repo(&repo)
            .with_locks(locks)
            .with_provider(Arc::clone(&tracer) as Arc<dyn crate::identity::IdentityProvider>),
        hashes,
        repo: repo.clone(),
        tracer,
        env_roots,
        ws,
        store: store.clone(),
        fw_configs: crate::fw::ConfigPaths::new(&repo, &files),
        members: Default::default(),
        walked_dirs,
        tree_index: Default::default(),
        tree_changed,
        obs_prev,
        obs_new: Default::default(),
        start_ns,
        tree_stats: Default::default(),
        fresh_listings: Default::default(),
        link_memo: Default::default(),
        seen_memo: Default::default(),
        view_memo: Default::default(),
        loader_view_memo: Default::default(),
        is_file_memo: Default::default(),
        closure_memo: Default::default(),
        places: crate::located::Places::new(&repo, Arc::clone(&files)),
        dir_mtimes: Default::default(),
        listings_prev,
        sink: std::sync::Mutex::new(ShardSink {
            log_end: pack.as_ref().map(|p| p.log_end()),
            ..Default::default()
        }),
        pack,
    };
    // A changed tree recomputes deep listings: index it once for all of them.
    if walk_state.0.is_none() && turborepo_trace_core::debug::var_os("NO_TREE_INDEX").is_none() {
        cx.tree_index.start(
            Arc::clone(cx.tracer.fs_arc()),
            repo.clone(),
            cx.walked_dirs.clone(),
        );
    }
    mark("build: cx");
    let fw = crate::fw::Fw::new(&cx.tracer, &repo, &files, &cx.places);
    // Hub members that `TargetCx::classify` (first-party, not following) links to a
    // third-party package rather than by path keep stored edges (`HubRec::skip`).
    let hub_third_party = |p: &Path| {
        cx.ids.language(p, cx.fs()).is_some()
            || (cx.ids.is_third_party(p) && cx.ids.package_root(p).is_some())
    };
    let tasks =
        crate::tasks::Tasks::new(&cx.tracer, &repo, &files, &cx.task_graph, &hub_third_party);
    mark("build: fw + tasks");
    let mut entries: Vec<Vec<String>> = vec![Vec::new(); cx.ws.packages.len()];
    // Classified in parallel (tens of thousands of files), distributed in walk
    // order.
    let owned: Vec<(usize, &String)> = files
        .par_iter()
        .filter(|f| {
            cx.tracer.languages().by_extension(Path::new(f)).is_some()
                && !is_vendored_third_party(&cx, f)
        })
        .map(|f| (cx.ws.owner(f), f))
        .collect();
    for (o, f) in owned {
        entries[o].push(f.clone());
    }
    let old_any = Manifest::read(&out);
    let prev_files = old_any.as_ref().and_then(|m| m.files.clone());
    let old = old_any.filter(|m| m.options_digest == cx.opts_digest);
    let old_first: HashMap<&str, &str> = old
        .iter()
        .flat_map(|m| &m.shards)
        .filter(|s| s.kind == "first-party")
        .map(|s| (s.root.as_str(), s.key.as_str()))
        .collect();
    report.timings_ms[0] = t0.elapsed().as_millis() as u64;
    mark("build: entries + old manifest");

    // 2. Validate previous shards.
    let t1 = Instant::now();
    let pack = cx.pack.as_ref().filter(|_| old.is_some());
    // Old shards are mapped; checksums are checked in parallel.
    let mut pkgs: Vec<PkgState> = cx
        .ws
        .packages
        .par_iter()
        .cloned()
        .zip(entries)
        .map(|(def, entries)| PkgState {
            old: old_first.get(def.root.as_str()).and_then(|k| {
                shard::load_from(&store, pack, k)
                    .ok()
                    .map(|s| ((*k).to_owned(), s))
            }),
            def,
            entries,
            hashes: None,
            manifest_hash: None,
        })
        .collect();
    let slow = std::sync::Mutex::new(Vec::new());
    mark("build: old shards");
    let reuse: Vec<Option<String>> = pkgs
        .par_iter_mut()
        .map(|p| {
            p.manifest_hash = p
                .def
                .manifest
                .as_ref()
                .and_then(|m| cx.hashes.hash(&repo.join(m)).ok());
            let (key, old) = p.old.as_ref()?;
            static HASH: Span = Span::new("validate: entry hashes of a package (stat)");
            static OBS: Span = Span::new("validate: observation digest of a package");
            let tv = Instant::now();
            let th = HASH.enter();
            let paths: Vec<PathBuf> = p.entries.iter().map(|e| repo.join(e)).collect();
            let hashes = cx.hashes.hash_many(&paths);
            drop(th);
            let t_hash = tv.elapsed();
            let _to = OBS.enter();
            let list: Vec<(String, Option<u128>)> = p
                .entries
                .iter()
                .map(|e| rel_in(&p.def.root, e))
                .zip(hashes.iter().copied())
                .collect();
            p.hashes = Some(hashes);
            let (obs, tree) = obs_digest_keyed(&cx, old.get(), Some(key), Some(&tasks));
            if turborepo_trace_core::timing::enabled() {
                slow.lock().unwrap_or_else(|e| e.into_inner()).push((
                    tv.elapsed(),
                    t_hash,
                    p.def.root.clone(),
                    p.entries.len(),
                ));
            }
            let k = first_party_key(&cx, &p.def, p.manifest_hash, &list, obs);
            if crate::keydump::enabled() {
                crate::keydump::dump(
                    &cx,
                    &tasks,
                    "validate",
                    &p.def,
                    p.manifest_hash,
                    &list,
                    old.get(),
                    &k,
                    None,
                );
            }
            (k == *key).then(|| {
                cx.obs_new
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.clone(), tree);
                key.clone()
            })
        })
        .collect();
    report.timings_ms[1] = t1.elapsed().as_millis() as u64;
    if turborepo_trace_core::timing::enabled() {
        let l = |i: usize| {
            (
                LISTING_N[i].load(std::sync::atomic::Ordering::Relaxed),
                LISTING_US[i].load(std::sync::atomic::Ordering::Relaxed) / 1000,
            )
        };
        mark(&format!(
            "build: validate (fresh listings: shallow {:?}, deep {:?} (count, thread-ms))",
            l(0),
            l(1)
        ));
    }
    mark("build: validate");
    {
        let mut v = slow.into_inner().unwrap_or_else(|e| e.into_inner());
        v.sort_by_key(|b| std::cmp::Reverse(b.0));
        for (t, th, root, n) in v.iter().take(6) {
            mark(&format!(
                "  validate {root:?}: {} ms (hashes {} ms, {n} entries)",
                t.as_millis(),
                th.as_millis()
            ));
        }
    }
    report.tree_reused = [
        cx.tree_stats[0].load(std::sync::atomic::Ordering::Relaxed),
        cx.tree_stats[1].load(std::sync::atomic::Ordering::Relaxed),
    ];
    if turborepo_trace_core::debug::var_os("DEBUG_WALK").is_some() {
        let mut why: HashMap<String, usize> = HashMap::new();
        for t in cx
            .obs_new
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            match &t.reuse {
                None => *why.entry("not reusable".into()).or_default() += 1,
                Some(d) => {
                    for (p, _) in d {
                        *why.entry(p.display().to_string()).or_default() += 1;
                    }
                }
            }
        }
        let mut v: Vec<_> = why.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        eprintln!(
            "tree parts: prev {} entries; reuse dirs: {:?}",
            cx.obs_prev.len(),
            &v[..v.len().min(8)]
        );
    }

    // 3 + 4. Analyze dirty packages, then assemble and write their shards: in
    // batches of packages (by file count) so that a cold build of a big
    // repository does not hold every file's analysis at once (a refresh is one
    // batch).
    let dirty: Vec<usize> = (0..pkgs.len()).filter(|&pi| reuse[pi].is_none()).collect();
    // A dirty package with a previous shard keeps the records of the files that
    // read nothing that changed ([`crate::filereuse`]): only the others are
    // analyzed.
    let plans: HashMap<usize, crate::filereuse::Plan> = {
        static PLAN: Span = Span::new("validate: files of a re-keyed package");
        dirty
            .par_iter()
            .filter_map(|&pi| {
                let _t = PLAN.enter();
                let p = &pkgs[pi];
                let (_, old) = p.old.as_ref()?;
                let rels: Vec<String> = p.entries.iter().map(|e| rel_in(&p.def.root, e)).collect();
                crate::filereuse::plan(&cx, &p.def, &rels, p.hashes.as_deref(), old.get())
                    .map(|plan| (pi, plan))
            })
            .collect()
    };
    report.kept_files = plans.values().map(|p| p.kept).sum();
    mark("build: per-file plans");
    let mut batches: Vec<Vec<usize>> = Vec::new();
    {
        let mut cur: Vec<usize> = Vec::new();
        let mut n = 0usize;
        for &pi in &dirty {
            let len = pkgs[pi].entries.len() - plans.get(&pi).map_or(0, |p| p.kept);
            if !cur.is_empty() && n + len > ANALYZE_BATCH_FILES {
                batches.push(std::mem::take(&mut cur));
                n = 0;
            }
            cur.push(pi);
            n += len;
        }
        if !cur.is_empty() {
            batches.push(cur);
        }
    }
    let mut t_trace = std::time::Duration::ZERO;
    let mut t_assemble = std::time::Duration::ZERO;
    let mut new_shards: HashMap<String, NewShard> = HashMap::new();
    for batch in &batches {
        let t2 = Instant::now();
        let mut inputs = Vec::new();
        let mut owners = Vec::new();
        for &pi in batch {
            let p = &pkgs[pi];
            let old_files: HashMap<&str, usize> = p
                .old
                .as_ref()
                .map(|(_, s)| {
                    s.get()
                        .files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| (f.path.as_str(), i))
                        .collect()
                })
                .unwrap_or_default();
            let plan = plans.get(&pi);
            for (ei, e) in p.entries.iter().enumerate() {
                if plan.is_some_and(|pl| pl.keep[ei].is_some()) {
                    continue;
                }
                let rel = rel_in(&p.def.root, e);
                let cached = (|| {
                    let (_, old) = p.old.as_ref()?;
                    let new_hash = p.hashes.as_ref()?[ei]?;
                    let f = &old.get().files[*old_files.get(rel.as_str())?];
                    let h = f.hash.as_ref()?;
                    if join([h[0].to_native(), h[1].to_native()]) != new_hash {
                        return None;
                    }
                    cached_extraction(f.extraction.as_ref()?, &cx.tracer)
                })();
                if cached.is_some() {
                    report.reused_extractions += 1;
                }
                inputs.push(AnalyzeInput {
                    path: repo.join(e),
                    cached,
                });
                owners.push((pi, ei));
            }
        }
        report.analyzed_files += inputs.len();
        let analyses = cx.tracer.analyze(inputs);
        t_trace += t2.elapsed();

        let t3 = Instant::now();
        let mut per_pkg: HashMap<usize, Vec<(usize, FileAnalysis)>> =
            batch.iter().map(|&pi| (pi, Vec::new())).collect();
        for ((pi, ei), a) in owners.into_iter().zip(analyses) {
            per_pkg.get_mut(&pi).expect("batch package").push((ei, a));
        }
        let work: Vec<(usize, Vec<(usize, FileAnalysis)>)> = batch
            .iter()
            .map(|pi| (*pi, per_pkg.remove(pi).unwrap_or_default()))
            .collect();
        let results: Vec<Result<(String, NewShard, bool), Error>> = work
            .into_par_iter()
            .map(|(pi, files)| {
                let p = &pkgs[pi];
                static PACKAGE: Span = Span::new("assemble: package, total");
                static CODEC: Span = Span::new("assemble: encode + decode the new shard");
                static OBS: Span = Span::new("assemble: observation digest of the new shard");
                static WRITE: Span = Span::new("assemble: write shard");
                let _tp = PACKAGE.enter();
                let ta = Instant::now();
                let mut shard = assemble_first_party(&cx, &fw, &tasks, p, files, plans.get(&pi));
                let ta = ta.elapsed();
                let tc = CODEC.enter();
                let encoded = shard::encode(&shard);
                // The shard's bytes before it is given its key (diagnostics).
                let content =
                    crate::keydump::enabled().then(|| xxhash_rust::xxh3::xxh3_128(&encoded));
                let loaded = shard::decode(&encoded, "new").expect("own encoding");
                drop(tc);
                // Task pseudo-files are not entries: the key covers them through observations.
                let list: Vec<(String, Option<u128>)> = shard
                    .files
                    .iter()
                    .filter(|f| !crate::tasks::is_pseudo_lang(f.lang.as_deref()))
                    .map(|f| (f.path.clone(), f.hash.map(join)))
                    .collect();
                let to = Instant::now();
                // The observation digest for the key, and next to it (one very large shard
                // must not wait for the two in turn) the state of everything the shard
                // observed, per table entry: what the next refresh compares to find which
                // files to trace again.
                let ((obs, tree), states) = rayon::join(
                    || {
                        let _t = OBS.enter();
                        obs_digest_keyed(&cx, loaded.get(), None, Some(&tasks))
                    },
                    || {
                        static STATES: Span =
                            Span::new("assemble: observation states of the new shard");
                        let _t = STATES.enter();
                        crate::filereuse::states(&cx, loaded.get()).rec()
                    },
                );
                shard.obs.states = states;
                if debug_assemble() {
                    eprintln!(
                        "assemble {:?}: shard {} ms, observations {} ms ({} probes, {} \
                         memberships, {} found)",
                        p.def.root,
                        ta.as_millis(),
                        to.elapsed().as_millis(),
                        shard.probes.len(),
                        shard.memberships.len(),
                        shard.obs.found.len()
                    );
                }
                let key = first_party_key(&cx, &p.def, p.manifest_hash, &list, obs);
                if crate::keydump::enabled() {
                    crate::keydump::dump(
                        &cx,
                        &tasks,
                        "assemble",
                        &p.def,
                        p.manifest_hash,
                        &list,
                        loaded.get(),
                        &key,
                        content,
                    );
                }
                cx.obs_new
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.clone(), tree);
                shard.key = key.clone();
                let tw = WRITE.enter();
                let written = write_shard(&cx, &shard)?;
                drop(tw);
                // Only the key and the package links are needed after this: the shard itself
                // (its files and edges) is freed as soon as it is staged.
                Ok((
                    p.def.root.clone(),
                    (
                        std::mem::take(&mut shard.key),
                        std::mem::take(&mut shard.obs.links),
                    ),
                    written,
                ))
            })
            .collect();
        for r in results {
            let (root, s, written) = r?;
            report.written += written as usize;
            report.traced += 1;
            new_shards.insert(root, s);
        }
        t_assemble += t3.elapsed();
    }
    report.timings_ms[2] = t_trace.as_millis() as u64;
    mark("build: trace");
    report.timings_ms[3] = t_assemble.as_millis() as u64;
    if turborepo_trace_core::timing::enabled() {
        mark(&format!(
            "build: assemble ({} framework config loads, {} memo cells, {} tests of more than one \
             project)",
            crate::fw::LOADS.load(std::sync::atomic::Ordering::Relaxed),
            fw.loaded_count(),
            crate::fw::SHARED_TESTS.load(std::sync::atomic::Ordering::Relaxed)
        ));
    }

    // 5. Third-party packages.
    let t4 = Instant::now();
    let mut entries_out: Vec<ShardEntry> = Vec::new();
    // Third-party identities reached by first-party shards: (name, version) ->
    // integrity.
    let mut links: BTreeMap<(String, String), Option<String>> = BTreeMap::new();
    for (pi, p) in pkgs.iter().enumerate() {
        let key = match &reuse[pi] {
            Some(k) => {
                report.reused += 1;
                if let Some((_, s)) = &p.old {
                    for l in s.get().obs.links.iter().filter(|l| !l.version.is_empty()) {
                        links
                            .entry((l.name.to_string(), l.version.to_string()))
                            .or_insert_with(|| l.integrity.as_ref().map(|i| i.to_string()));
                    }
                }
                k.clone()
            }
            None => {
                let (key, shard_links) = &new_shards[&p.def.root];
                for l in shard_links.iter().filter(|l| !l.version.is_empty()) {
                    links
                        .entry((l.name.clone(), l.version.clone()))
                        .or_insert_with(|| l.integrity.clone());
                }
                key.clone()
            }
        };
        entries_out.push(ShardEntry {
            key,
            kind: "first-party".to_owned(),
            name: p.def.name.clone(),
            version: None,
            root: p.def.root.clone(),
        });
    }
    report.first_party = pkgs.len();
    // Package roots (real paths) per identity, from the link records.
    let roots = third_party_roots(&cx, &pkgs, &reuse, &new_shards);
    // With `--externals follow`, installed packages are traced; packages known only
    // from the lockfile (nothing installed) stay opaque in both modes.
    type Ids = (BTreeMap<(String, String), String>, Vec<(String, String)>);
    let (traced_roots, opaque_ids): Ids = match opts.externals {
        Externals::Opaque => (BTreeMap::new(), links.keys().cloned().collect()),
        Externals::Follow => {
            let mut t = BTreeMap::new();
            let mut o = Vec::new();
            for id in links.keys() {
                // Language packages (namespaced `eco:name`) stay opaque: their files are
                // traced by their own languages' rules, not as `node_modules` packages.
                if id.0.contains(':') {
                    o.push(id.clone());
                    continue;
                }
                match roots.get(id).filter(|r| !r.is_empty()) {
                    Some(r) => {
                        t.insert(id.clone(), r.clone());
                    }
                    None => o.push(id.clone()),
                }
            }
            (t, o)
        }
    };
    for (name, version) in &opaque_ids {
        let integrity = links[&(name.clone(), version.clone())].clone();
        let root = roots
            .get(&(name.clone(), version.clone()))
            .cloned()
            .unwrap_or_default();
        let key = opaque_key(&cx, name, version, integrity.as_deref());
        let shard = Shard {
            format: shard::FORMAT_VERSION,
            key: key.clone(),
            package: PackageRec {
                kind: kind::OPAQUE,
                name: name.clone(),
                version: Some(version.clone()),
                integrity,
                // The installed location is display data, kept in the manifest entry: the
                // shard holds exactly what its key covers.
                root: String::new(),
            },
            files: Vec::new(),
            probes: Vec::new(),
            memberships: Vec::new(),
            configs: Vec::new(),
            path_configs: Vec::new(),
            dep_closures: Vec::new(),
            obs: ObsRec::default(),
            details: Vec::new(),
            tests: Vec::new(),
            tasks: Vec::new(),
            hubs: Vec::new(),
            parts: Vec::new(),
            hub_members: Vec::new(),
            key_only: Vec::new(),
            key_groups: Vec::new(),
            located: Vec::new(),
        };
        if write_shard(&cx, &shard)? {
            report.written += 1;
            report.traced += 1;
        } else {
            report.reused += 1;
        }
        entries_out.push(ShardEntry {
            key,
            kind: "opaque".to_owned(),
            name: name.clone(),
            version: Some(version.clone()),
            root,
        });
    }
    report.third_party = opaque_ids.len();
    if !traced_roots.is_empty() {
        let (entries, stats) = crate::follow::build_third_party(&cx, old.as_ref(), traced_roots)?;
        report.third_party += entries.len();
        report.reused += stats.0;
        report.traced += stats.1;
        report.written += stats.2;
        report.analyzed_files += stats.3;
        entries_out.extend(entries);
    }
    report.timings_ms[4] = t4.elapsed().as_millis() as u64;
    mark("build: third-party");

    // 6. Manifest.
    entries_out.sort_by(|a, b| {
        (
            a.kind.as_str(),
            a.root.as_str(),
            a.name.as_str(),
            a.version.as_deref(),
        )
            .cmp(&(
                b.kind.as_str(),
                b.root.as_str(),
                b.name.as_str(),
                b.version.as_deref(),
            ))
    });
    let mut manifest = Manifest::new(
        &repo,
        opts,
        cx.opts_digest.clone(),
        lock_hashes,
        entries_out,
    );
    flush_shards(&cx)?;
    mark("build: flush shards");
    manifest.files = crate::manifest::write_files(&store, &files);
    report.manifest_written = manifest.write(&out)?;
    mark("build: files list + manifest");
    // The previous snapshot's walked-file list is referenced by nothing now.
    if let Some(prev) = prev_files.filter(|p| Some(p) != manifest.files.as_ref()) {
        let _ = std::fs::remove_file(store.join(prev));
    }
    let _ = cx.hashes.save(None);
    let _ = manifests.save();
    mark("build: hashes save");
    // Listing digests of the walked directories listed in this build (the next
    // build keeps tree parts across mtime-only changes of these directories,
    // `Changed::refine`). A directory not listed now keeps the previous build's
    // digest while its mtime is unchanged (not in the walk's changed list).
    let dir_digests: Vec<(u64, u64)> = {
        let mtime_changed: std::collections::HashSet<u64> =
            walk_state.2.iter().map(|d| dir_hash(d)).collect();
        let dirs: Vec<&PathBuf> = cx.walked_dirs.iter().collect();
        dirs.par_iter()
            .filter_map(|d| {
                let rel = d.strip_prefix(&cx.repo).ok()?.to_string_lossy();
                let k = dir_hash(&rel);
                match cx.fs().cached_listing(d) {
                    Some(l) => Some((k, listing_digest(&l))),
                    None if !mtime_changed.contains(&k) => old_digests.get(&k).map(|&x| (k, x)),
                    None => None,
                }
            })
            .collect()
    };
    mark("build: dir digests");
    // Nothing observed differently from the build the obs cache was written for
    // (same walk, every tree part and listing served from it, no shard
    // written): it is still exact.
    let obs_unchanged = cx.tree_changed.as_ref().is_some_and(|c| c.same_walk)
        && cx.tree_stats[1].load(std::sync::atomic::Ordering::Relaxed) == 0
        && cx.fresh_listings.load(std::sync::atomic::Ordering::Relaxed) == 0
        && report.written == 0;
    if turborepo_trace_core::timing::enabled() {
        mark(&format!(
            "build: obs unchanged {obs_unchanged} (same walk {:?}, tree parts computed {}, fresh \
             listings {}, written {})",
            cx.tree_changed.as_ref().map(|c| c.same_walk),
            cx.tree_stats[1].load(std::sync::atomic::Ordering::Relaxed),
            cx.fresh_listings.load(std::sync::atomic::Ordering::Relaxed),
            report.written
        ));
    }
    report.timings_ms[5] = t0.elapsed().as_millis() as u64;
    mark("build: dir digests + obs cache");
    turborepo_trace_core::timing::report_spans();
    report.manifest = manifest;
    // Freed with the rest of the build state, below: on a large repository the
    // framework memo tables alone take 0.4 to 0.55 s to free.
    let fw_memos = fw.into_memos();
    drop(tasks);
    mark("build: task state dropped");
    // The obs cache is written in the background (the graph load follows); the next
    // build and the CLI wait for it (`wait_for_cache_writes`). Then the build
    // state (dir cache, memo tables, shards) is freed there too, off the
    // caller's critical path.
    let cx = Arc::new(cx);
    let cx2 = Arc::clone(&cx);
    report.obs_written = !obs_unchanged;
    let obs_out = (!obs_unchanged).then(|| out.clone());
    let deep: Arc<std::sync::Mutex<Option<DeepListings>>> = Arc::default();
    let deep2 = Arc::clone(&deep);
    crate::spawn_cache_write(
        &out,
        move || {
            let Some(out) = obs_out else {
                return;
            };
            let cx = cx2;
            let listings = cx.deep_listings();
            write_obs_cache(
                &out,
                &cx.ws,
                &cx.obs_new.lock().unwrap_or_else(|e| e.into_inner()),
                &dir_digests,
                &listings,
                &|d| cx.dir_mtime(d),
                cx.start_ns - 2_000_000_000,
            );
            *deep2.lock().unwrap_or_else(|e| e.into_inner()) = Some(listings);
        },
        move || drop((deep, cx, pkgs, new_shards, fw_memos)),
    );
    report.walked = Arc::try_unwrap(files).unwrap_or_else(|f| (*f).clone());
    Ok(report)
}

/// For every third-party identity linked from first-party shards, the smallest
/// real root path (repo-relative, or absolute outside the repository).
/// What the build keeps of a first-party shard it wrote: the key and the
/// package links.
type NewShard = (String, Vec<crate::shard::LinkRec>);

fn third_party_roots(
    cx: &Cx,
    pkgs: &[PkgState],
    reuse: &[Option<String>],
    new_shards: &HashMap<String, NewShard>,
) -> BTreeMap<(String, String), String> {
    let mut out: BTreeMap<(String, String), String> = BTreeMap::new();
    // The smallest real root; a lockfile-only link (empty root) never wins over an
    // installed one. Workspace links (empty version) are not third-party packages.
    let mut add = |name: &str, version: &str, root: &str| {
        if version.is_empty() {
            return;
        }
        let e = out
            .entry((name.to_owned(), version.to_owned()))
            .or_insert_with(|| root.to_owned());
        if !root.is_empty() && (e.is_empty() || root < e.as_str()) {
            *e = root.to_owned();
        }
    };
    for (pi, p) in pkgs.iter().enumerate() {
        if reuse[pi].is_some() {
            if let Some((_, s)) = &p.old {
                for l in s.get().obs.links.iter() {
                    add(l.name.as_str(), l.version.as_str(), l.root.as_str());
                }
            }
        } else if let Some((_, shard_links)) = new_shards.get(&p.def.root) {
            for l in shard_links {
                add(&l.name, &l.version, &l.root);
            }
        }
    }
    let _ = cx;
    out
}

/// Repo-relative `e` relative to the package root `root`.
fn rel_in(root: &str, e: &str) -> String {
    if root.is_empty() {
        e.to_owned()
    } else {
        e.strip_prefix(root)
            .and_then(|r| r.strip_prefix('/'))
            .unwrap_or(e)
            .to_owned()
    }
}

/// Stages a shard for the store's log unless the store already holds its key
/// (in the pack, its log, or a loose file). Returns whether it is new. Staged
/// shards are appended in batches, and at the end of the build
/// ([`flush_shards`]). Packages are assembled in parallel, so the order of the
/// staged shards, and with it the order of the log's records, is the order the
/// threads finished in: not a function of the tree, and read by nothing
/// ([`shard::Pack`]).
pub(crate) fn write_shard(cx: &Cx, s: &Shard) -> Result<bool, Error> {
    if cx.pack.as_ref().is_some_and(|p| p.contains(&s.key)) || cx.store.join(&s.key).exists() {
        return Ok(false);
    }
    let bytes = shard::encode(s);
    let mut sink = cx.sink.lock().unwrap_or_else(|e| e.into_inner());
    if !sink.keys.insert(s.key.clone()) {
        return Ok(false);
    }
    sink.bytes += bytes.len();
    sink.staged.push((s.key.clone(), bytes));
    if sink.bytes >= SINK_FLUSH_BYTES {
        sink.flush(&cx.store)?;
    }
    Ok(true)
}

/// Appends the shards staged by [`write_shard`] to the store's log.
pub(crate) fn flush_shards(cx: &Cx) -> Result<(), Error> {
    cx.sink
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .flush(&cx.store)
}

/// Whether `TURBO_TRACE_DEBUG_DEBUG_ASSEMBLE` is set: per-shard assemble
/// timings on stderr.
fn debug_assemble() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| turborepo_trace_core::debug::var_os("DEBUG_ASSEMBLE").is_some())
}

/// Builds a first-party shard from its files' analyses and, for the entries
/// `plan` keeps, from their records in the package's previous shard.
// Invariant: a file is kept only when a previous shard has it.
#[allow(clippy::expect_used)]
fn assemble_first_party(
    cx: &Cx,
    fw: &crate::fw::Fw<'_>,
    tasks: &crate::tasks::Tasks<'_>,
    p: &PkgState,
    fresh: Vec<(usize, FileAnalysis)>,
    plan: Option<&crate::filereuse::Plan>,
) -> Shard {
    let old = plan.and(p.old.as_ref()).map(|(_, s)| s.get());
    let mut files: Vec<(usize, FileSource)> = fresh
        .into_iter()
        .map(|(ei, a)| (ei, FileSource::Fresh(a)))
        .collect();
    if let Some(plan) = plan {
        files.extend(
            plan.keep
                .iter()
                .enumerate()
                .filter_map(|(ei, fi)| fi.map(|fi| (ei, FileSource::Kept(fi)))),
        );
    }
    files.sort_by_key(|(ei, _)| *ei);
    let root_abs = cx.pkg_root_abs(&p.def.root);
    // Framework inputs: extra edges and groups, test marks, key-only config inputs.
    // Per file, in parallel (a large package must not be a sequential long pole).
    // Build tasks (see `crate::tasks`) are discovered next to that: task analysis
    // of a big package (the root) is long and mostly sequential.
    type TestMark = (String, &'static str, Vec<PathBuf>);
    type FileFw = (
        Vec<TestMark>,
        Vec<PathBuf>,
        Vec<crate::fw::Reached>,
        Vec<Arc<crate::shard::LocatedRec>>,
    );
    let (per_file, pt): (Vec<FileFw>, _) = rayon::join(
        || {
            files
                .par_iter_mut()
                .map(|(ei, src)| {
                    let rel = rel_in(&p.def.root, &p.entries[*ei]);
                    let a = match src {
                        FileSource::Fresh(a) => a,
                        // A kept file: the test marks and key-only configs the previous
                        // shard recorded for it.
                        FileSource::Kept(fi) => {
                            let old = old.expect("a kept file has a previous shard");
                            let (t, k, r) =
                                crate::filereuse::kept_framework(cx, old, *fi, &root_abs, rel);
                            return (t, k, r, Vec::new());
                        }
                    };
                    static AUGMENT: Span = Span::new("assemble: framework augment, per file");
                    let _t = AUGMENT.enter();
                    mark_generated_header(&p.entries[*ei], a);
                    let ff = fw.augment(&cx.repo.join(&p.entries[*ei]), a);
                    (
                        ff.tests
                            .into_iter()
                            .map(|(t, cwds)| (rel.clone(), t, cwds))
                            .collect(),
                        ff.key_reads.into_iter().collect(),
                        ff.reached,
                        ff.located,
                    )
                })
                .collect()
        },
        || {
            static TASKS: Span = Span::new("assemble: package tasks");
            let _t = TASKS.enter();
            let tt = Instant::now();
            let pt = tasks.package_tasks(&p.def.root, &root_abs);
            if debug_assemble() {
                eprintln!(
                    "assemble {:?}: tasks {} ms",
                    p.def.root,
                    tt.elapsed().as_millis()
                );
            }
            pt
        },
    );
    let mut tests: Vec<TestMark> = Vec::new();
    let mut key_reads: BTreeSet<PathBuf> = BTreeSet::new();
    // Each file's own key-only configs, in the order of `files` (the shard's file
    // order).
    let mut file_keys: Vec<Vec<PathBuf>> = Vec::with_capacity(per_file.len());
    // What the framework pass found for an entry without making it an edge target.
    let mut reached: HashMap<usize, Vec<crate::fw::Reached>> = HashMap::new();
    // The location records of the configs that were consulted for the shard's files
    // ([`crate::located`]). A kept file was consulted for the configs of the
    // previous shard, whose contents and states are unchanged
    // ([`crate::filereuse::plan`]).
    let mut located: BTreeSet<crate::shard::LocatedRec> = BTreeSet::new();
    if let (Some(old), true) = (
        old,
        files.iter().any(|(_, f)| matches!(f, FileSource::Kept(_))),
    ) {
        located.extend(old.located.iter().map(|l| crate::shard::LocatedRec {
            config: l.config.to_string(),
            spec: l.spec.to_string(),
            root: l.root.to_string(),
        }));
    }
    for ((t, k, r, l), (ei, _)) in per_file.into_iter().zip(&files) {
        located.extend(l.iter().map(|l| (**l).clone()));
        tests.extend(t);
        key_reads.extend(k.iter().cloned());
        file_keys.push(k);
        if !r.is_empty() {
            reached.insert(*ei, r);
        }
    }
    key_reads.extend(pt.key_reads.iter().cloned());
    let mut rels: Vec<String> = p.entries.iter().map(|e| rel_in(&p.def.root, e)).collect();
    // Build tasks: pseudo-files after the entries.
    let mut pseudo_langs: Vec<(String, &'static str)> = Vec::new();
    for (rel, a, lang) in pt.files {
        files.push((rels.len(), FileSource::Fresh(a)));
        pseudo_langs.push((rel.clone(), lang));
        rels.push(rel);
    }
    let file_ix: HashMap<&str, u32> = rels
        .iter()
        .enumerate()
        .map(|(i, r)| (r.as_str(), i as u32))
        .collect();
    let package = PackageRec {
        kind: kind::FIRST_PARTY,
        name: p.def.name.clone(),
        version: None,
        integrity: None,
        root: p.def.root.clone(),
    };
    let prehash = |ei: usize| p.hashes.as_ref().and_then(|h| h.get(ei).copied().flatten());
    let target_cx = TargetCx {
        cx,
        root_abs: &root_abs,
        own: Some(cx.ws.by_root(&p.def.root).unwrap_or(0)),
        file_ix: &file_ix,
        follow: false,
    };
    let mut shard = assemble(
        cx, package, &root_abs, &rels, files, old, &reached, prehash, &target_cx,
    );
    {
        let by_path: HashMap<String, usize> = shard
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();
        for (rel, lang) in &pseudo_langs {
            if let Some(&i) = by_path.get(rel) {
                shard.files[i].lang = Some((*lang).to_owned());
            }
        }
        let mut recs = pt.recs;
        for r in &mut recs {
            r.file = by_path.get(&format!("#{}", r.id)).copied().unwrap_or(0) as u32;
        }
        recs.sort_by(|a, b| a.id.cmp(&b.id));
        shard.tasks = recs;
        shard.dep_closures = pt.dep_closures;
        let mut hubs: Vec<crate::shard::HubRec> = pt
            .hubs
            .into_iter()
            .filter_map(|(rel, mut h)| {
                h.file = *by_path.get(&rel)? as u32;
                Some(h)
            })
            .collect();
        hubs.sort_by_key(|h| h.file);
        shard.hubs = hubs;
        let mut parts: Vec<crate::shard::PartRec> = pt
            .parts
            .into_iter()
            .filter_map(|(rel, mut p)| {
                p.file = *by_path.get(&rel)? as u32;
                Some(p)
            })
            .collect();
        parts.sort_by_key(|p| p.file);
        shard.parts = parts;
    }
    let ix: HashMap<&str, u32> = shard
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.as_str(), i as u32))
        .collect();
    let mut recs: Vec<crate::shard::TestRec> = tests
        .iter()
        .filter_map(|(rel, t, cwds)| {
            // Working directories inside the repository, repo-relative.
            let mut cwds: Vec<String> = cwds
                .iter()
                .filter_map(|d| {
                    let r = d.strip_prefix(&cx.repo).ok()?;
                    Some(r.to_string_lossy().replace('\\', "/"))
                })
                .collect();
            cwds.sort();
            cwds.dedup();
            Some(crate::shard::TestRec {
                file: *ix.get(rel.as_str())?,
                framework: (*t).to_owned(),
                cwds,
            })
        })
        .collect();
    recs.sort();
    recs.dedup();
    shard.tests = recs;
    shard.located = located.into_iter().collect();
    // Listings that only hub pseudo-files reference: keyed by the hubs' states, not
    // by the listing ([`crate::shard::Shard::hub_members`]).
    {
        let mut readers: Vec<u8> = vec![0; shard.memberships.len()];
        for f in &shard.files {
            let bit = if f.lang.as_deref() == Some(crate::tasks::MEMBERS_LANG) {
                1
            } else {
                2
            };
            for r in &f.refs {
                for &m in &r.memberships {
                    readers[m as usize] |= bit;
                }
            }
        }
        for g in &shard.key_groups {
            for &m in &g.memberships {
                readers[m as usize] |= 2;
            }
        }
        shard.hub_members = (0..readers.len() as u32)
            .filter(|&i| readers[i as usize] == 1)
            .collect();
    }
    // What task discovery looked for in a package without tasks, and what a source
    // decided its tasks by without an input of them, stays in the key (the
    // first workflow, Makefile or script of the package, a Python package that
    // comes or goes, must re-key the shard).
    if !pt.discovery.is_empty() {
        let seen =
            crate::normalize::add_key_only(&cx.normcx(&root_abs, false), &pt.discovery, &mut shard);
        // The places the observations went through (links, package directories): where
        // each leads is part of the key, as for a group's ([`TargetCx::seen`]).
        let mut have: std::collections::HashSet<LinkRec> =
            shard.obs.links.iter().cloned().collect();
        for dir in seen {
            if let Some(l) = seen_link(cx, &root_abs, &dir)
                && have.insert(l.clone())
            {
                shard.obs.links.push(l);
            }
        }
    }
    // Key-only config inputs: in the table (contents hashed into the key),
    // referenced by no group.
    for r in key_reads {
        let c = rel_to(&root_abs, &cx.repo, &r);
        match shard.configs.iter().position(|x| *x == c) {
            Some(i) => shard.path_configs.retain(|&j| j as usize != i),
            None => shard.configs.push(c),
        }
    }
    // The listings that only sets of key observations name
    // ([`crate::shard::Shard::key_only`]): what a config's load enumerated (the
    // haste map of a jest project, the directories a `projects` glob matches),
    // for every file the load is consulted for.
    {
        let mut read = vec![false; shard.memberships.len()];
        for f in &shard.files {
            if f.lang.as_deref() == Some(crate::tasks::MEMBERS_LANG) {
                continue;
            }
            for r in &f.refs {
                for &m in &r.memberships {
                    read[m as usize] = true;
                }
            }
        }
        let keyed: BTreeSet<u32> = shard
            .key_groups
            .iter()
            .flat_map(|g| g.memberships.iter().copied())
            .collect();
        // Only the listings of a subtree: the entries of one directory (the directories
        // a `projects` glob matches) are part of the tree part like a probe,
        // which then depends on that one directory and is reused while it is
        // unchanged. (As a key-only listing it would be listed again by every
        // refresh.)
        shard.key_only = keyed
            .into_iter()
            .filter(|&i| {
                !read[i as usize] && is_deep_pattern(&shard.memberships[i as usize].pattern)
            })
            .collect();
    }
    // Each file's own key-only configs, as indexes: what decides its framework
    // inputs besides what its groups reference
    // ([`crate::shard::FileRec::key_configs`]).
    if file_keys.iter().any(|k| !k.is_empty()) {
        let config_ix: HashMap<&str, u32> = shard
            .configs
            .iter()
            .enumerate()
            .map(|(i, c)| (c.as_str(), i as u32))
            .collect();
        for (f, keys) in shard.files.iter_mut().zip(&file_keys) {
            let mut ix: Vec<u32> = keys
                .iter()
                .filter_map(|r| {
                    config_ix
                        .get(rel_to(&root_abs, &cx.repo, r).as_str())
                        .copied()
                })
                .collect();
            ix.sort_unstable();
            ix.dedup();
            f.key_configs = ix;
        }
    }
    shard
}

/// Generated-file headers: a freshly read first-party file whose
/// first bytes carry a generated-code marker
/// ([`turborepo_trace_tasks::recognizers::codegen::headers`]) gets a
/// `generated_file` diagnostic naming it. It is stored with the extraction (so
/// cached extractions keep it) and dropped by generated-file linking when a
/// task output covers the file; otherwise importers reach it as uncertainty
/// (the producer's inputs are unknown).
fn mark_generated_header(rel: &str, a: &mut FileAnalysis) {
    use turborepo_trace_tasks::recognizers::codegen::headers::{HEAD_BYTES, generated_marker};
    // The analysis keeps the head of a freshly read file: no second open (file
    // opens are serialized by endpoint security on macOS).
    const _: () = assert!(turborepo_trace_core::FILE_HEAD_BYTES >= HEAD_BYTES);
    if a.hash.is_none() {
        return;
    }
    let Some(head) = a.head.take() else {
        return;
    };
    let Some(ex) = a.extraction.as_mut() else {
        return;
    };
    if let Some(marker) = generated_marker(&head[..head.len().min(HEAD_BYTES)]) {
        ex.diagnostics.push(Uncertainty::new(
            turborepo_trace_core::UncertaintyKind::GeneratedFile,
            turborepo_trace_core::Span::DUMMY,
            format!("{rel} is generated (`{marker}`) but no task produces it"),
        ));
        ex.diagnostic_lines.push(1);
    }
}

/// How edge targets are classified for one shard.
pub(crate) struct TargetCx<'a> {
    pub cx: &'a Cx,
    pub root_abs: &'a Path,
    /// The workspace package of a first-party shard.
    pub own: Option<usize>,
    /// Files of this shard by package-relative path.
    pub file_ix: &'a HashMap<&'a str, u32>,
    /// Whether this is a traced third-party shard (its own package's files are
    /// `File`).
    pub follow: bool,
}

#[derive(Default)]
pub(crate) struct ObsAcc {
    pub found: BTreeSet<String>,
    pub links: BTreeSet<LinkRec>,
    /// Version pins of language packages linked by the current reference group
    /// (absolute), drained into its config inputs.
    pub pins: BTreeSet<PathBuf>,
    /// Manifests the current reference group resolved through, by their
    /// resolution view (absolute), drained into its config inputs.
    pub view_pins: BTreeSet<PathBuf>,
    /// Observations of resolutions made while classifying the current reference
    /// group's edges (workspace links resolved from the lockfile), drained
    /// into its observations.
    pub extra: turborepo_trace_core::Observations,
}

impl TargetCx<'_> {
    /// The record of a load next to a module that is not installed
    /// ([`turborepo_trace_core::located`]), when the lockfile links the
    /// importer to a package of the repository: the open record (a path
    /// next to that package is a file of the repository, which the edge
    /// does not have). `None` when the resolver's record stands:
    /// the load is taken to stay in the tree of a registry package, or nothing
    /// links the name (it cannot be resolved until something is installed,
    /// which the `node_modules` probes and the lockfile link see).
    ///
    /// The lockfile's answer is an observation of the shard. For the file's own
    /// package the bare module edge of the same reference made the link
    /// ([`TargetCx::via_lockfile`]); for a search that started in another
    /// package of the repository the link of that importer is recorded
    /// here, without an identity (the key and the per-file reuse check
    /// a lock link by its location).
    fn located_in_repository(
        &self,
        edge: &turborepo_trace_core::Edge,
        u: &Uncertainty,
        acc: &mut ObsAcc,
    ) -> Option<Uncertainty> {
        use turborepo_trace_core::UncertaintyKind as K;
        if u.kind != K::UnresolvedSpecifier {
            return None;
        }
        let (package, from) = turborepo_trace_core::located::parse(&edge.spec)?;
        let cx = self.cx;
        let own = self.own?;
        let importer = match from {
            None => own,
            Some(dir) => {
                let rel = Path::new(dir).strip_prefix(&cx.repo).ok()?;
                cx.ws.owner(&format!("{}/_", rel.display()))
            }
        };
        let def = &cx.ws.packages[importer];
        if def.name.contains(':') {
            return None;
        }
        if importer != own {
            acc.links.insert(LinkRec {
                location: format!("{LOCK_LINK}{}:{package}", def.root),
                name: package.to_owned(),
                version: String::new(),
                integrity: None,
                root: String::new(),
            });
        }
        match cx.ids.lock_resolve(&def.root, package) {
            Some(crate::identity::LockTarget::Workspace(_)) => {
                let mut open = u.clone();
                open.kind = K::ComputedSpecifier;
                open.detail = turborepo_trace_core::located::open_detail(&u.detail).into();
                Some(open)
            }
            _ => None,
        }
    }

    /// An unresolved bare module specifier of a first-party file, resolved
    /// through the lockfile importer of its package (nothing installed). A
    /// workspace dependency (`link:`) resolves as through a
    /// `node_modules/<name>` link to the package directory: the specifier
    /// is resolved in that directory, by self-reference when the package has
    /// `exports`, else as `./<subpath>` (`main`, extension and index probing).
    /// The target's manifest is a config input of the reference and the
    /// resolution's probes are its observations.
    fn via_lockfile(
        &self,
        edge: &turborepo_trace_core::Edge,
        acc: &mut ObsAcc,
    ) -> Option<Vec<TargetRec>> {
        use turborepo_trace_core::RefKind as K;
        let own = self.own?;
        if !matches!(
            edge.kind,
            K::Import | K::Require | K::DynamicImport | K::ReExport
        ) {
            return None;
        }
        let spec: &str = &edge.spec;
        if spec.is_empty()
            || spec.starts_with(['.', '/', '#'])
            || spec.contains(':')
            || spec.contains(' ')
        {
            return None;
        }
        let mut it = spec.splitn(if spec.starts_with('@') { 3 } else { 2 }, '/');
        let name = if spec.starts_with('@') {
            format!("{}/{}", it.next()?, it.next()?)
        } else {
            it.next()?.to_owned()
        };
        let sub = it.next().unwrap_or("").to_owned();
        let cx = self.cx;
        // Lockfile-only resolution is for JS bare specifiers; language packages
        // (Python, Go, Rust) resolve through their own layouts.
        if cx.ws.packages[own].name.contains(':') {
            return None;
        }
        let importer = &cx.ws.packages[own].root;
        // A type-only import that the lockfile doesn't resolve is satisfied by the
        // package's DefinitelyTyped stubs, as TypeScript resolves it (`import
        // type {X} from 'retry'` → `@types/retry`). Runtime never loads it.
        // A name the lockfile does not resolve is an observation too: the importer
        // gaining the dependency (or the repository its first lockfile)
        // resolves this reference. Recorded as a link with no identity, which
        // the key checks like any lock link.
        let absent = |name: &str, acc: &mut ObsAcc| {
            acc.links.insert(LinkRec {
                location: format!("{LOCK_LINK}{importer}:{name}"),
                name: name.to_owned(),
                version: String::new(),
                integrity: None,
                root: String::new(),
            });
        };
        let (name, target) = match cx.ids.lock_resolve(importer, &name) {
            Some(t) => (name, t),
            None if edge
                .flags
                .contains(turborepo_trace_core::RefFlags::TYPE_ONLY)
                && !name.starts_with("@types/") =>
            {
                absent(&name, acc);
                let types = types_package(&name);
                let Some(t) = cx.ids.lock_resolve(importer, &types) else {
                    absent(&types, acc);
                    return None;
                };
                (types, t)
            }
            None => {
                absent(&name, acc);
                return None;
            }
        };
        let location = format!("{LOCK_LINK}{importer}:{name}");
        match target {
            crate::identity::LockTarget::Package(id) => {
                acc.links.insert(LinkRec {
                    location,
                    name: id.name.clone(),
                    version: id.version.clone(),
                    integrity: id.integrity.clone(),
                    root: String::new(),
                });
                Some(vec![TargetRec::Cross(CrossRec {
                    name: id.name,
                    version: Some(id.version),
                    subpath: sub,
                })])
            }
            crate::identity::LockTarget::Workspace(root) => {
                let target = cx.ws.by_root(&root)?;
                let def = &cx.ws.packages[target];
                let manifest = cx.repo.join(def.manifest.as_ref()?);
                acc.links.insert(LinkRec {
                    location,
                    name: def.name.clone(),
                    version: String::new(),
                    integrity: None,
                    root: root.clone(),
                });
                // `exports`, `main` and `types` decide the target: contents are an input.
                acc.view_pins.insert(manifest.clone());
                let dir = cx.pkg_root_abs(&root);
                let has_exports = cx
                    .fs()
                    .read_manifest(&manifest)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .is_some_and(|m| m.get("exports").is_some());
                let request = if has_exports {
                    spec.to_owned()
                } else if sub.is_empty() {
                    "./".to_owned()
                } else {
                    format!("./{sub}")
                };
                let src = format!(
                    "require({});\n",
                    serde_json::to_string(&request).unwrap_or_default()
                );
                let a = cx
                    .tracer
                    .analyze_source(&dir.join("__turbotrace_link__.js"), src.as_bytes());
                let mut out = Vec::new();
                for g in a.groups.iter().filter(|g| g.ref_index == Some(0)) {
                    acc.extra.extend_from(&g.observations);
                    for e in &a.edges[g.edges.clone()] {
                        let links: Vec<PathBuf> = e
                            .links
                            .iter()
                            .map(|l| cx.tracer.path_of(*l).to_path_buf())
                            .collect();
                        for s in &e.succ {
                            let path = cx.tracer.path_of(*s).to_path_buf();
                            out.push(self.classify(&path, &links, acc));
                        }
                    }
                }
                if out.is_empty() {
                    // Unresolvable inside the package: the manifest stands for it.
                    acc.found.insert(rel_to(self.root_abs, &cx.repo, &manifest));
                    out.push(TargetRec::Cross(CrossRec {
                        name: def.name.clone(),
                        version: None,
                        subpath: "package.json".to_owned(),
                    }));
                }
                Some(out)
            }
        }
    }

    /// Whether `path` lies in a `node_modules` directory above the repository
    /// root: an install outside the repository (another checkout's, a
    /// parent directory's), which the repository's own lockfile does not
    /// describe and CI does not have.
    fn outside_install(&self, path: &Path) -> bool {
        let repo = &self.cx.repo;
        if path.starts_with(repo) {
            return false;
        }
        repo.ancestors()
            .skip(1)
            .any(|a| path.starts_with(a.join("node_modules")))
    }

    fn classify(&self, path: &Path, links: &[PathBuf], acc: &mut ObsAcc) -> TargetRec {
        let cx = self.cx;
        let repo = &cx.repo;
        // A task pseudo-file is named after an id or after globs
        // (`<package>/#members:<dir>/<pattern> !<exclude>`): a `node_modules` in the
        // name is text of a pattern (`!**/node_modules/**`, the excludes of a
        // `.dockerignore` or a tsconfig), not a directory. Read as a path, such
        // a hub became a file of a third-party package called
        // `node_modules/**`, and its task lost every member.
        let pseudo = is_task_pseudo_path(path);
        // A language's third-party package (Python distribution, Go module, crate).
        if !self.follow
            && !pseudo
            && let Some(li) = cx.ids.language(path, cx.fs())
        {
            acc.pins.extend(li.pins.iter().cloned());
            acc.links.insert(LinkRec {
                location: rel_to(self.root_abs, repo, &li.root),
                name: li.id.name.clone(),
                version: li.id.version.clone(),
                integrity: li.id.integrity.clone(),
                root: rel_to(repo, repo, &li.root),
            });
            return TargetRec::Cross(CrossRec {
                name: li.id.name,
                version: Some(li.id.version),
                subpath: rel_to(&li.root, repo, path),
            });
        }
        if !pseudo
            && cx.ids.is_third_party(path)
            && let Some((proot, pname)) = cx.ids.package_root(path)
        {
            if self.follow && proot == self.root_abs {
                let rel = rel_to(self.root_abs, repo, path);
                return match self.file_ix.get(rel.as_str()) {
                    Some(&i) => TargetRec::File(i),
                    None => {
                        acc.found.insert(rel.clone());
                        TargetRec::Intra(rel)
                    }
                };
            }
            let id = cx.ids.of_root(cx.fs(), &proot, &pname);
            let suffix = Path::new("node_modules").join(&pname);
            let location = links
                .iter()
                .find(|l| l.ends_with(&suffix))
                .cloned()
                .unwrap_or_else(|| proot.clone());
            acc.links.insert(LinkRec {
                location: rel_to(self.root_abs, repo, &location),
                name: id.name.clone(),
                version: id.version.clone(),
                integrity: id.integrity.clone(),
                root: rel_to(repo, repo, &proot),
            });
            return TargetRec::Cross(CrossRec {
                name: id.name,
                version: Some(id.version),
                subpath: rel_to(&proot, repo, path),
            });
        }
        // A first-party file, or one outside the repository: the links the resolution
        // crossed (a workspace package's `node_modules/<name>` link) decide that it is
        // reached at all. A link that goes or leads elsewhere changes the resolution,
        // and the file it used to lead to is still there.
        if !pseudo {
            self.crossed(links, acc);
            // A link of the tree is an input of the reference at its own path, too: a
            // diff that makes it lead elsewhere names the link and nothing else, and a
            // query finds the reference by it. (A package directory is made by an
            // install: what it leads to changes with the lockfile or a manifest.)
            for l in links {
                if l.starts_with(repo) && !cx.ids.is_third_party(&l.join("x")) {
                    acc.extra.reads.push(l.clone());
                }
            }
        }
        if let Ok(rel) = path.strip_prefix(repo) {
            let rel = rel.to_string_lossy();
            let owner = cx.ws.owner(&rel);
            if Some(owner) == self.own {
                let prel = rel_to(self.root_abs, repo, path);
                return match self.file_ix.get(prel.as_str()) {
                    Some(&i) => TargetRec::File(i),
                    None => {
                        acc.found.insert(prel.clone());
                        TargetRec::Intra(prel)
                    }
                };
            }
            acc.found.insert(rel_to(self.root_abs, repo, path));
            let def = &cx.ws.packages[owner];
            return TargetRec::Cross(CrossRec {
                name: def.name.clone(),
                version: None,
                subpath: rel_in(&def.root, &rel),
            });
        }
        let s = path.to_string_lossy().into_owned();
        acc.found.insert(s.clone());
        TargetRec::Outside(s)
    }

    /// Records the place `location` (a link, a package directory) as seen by
    /// the current file: a link without identity whose state is where the
    /// place leads now ([`seen_digest`]).
    fn seen(&self, location: &Path, acc: &mut ObsAcc) {
        if let Some(l) = seen_link(self.cx, self.root_abs, location) {
            acc.links.insert(l);
        }
    }

    /// Records the symbolic links a resolution crossed.
    fn crossed(&self, links: &[PathBuf], acc: &mut ObsAcc) {
        for l in links {
            self.seen(l, acc);
        }
    }

    /// Records `path`, which a lookup found without making it the target of an
    /// edge (a module the framework pass resolved to name it in a marker,
    /// [`crate::fw::FileFw`]), as reached by the current file: the links
    /// crossed, then the package directory for a file of a language's or an
    /// installed package (where it leads is its state), else the
    /// file itself (present, and owned as before), also when it is a file of
    /// this shard.
    pub(crate) fn reach(&self, path: &Path, links: &[PathBuf], acc: &mut ObsAcc) {
        if is_task_pseudo_path(path) {
            return;
        }
        let cx = self.cx;
        self.crossed(links, acc);
        // Links that were crossed without a file to name ([`crate::fw::Reached`]).
        if path.as_os_str().is_empty() {
            return;
        }
        if let Some(li) = cx.ids.language(path, cx.fs()) {
            self.seen(&li.root, acc);
        } else if cx.ids.is_third_party(path)
            && let Some((proot, _)) = cx.ids.package_root(path)
        {
            self.seen(&proot, acc);
        } else if path.starts_with(&cx.repo) {
            acc.found.insert(rel_to(self.root_abs, &cx.repo, path));
        } else {
            acc.found.insert(path.to_string_lossy().into_owned());
        }
    }
}

/// The record of a place seen at `location` (a link an observation went
/// through, a package directory a lookup entered) in the shard of the package
/// at `root_abs`.
pub(crate) fn seen_link(cx: &Cx, root_abs: &Path, location: &Path) -> Option<LinkRec> {
    let state = seen_state(cx, location);
    // Outside the repository and leading nowhere: not recorded, like a probe there
    // (the directory that would cover it is the repository's parent, which is
    // anyone's).
    if state.real.is_none() && !location.starts_with(&cx.repo) {
        return None;
    }
    Some(LinkRec {
        location: format!("{SEEN_LINK}{}", rel_to(root_abs, &cx.repo, location)),
        // The installed package it leads into, if any: such a place is checked with the
        // package links, every other one with the tree ([`seen_state`]).
        name: state.package.clone().unwrap_or_default(),
        version: String::new(),
        integrity: None,
        root: String::new(),
    })
}

/// Whether `path` is a task pseudo-file (`<dir>/#members:…`, `<dir>/#task:…`,
/// `<dir>/#tsc:…`, see [`crate::tasks`]): a component that starts with `#` and
/// names a kind. Everything after that component is the rest of an id or a
/// glob, whatever slashes it has.
fn is_task_pseudo_path(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .and_then(|s| s.strip_prefix('#'))
            .is_some_and(|rest| {
                let kind = rest.split(':').next().unwrap_or("");
                rest.len() > kind.len()
                    && !kind.is_empty()
                    && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    })
}

/// An uncertainty record with the repo-relative scope of a covered one, and its
/// effect.
pub(crate) type PendingUnc = (UncRec, Option<crate::shard::ScopeRec>, u8);

/// A reference group before its observations are interned into the shard
/// tables.
pub(crate) struct PendingRef {
    pub rec: RefRec,
    pub norm: crate::normalize::Normalized,
    pub pins: Vec<String>,
}

/// A file's records before interning: from its analysis, or from the previous
/// shard ([`crate::filereuse::pending`]).
pub(crate) struct PendingFile {
    pub rec: FileRec,
    pub refs: Vec<PendingRef>,
    /// The sets of key observations the file depends on.
    pub keys: Vec<PendingKey>,
    pub uncs: Vec<PendingUnc>,
    pub acc: ObsAcc,
}

/// The records of a set of key observations: its observations, and the package
/// directories they were located through (links without identity,
/// [`SEEN_LINK`]).
pub(crate) type KeyRecords = (crate::normalize::Normalized, BTreeSet<LinkRec>, bool);

/// A set of key observations ([`turborepo_trace_core::record::key`]) that a
/// file names, before it is interned into the shard tables. Many files name the
/// same set: it is normalized once per shard (a set from an analysis, which the
/// resolver hands out by reference: [`KeyNorms`]) or read once from the
/// previous shard, and interned once per shard.
pub(crate) enum PendingKey {
    /// A set from an analysis, known by its identity (it stays alive here).
    Fresh {
        set: Arc<turborepo_trace_core::Observations>,
    },
    /// A set of the previous shard, by its index there
    /// ([`crate::filereuse::key_set`]).
    Kept(u32),
}

/// The records of the sets of key observations that the analyses of one shard
/// name, by the set's identity: computed by the first file that names a set,
/// for every chunk of files. (A set that every file below a config names, the
/// load of that config, was normalized once per chunk of 32 files: on a
/// repository of 30,000 files a thousand times.)
type KeyNorms = dashmap::DashMap<usize, Arc<std::sync::OnceLock<KeyRecords>>>;

/// Where a file's records come from when a shard is assembled.
// In a cold build every file is `Fresh`: boxing the analysis would cost an allocation per
// file to save space in the few lists that hold kept files.
#[allow(clippy::large_enum_variant)]
pub(crate) enum FileSource {
    /// Its analysis in this build.
    Fresh(FileAnalysis),
    /// Its records in the previous shard, by file index: nothing its references
    /// observed changed ([`crate::filereuse::plan`]).
    Kept(u32),
}

/// Assembles a shard from analyses (shared by first- and third-party shards).
///
/// Per-file work (target classification, observation normalization, extraction
/// records) runs in parallel over chunks of files, each chunk with its own
/// `locate` memo; the results are interned into the shard tables sequentially
/// in file order, so the tables keep their first-seen order and the shard bytes
/// do not depend on the thread count.
///
/// A file whose source is [`FileSource::Kept`] takes its records from `old`,
/// the package's previous shard, instead of an analysis.
#[allow(clippy::too_many_arguments)]
// Invariant: a file is kept only when a previous shard has it; every chunk is done, so the key
// set's records are set and this map holds the last reference; each key set is taken once.
#[allow(clippy::expect_used)]
pub(crate) fn assemble(
    cx: &Cx,
    package: PackageRec,
    root_abs: &Path,
    rels: &[String],
    files: Vec<(usize, FileSource)>,
    old: Option<&ArchivedShard>,
    reached: &HashMap<usize, Vec<crate::fw::Reached>>,
    prehash: impl Fn(usize) -> Option<u128> + Sync,
    tcx: &TargetCx<'_>,
) -> Shard {
    // First-party shards drop observations inside installed packages: their link
    // identities cover them. Traced third-party shards keep them.
    let ncx = cx.normcx(root_abs, package.kind == kind::THIRD_PARTY);
    let repo_string = cx.repo.to_string_lossy();
    let repo_str: &str = &repo_string;
    // An edge's specifier as stored: the source text, without the checkout's
    // directory (a resolver names a config or a manifest by its absolute path).
    let spec_of = |e: &turborepo_trace_core::AnalyzedEdge| -> String {
        crate::paths::without_repo(repo_str, &e.edge.spec).into_owned()
    };
    // `key_seen`: the key sets an earlier file of the chunk named, by identity.
    type KeySeen = std::collections::HashSet<usize>;
    let key_norms = KeyNorms::new();
    let one = |memo: &mut crate::normalize::Located,
               key_seen: &mut KeySeen,
               ei: usize,
               a: FileAnalysis|
     -> PendingFile {
        static ONE: Span = Span::new("assemble: classify targets + normalize, per file");
        let _t = ONE.enter();
        let mut acc = ObsAcc::default();
        let mut keys: Vec<PendingKey> = Vec::new();
        let rel = rels[ei].clone();
        let mut uncs: Vec<PendingUnc> = Vec::new();
        let mut seen_unc = std::collections::HashSet::new();
        // `effect`: what the reference that raised the record does with its target
        // (`crate::shard::EFFECT_*`).
        let mut push_unc = |u: &Uncertainty, line: u32, effect: u8, uncs: &mut Vec<PendingUnc>| {
            let mut r = unc_rec(u, line);
            // No record names the directory the checkout lives in.
            if let std::borrow::Cow::Owned(d) = crate::paths::without_repo(repo_str, &r.detail) {
                r.detail = d;
            }
            if seen_unc.insert((r.kind, r.span, r.detail.clone())) {
                uncs.push((r, scope_rec(&cx.repo, u), effect));
            }
        };
        if let Some(e) = &a.read_error {
            push_unc(e, 0, crate::shard::EFFECT_NONE, &mut uncs);
        }
        if let Some(ex) = &a.extraction {
            for (d, &l) in ex.diagnostics.iter().zip(&ex.diagnostic_lines) {
                push_unc(d, l, crate::shard::EFFECT_NONE, &mut uncs);
            }
        }
        let mut refs = Vec::with_capacity(a.groups.len());
        // Whether a tsconfig with aliases decides one of the file's records
        // ([`crate::shard::FileRec::aliased`]): its own resolutions, and those made for
        // it elsewhere (a link resolved into a package, `acc.extra`).
        let mut aliased = false;
        for g in &a.groups {
            let line = match (g.ref_index, &a.extraction) {
                (Some(i), Some(ex)) => ex.ref_lines.get(i as usize).copied().unwrap_or(0),
                _ => 0,
            };
            let mut edges = Vec::new();
            let group = &a.edges[g.edges.clone()];
            // Resolved only into an install outside the repository (`node_modules` above
            // the root), which the repository's lockfile does not describe and
            // CI does not have.
            let outside_only = |e: &turborepo_trace_core::AnalyzedEdge| {
                !e.succ.is_empty()
                    && e.succ
                        .iter()
                        .all(|s| tcx.outside_install(&cx.tracer.path_of(*s)))
            };
            // Unresolved, or resolved outside the repository: the importer's lockfile entry
            // decides. When it does, the reference's other outside-only edges (the outside
            // package's manifest reads) belong to the discarded resolution and are dropped.
            let replaced: Vec<Option<Vec<TargetRec>>> = group
                .iter()
                .map(|e| {
                    let lockable = if e.succ.is_empty() {
                        matches!(e.edge.to, turborepo_trace_core::Target::Unresolved)
                    } else {
                        outside_only(e)
                    };
                    if lockable {
                        tcx.via_lockfile(&e.edge, &mut acc)
                    } else if e.succ.is_empty() {
                        // A language package known from its manifest but not installed.
                        crate::external::link(tcx, &e.edge).map(|(link, cross, pins)| {
                            acc.links.insert(link);
                            acc.pins.extend(pins);
                            vec![TargetRec::Cross(cross)]
                        })
                    } else {
                        None
                    }
                })
                .collect();
            let drop_outside = group
                .iter()
                .zip(&replaced)
                .any(|(e, r)| r.is_some() && outside_only(e));
            let lang = a.extraction.as_ref().map(|x| x.lang.0);
            for (e, r) in group.iter().zip(replaced) {
                let paths: Vec<Arc<Path>> = match e.edge.kind {
                    turborepo_trace_core::RefKind::Embed => {
                        e.succ.iter().map(|s| cx.tracer.path_of(*s)).collect()
                    }
                    _ => Vec::new(),
                };
                let flags = e.edge.flags
                    | crate::external::read_flags(lang, e.edge.kind, paths.iter().map(|p| &**p));
                let attr = edge_attr(e.edge.kind, e.edge.phase, flags);
                let span = [e.edge.span.start, e.edge.span.end];
                if let Some(ts) = r {
                    for t in ts {
                        edges.push(EdgeRec {
                            attr,
                            span,
                            spec: spec_of(e),
                            target: t,
                        });
                    }
                    continue;
                }
                if drop_outside && outside_only(e) {
                    continue;
                }
                if let Some(u) = &e.edge.uncertainty {
                    let effect = crate::convert::effect_of(e.edge.kind);
                    match tcx.located_in_repository(&e.edge, u, &mut acc) {
                        Some(open) => push_unc(&open, line, effect, &mut uncs),
                        None => push_unc(u, line, effect, &mut uncs),
                    }
                }
                let links: Vec<PathBuf> = e
                    .links
                    .iter()
                    .map(|l| cx.tracer.path_of(*l).to_path_buf())
                    .collect();
                if e.succ.is_empty() {
                    edges.push(EdgeRec {
                        attr,
                        span,
                        spec: spec_of(e),
                        target: TargetRec::None,
                    });
                }
                for s in &e.succ {
                    let path = cx.tracer.path_of(*s);
                    edges.push(EdgeRec {
                        attr,
                        span,
                        spec: spec_of(e),
                        target: tcx.classify(&path, &links, &mut acc),
                    });
                }
            }
            // Pins live in the language's environment or the repo; recorded even where
            // reads would be dropped (they decide which version is linked).
            let pins: Vec<PathBuf> = std::mem::take(&mut acc.pins).into_iter().collect();
            let view_pins: Vec<PathBuf> = std::mem::take(&mut acc.view_pins).into_iter().collect();
            let extra = std::mem::take(&mut acc.extra);
            aliased |= g.observations.aliased || extra.aliased;
            let norm = if extra.is_empty() {
                crate::normalize::normalize(&ncx, &g.observations, memo)
            } else {
                let mut o = g.observations.clone();
                o.extend(extra);
                o.dedup();
                crate::normalize::normalize(&ncx, &o, memo)
            };
            // The package directories the group's observations went through.
            for dir in &norm.seen {
                tcx.seen(dir, &mut acc);
            }
            refs.push(PendingRef {
                rec: RefRec {
                    index: g.ref_index.unwrap_or(FILE_LEVEL),
                    line,
                    edges,
                    probes: Vec::new(),
                    memberships: Vec::new(),
                    configs: Vec::new(),
                },
                norm,
                pins: {
                    let mut p = crate::normalize::config_rels(&ncx, &pins);
                    p.extend(
                        crate::normalize::config_rels(&ncx, &view_pins)
                            .into_iter()
                            .map(|c| c + crate::normalize::VIEW_SUFFIX),
                    );
                    p
                },
            });
            // What the group's result was computed from without being an input of it:
            // sets the file names ([`crate::shard::Shard::key_groups`]).
            for set in &g.observations.key {
                let id = Arc::as_ptr(set) as usize;
                if key_seen.insert(id) {
                    let cell = Arc::clone(&*key_norms.entry(id).or_default());
                    cell.get_or_init(|| {
                        let norm = crate::normalize::normalize(&ncx, set, memo);
                        // The package directories the set's observations went through.
                        let mut through = ObsAcc::default();
                        for dir in &norm.seen {
                            tcx.seen(dir, &mut through);
                        }
                        // A set of the framework pass
                        // ([`crate::shard::KeyGroupRec::framework`]; a set the pass did not
                        // make is named by no framework group: `Fw::augment`).
                        let framework = g.ref_index == Some(crate::shard::FRAMEWORK_LEVEL);
                        (norm, through.links, framework)
                    });
                }
                keys.push(PendingKey::Fresh {
                    set: Arc::clone(set),
                });
            }
        }
        // What a lookup found for the file without making it an edge target.
        for (path, links) in reached.get(&ei).into_iter().flatten() {
            tcx.reach(path, links, &mut acc);
        }
        aliased |= acc.extra.aliased;
        let hash = a.hash.or_else(|| prehash(ei));
        if let Some(h) = a.hash {
            cx.hashes.insert(&a.path, h);
        }
        PendingFile {
            rec: FileRec {
                path: rel,
                lang: a.extraction.as_ref().map(|e| e.lang.0.to_owned()),
                hash: hash.map(split),
                extraction: a
                    .extraction
                    .as_ref()
                    .map(|e| extraction_rec_in(e, &cx.tracer)),
                refs: Vec::new(),
                uncertainties: Vec::new(),
                key_configs: Vec::new(),
                key_groups: Vec::new(),
                found: Vec::new(),
                links: Vec::new(),
                aliased,
            },
            refs,
            keys,
            uncs,
            acc,
        }
    };
    const CHUNK: usize = 32;
    let mut chunks: Vec<Vec<(usize, FileSource)>> = Vec::new();
    let mut it = files.into_iter().peekable();
    while it.peek().is_some() {
        chunks.push(it.by_ref().take(CHUNK).collect());
    }
    let pending: Vec<Vec<PendingFile>> = chunks
        .into_par_iter()
        .map(|chunk| {
            let mut memo = crate::normalize::Located::new();
            let mut key_seen = KeySeen::new();
            chunk
                .into_iter()
                .map(|(ei, src)| match src {
                    FileSource::Fresh(a) => one(&mut memo, &mut key_seen, ei, a),
                    FileSource::Kept(fi) => crate::filereuse::pending(
                        old.expect("a kept file has a previous shard"),
                        fi,
                        tcx.file_ix,
                    ),
                })
                .collect()
        })
        .collect();
    static INTERN: Span = Span::new("assemble: intern shard tables (sequential per package)");
    let _t = INTERN.enter();
    let mut tables = Tables::default();
    let mut acc = ObsAcc::default();
    let mut out_files = Vec::new();
    // What each file's edges reached and went through, until the shard's lists are
    // final.
    let mut file_obs: Vec<(BTreeSet<String>, BTreeSet<LinkRec>)> = Vec::new();
    let mut details: Vec<String> = Vec::new();
    let mut detail_ix: HashMap<String, u32> = HashMap::new();
    // The distinct sets of key observations (by the identity the resolver gave
    // each, by their index in the previous shard, and by value), and per file
    // the sets it names. They are interned after every file's groups, in the
    // order of their values: which file names a set first is no part of the
    // shard.
    let mut key_sets: Vec<KeyRecords> = Vec::new();
    let mut key_by_id: HashMap<usize, usize> = HashMap::new();
    let mut key_by_old: HashMap<u32, usize> = HashMap::new();
    let mut key_by_value: HashMap<KeyRecords, usize> = HashMap::new();
    let mut file_sets: Vec<Vec<usize>> = Vec::new();
    // The analyses' sets stay alive until every identity is resolved.
    let mut key_alive: Vec<Arc<turborepo_trace_core::Observations>> = Vec::new();
    let mut key_value = |(norm, links, fw): KeyRecords, key_sets: &mut Vec<KeyRecords>| -> usize {
        let records = (norm.canonical(), links, fw);
        let next = key_sets.len();
        *key_by_value.entry(records.clone()).or_insert_with(|| {
            key_sets.push(records);
            next
        })
    };
    for f in pending.into_iter().flatten() {
        let mut rec = f.rec;
        rec.uncertainties = f
            .uncs
            .into_iter()
            .map(|(u, scope, effect)| {
                let detail = match detail_ix.get(&u.detail) {
                    Some(&i) => i,
                    None => {
                        let i = details.len() as u32;
                        detail_ix.insert(u.detail.clone(), i);
                        details.push(u.detail);
                        i
                    }
                };
                crate::shard::FileUncRec {
                    kind: u.kind,
                    line: u.line,
                    span: u.span,
                    detail,
                    scope,
                    effect,
                    origin: u.origin,
                }
            })
            .collect();
        for r in f.refs {
            let ix = tables.intern(r.norm, r.pins);
            let mut rref = r.rec;
            rref.probes = ix.probes;
            rref.memberships = ix.memberships;
            rref.configs = ix.configs;
            rec.refs.push(rref);
        }
        let mut named = Vec::with_capacity(f.keys.len());
        for k in f.keys {
            let i = match k {
                PendingKey::Fresh { set } => {
                    let id = Arc::as_ptr(&set) as usize;
                    key_alive.push(set);
                    match key_by_id.get(&id) {
                        Some(&i) => i,
                        None => {
                            // (Every chunk is done: the records are this map's alone.)
                            let records = key_norms
                                .remove(&id)
                                .and_then(|(_, cell)| Arc::try_unwrap(cell).ok())
                                .and_then(std::sync::OnceLock::into_inner)
                                .expect("the records of a key set that a file names");
                            let i = key_value(records, &mut key_sets);
                            key_by_id.insert(id, i);
                            i
                        }
                    }
                }
                PendingKey::Kept(o) => match key_by_old.get(&o) {
                    Some(&i) => i,
                    None => {
                        let old = old.expect("a kept file has a previous shard");
                        let i = key_value(crate::filereuse::key_set(old, o), &mut key_sets);
                        key_by_old.insert(o, i);
                        i
                    }
                },
            };
            named.push(i);
        }
        file_sets.push(named);
        acc.found.extend(f.acc.found.iter().cloned());
        acc.links.extend(f.acc.links.iter().cloned());
        file_obs.push((f.acc.found, f.acc.links));
        out_files.push(rec);
    }
    // The directories the sets of key observations were located through are links
    // of the shard, named by the set.
    for (_, through, _) in &key_sets {
        acc.links.extend(through.iter().cloned());
    }
    // Per file, its own reached files and links as indexes into the shard's sorted
    // lists: what a later refresh checks before it keeps the file's records.
    let found: Vec<String> = acc.found.into_iter().collect();
    let links: Vec<LinkRec> = acc.links.into_iter().collect();
    for (rec, (ff, fl)) in out_files.iter_mut().zip(file_obs) {
        rec.found = ff
            .iter()
            .filter_map(|x| found.binary_search(x).ok())
            .map(|i| i as u32)
            .collect();
        rec.links = fl
            .iter()
            .filter_map(|x| links.binary_search(x).ok())
            .map(|i| i as u32)
            .collect();
    }
    // The sets of key observations, in the order of their values; a file names its
    // sets.
    let mut order: Vec<usize> = (0..key_sets.len()).collect();
    order.sort_unstable_by(|a, b| key_sets[*a].cmp(&key_sets[*b]));
    let mut key_groups: Vec<crate::shard::KeyGroupRec> = Vec::new();
    let mut group_ix: HashMap<crate::shard::KeyGroupRec, u32> = HashMap::new();
    let mut final_ix = vec![0u32; key_sets.len()];
    let mut key_sets: Vec<Option<KeyRecords>> = key_sets.into_iter().map(Some).collect();
    for i in order {
        let (norm, through, framework) = key_sets[i].take().expect("each set once");
        let ix = tables.intern(norm, Vec::new());
        let g = crate::shard::KeyGroupRec {
            framework,
            probes: ix.probes,
            memberships: ix.memberships,
            configs: ix.configs,
            links: through
                .iter()
                .filter_map(|x| links.binary_search(x).ok())
                .map(|i| i as u32)
                .collect(),
        };
        let next = key_groups.len() as u32;
        final_ix[i] = *group_ix.entry(g.clone()).or_insert_with(|| {
            key_groups.push(g);
            next
        });
    }
    for (rec, named) in out_files.iter_mut().zip(file_sets) {
        rec.key_groups = named.into_iter().map(|i| final_ix[i]).collect();
        rec.key_groups.sort_unstable();
        rec.key_groups.dedup();
    }
    drop(key_alive);
    Shard {
        format: shard::FORMAT_VERSION,
        key: String::new(),
        package,
        files: out_files,
        path_configs: tables.path_configs(),
        probes: tables.probes,
        memberships: tables.memberships,
        configs: tables.configs,
        dep_closures: Vec::new(),
        obs: ObsRec {
            found,
            links,
            states: Default::default(),
        },
        details,
        tests: Vec::new(),
        tasks: Vec::new(),
        hubs: Vec::new(),
        parts: Vec::new(),
        hub_members: Vec::new(),
        key_only: Vec::new(),
        key_groups,
        located: Vec::new(),
    }
}

/// Whether the repo-relative code file `rel` is a vendored third-party file of
/// a language that has a package identity for it (a Go module in `vendor/`, a
/// vendored crate, a committed virtualenv): such files are opaque package
/// members, not first-party entries.
fn is_vendored_third_party(cx: &Cx, rel: &str) -> bool {
    let abs = cx.repo.join(rel);
    if cx.env_roots.iter().any(|r| abs.starts_with(r)) {
        return true;
    }
    let vendored = rel.starts_with("vendor/") || rel.contains("/vendor/");
    let lang_file = rel.ends_with(".go") || rel.ends_with(".rs") || rel.ends_with(".py");
    (vendored && lang_file && cx.ids.language(&abs, cx.fs()).is_some())
        || cx
            .tracer
            .environment_roots()
            .iter()
            .any(|r| r.name.starts_with("python:") && abs.starts_with(&r.path))
}
