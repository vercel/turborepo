//! The shard format: one file per package in `<graph dir>/store/<key>`.
//!
//! A shard file is a 16-byte header followed by an [`rkyv`] archive of
//! [`Shard`]:
//!
//! | Bytes | Content |
//! |---|---|
//! | 0..8 | magic `DTSHARD\0` |
//! | 8..12 | [`FORMAT_VERSION`], little endian |
//! | 12..16 | reserved (0) |
//! | 16..24 | xxh3-64 of the archive, little endian |
//! | 24..32 | reserved (0) |
//! | 32.. | the archive |
//!
//! The archive is read in place (zero-copy): [`load`] copies the file into an
//! aligned buffer, checks the checksum and hands out `&ArchivedShard`. A shard
//! with another magic or format version is rejected, which makes the build
//! re-trace that package.
//!
//! Paths inside a shard are relative to the package root (`/`-separated, `..`
//! allowed to go above it), or absolute when they lie outside the repository
//! root.

use std::path::Path;

use rkyv::{Archive, Deserialize, Serialize, util::AlignedVec};

/// Version of the shard layout. Bump on any change to the types in this module.
/// (44: the key of a Python project's shard has what its package discovery
/// asked.)
pub const FORMAT_VERSION: u32 = 44;

const MAGIC: &[u8; 8] = b"DTSHARD\0";
const HEADER: usize = 32;

/// Kind of package a shard describes.
pub mod kind {
    /// A workspace package (or the repo root): traced, keyed by its contents.
    pub const FIRST_PARTY: u8 = 0;
    /// A third-party package traced with `--externals follow`.
    pub const THIRD_PARTY: u8 = 1;
    /// A third-party package kept opaque: one node, no files.
    pub const OPAQUE: u8 = 2;
}

/// One package's slice of the graph.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Shard {
    /// [`FORMAT_VERSION`] at write time.
    pub format: u32,
    /// The shard key (hex), also the file name.
    pub key: String,
    /// The package.
    pub package: PackageRec,
    /// Files, sorted by path, then the task and membership-hub pseudo-files
    /// ([`crate::tasks`]).
    pub files: Vec<FileRec>,
    /// Probe table, referenced by index from [`RefRec::probes`].
    pub probes: Vec<ProbeRec>,
    /// Membership table, referenced by index from [`RefRec::memberships`].
    pub memberships: Vec<MembershipRec>,
    /// Config inputs (files read during resolution), referenced from
    /// [`RefRec::configs`]. An entry that ends with
    /// [`crate::normalize::VIEW_SUFFIX`] is the resolution view of
    /// the manifest at that path: keyed by the view's bytes, not by the file's.
    pub configs: Vec<String>,
    /// Indexes into [`Shard::configs`] of the inputs keyed by path only,
    /// sorted: queries see them as inputs like any config, but their
    /// contents are not part of the shard key (the manifests of a task's
    /// transitive workspace dependencies; what of them matters
    /// for task resolution is covered by [`Shard::dep_closures`]).
    pub path_configs: Vec<u32>,
    /// Package roots (repo-relative, `.` for the root) whose transitive
    /// workspace-dependency closure this shard's tasks resolved (turbo
    /// `^task`), sorted. The key covers the closure's current package
    /// roots, so a manifest edit that does not change a closure (a version
    /// bump) re-keys no transitive dependent.
    pub dep_closures: Vec<String>,
    /// Further observations the key validates: files the edges reached outside
    /// this shard's file list, and third-party package links.
    pub obs: ObsRec,
    /// Test files of this shard per test framework, sorted by file then
    /// framework (see [`crate::fw`]).
    pub tests: Vec<TestRec>,
    /// Uncertainty detail texts referenced by [`FileUncRec::detail`], in
    /// first-seen order. A diagnostic shared by many files (a tsconfig
    /// warning reported for every file it governs) is stored once.
    pub details: Vec<String>,
    /// Build tasks of this package (see [`crate::tasks`]), sorted
    /// by id. Each task is also a pseudo-file in [`Shard::files`] (language
    /// `task`, path `#<id>`) whose references are the task's inputs.
    pub tasks: Vec<TaskRec>,
    /// Membership hubs of this shard's tasks, sorted by file. A hub's edges to
    /// the walked files it matches are not stored: the composed graph
    /// expands them from the snapshot's walked-file list
    /// ([`crate::manifest::Manifest::files`]), so a repo-wide hub that many
    /// packages share (`knip --directory ../..`) costs nothing per shard. Its
    /// listing (a membership of the pseudo-file) keys the shard as before.
    pub hubs: Vec<HubRec>,
    /// Task parts of this shard's tasks
    /// ([`turborepo_trace_tasks::spec::TaskPart`]: the tsconfig projects of
    /// `tsc -b` builds), sorted by file. Each is a pseudo-file in
    /// [`Shard::files`] (language `task-part`, path `#<kind>:<path>`) whose
    /// references are the part's inputs. A part that several shards define
    /// (every `tsc -b` that builds the project) has one node in the
    /// composed graph.
    pub parts: Vec<PartRec>,
    /// Indexes into [`Shard::memberships`] of the listings that only membership
    /// hub pseudo-files reference, sorted. The shard stores nothing of such
    /// a listing but its hubs' [`HubRec::skip`] lists (the composed graph
    /// expands a hub's edges from the snapshot's walked-file list), so the
    /// key covers what the hubs keep of it
    /// ([`crate::tasks::Tasks::hub_state`]) and not the listing: a file added
    /// or removed somewhere in a repository-wide hub's directory re-keys no
    /// shard.
    pub hub_members: Vec<u32>,
    /// Indexes into [`Shard::memberships`] of the listings of a subtree that no
    /// reference of a file's own reads, only sets of key observations
    /// ([`Shard::key_groups`]: what the load of a config enumerated, such
    /// as the haste map of a jest project or the directories a `projects`
    /// glob matches, for every file below the config), sorted. The key takes
    /// such a listing's digest directly, one per directory and pattern per
    /// build, instead of through the shard's tree part: a tree part that
    /// held a project-wide listing would depend on every directory of the
    /// project, in every package of it.
    pub key_only: Vec<u32>,
    /// Sets of **key observations** ([`turborepo_trace_core::record::key`]),
    /// each stored once and named by the files that depend on it
    /// ([`FileRec::key_groups`]), in first-seen order over the files: what
    /// a file's records were computed from without being an input of
    /// the file (the project references a `tsconfig.json` above it lists:
    /// whether each loads and which files it includes). The shard's key
    /// covers their table entries like any other, a refresh traces a file
    /// again when an entry of one of its sets changed
    /// its state ([`crate::filereuse::plan`]), and the composed graph never
    /// reads them, so no query selects through them.
    pub key_groups: Vec<KeyGroupRec>,
    /// The place of the checkout as an input of the test discovery of the
    /// projects that the shard's files are tests of or are governed by
    /// ([`crate::located`]), sorted.
    pub located: Vec<LocatedRec>,
}

/// A project whose runner matches the absolute path of a file (jest), for the
/// shards of the files its config is consulted for: whether the place of the
/// checkout decides which files are its tests ([`crate::located`]).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocatedRec {
    /// The project's config file (repo-relative).
    pub config: String,
    /// The project's discovery as its config writes it, without the place
    /// ([`turborepo_trace_frameworks::location::ProjectSpec::to_text`]).
    pub spec: String,
    /// Empty when the place of the checkout decides nothing for the project:
    /// the record is the same wherever the repository is. Otherwise the
    /// absolute root the shard was traced at: there, files are tests that
    /// are none elsewhere (or the other way round), or the discovery cannot
    /// be kept without the place. The shard's key covers the state the spec
    /// has at the root of the build that asks.
    pub root: String,
}

/// One set of key observations ([`Shard::key_groups`]).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyGroupRec {
    /// Whether the set is one of the framework pass ([`crate::fw`]: what the
    /// load of a runner config, or the walk that finds one, accessed). Such
    /// a set only says when a file is traced again. The other sets (what
    /// the load of a referenced tsconfig project read) are also what a
    /// query that compares two revisions asks about
    /// ([`crate::Graph::key_observers`]): which project governed a file at the
    /// base.
    pub framework: bool,
    /// Indexes into [`Shard::probes`], sorted.
    pub probes: Vec<u32>,
    /// Indexes into [`Shard::memberships`], sorted.
    pub memberships: Vec<u32>,
    /// Indexes into [`Shard::configs`], sorted.
    pub configs: Vec<u32>,
    /// Indexes into [`ObsRec::links`]: the package directories the set's
    /// observations were located through, sorted (where each leads is a
    /// state of the set).
    pub links: Vec<u32>,
}

/// A task part: the outputs it alone produces. Files they cover link back to
/// the part's node instead of the tasks that contain the part.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PartRec {
    /// Index of the part's pseudo-file in [`Shard::files`].
    pub file: u32,
    /// Outputs (as [`TaskRec::outputs`]).
    pub outputs: Vec<TaskOutputRec>,
}

/// A membership hub pseudo-file ([`crate::tasks`]): its edges are every walked
/// file below `dir` that matches `pattern` and no `exclude` glob, except
/// `skip`.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HubRec {
    /// Index of the hub's pseudo-file in [`Shard::files`].
    pub file: u32,
    /// Directory, repo-relative (`""` for the root).
    pub dir: String,
    /// Glob relative to `dir`.
    pub pattern: String,
    /// Excluded globs relative to `dir`.
    pub exclude: Vec<String>,
    /// Matching walked files (repo-relative, sorted) whose edges the
    /// pseudo-file stores itself: third-party files (vendored packages),
    /// linked to their package node.
    pub skip: Vec<String>,
    /// Only the own files of `dir`'s package: files inside a package nested
    /// below `dir` are not members ([`crate::own`]).
    pub own: bool,
}

/// A build task: what is not a file dependency (those are the edges, probes,
/// memberships and configs of its pseudo-file).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TaskRec {
    /// Index of the task's pseudo-file in [`Shard::files`].
    pub file: u32,
    /// The task id (`task:apps/web#build`).
    pub id: String,
    /// Display name (`build`).
    pub name: String,
    /// Task sources that contributed (`npm`, `turbo`).
    pub sources: Vec<String>,
    /// Command lines as written.
    pub lines: Vec<String>,
    /// Per-command recognition.
    pub commands: Vec<TaskCommandRec>,
    /// Outputs: generated files link back to this task.
    pub outputs: Vec<TaskOutputRec>,
    /// Environment variable names.
    pub env: Vec<String>,
    /// Toolchain packages as `<ecosystem>:<name>`.
    pub toolchain: Vec<String>,
    /// Task ids this task depends on, as resolved at build time (display; the
    /// edges of the pseudo-file are authoritative).
    pub deps: Vec<String>,
    /// Uncertainties as `<kind>: <detail>` (the pseudo-file records them as
    /// `unknown_command` / `shell_unsupported`, which are covered by a package
    /// membership).
    pub uncertainties: Vec<String>,
    /// The tests the task executes (`turborepo_trace_tasks::runs`), encoded by
    /// [`crate::tasks::encode_run`]: `tests:<framework>:<dir>`, `script:<package>#<name>`,
    /// `id:<task id>`, `unknown:<reason>`, `computed:<reason>`, and the task's
    /// definition files as `def:<path>` ([`crate::tasks::encode_runs`]).
    /// Not a dependency of the task: no edge stands for it.
    pub runs: Vec<String>,
}

/// One command of a task.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TaskCommandRec {
    /// The command text.
    pub text: String,
    /// The program after wrapper stripping.
    pub program: Option<String>,
    /// The recognizer id.
    pub recognizer: Option<String>,
    /// 0 full, 1 partial, 2 ignored, 3 unknown.
    pub recognition: u8,
}

/// A task output: a package-relative path, optionally a glob below it.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TaskOutputRec {
    /// File or directory, package-relative (absolute outside the repository).
    pub path: String,
    /// Glob relative to `path`.
    pub pattern: Option<String>,
    /// Excluded globs relative to `path`.
    pub exclude: Vec<String>,
    /// For a location of an output package (`<dir>/node_modules/<name>`): the
    /// npm package name. Also links the package's opaque node, and bare
    /// imports of it.
    pub package: Option<String>,
}

/// A file of the shard that a test framework runs as a test.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TestRec {
    /// Index into [`Shard::files`].
    pub file: u32,
    /// Framework id (`jest`, `vitest`, `pytest`, `gotest`, `cargo-test`).
    pub framework: String,
    /// The working directories the framework may run this test in
    /// (repo-relative, `""` for the root; sorted): what `process.cwd()`
    /// returns in the test and the helpers it loads. For jest and vitest:
    /// the directory of the project's config file, its `rootDir` /
    /// `root`, and the directory of the config that lists the project. Empty
    /// when unknown. The composed graph resolves working-directory-relative
    /// references in a test's closure against them
    /// ([`crate::Graph::cwd_patch`]).
    pub cwds: Vec<String>,
}

/// Package identity.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PackageRec {
    /// One of [`kind`].
    pub kind: u8,
    /// Package name (`@scope/name`; for unnamed workspace packages, the root
    /// path or `.`).
    pub name: String,
    /// Resolved version; `None` for first-party packages.
    pub version: Option<String>,
    /// Integrity from the lockfile, when known.
    pub integrity: Option<String>,
    /// Package root relative to the repository root (`""` for the root
    /// package). For traced third-party packages, the real path of the
    /// installed package; empty for opaque packages (the manifest entry
    /// records where it is installed).
    pub root: String,
}

/// One file of a traced package.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct FileRec {
    /// Path relative to the package root.
    pub path: String,
    /// Language id, for source files.
    pub lang: Option<String>,
    /// xxh3-128 of the contents as `[low, high]`.
    pub hash: Option<[u64; 2]>,
    /// The extraction, reused when the contents are unchanged.
    pub extraction: Option<ExtractionRec>,
    /// Resolved references in extraction order, then one file-level group.
    pub refs: Vec<RefRec>,
    /// Uncertainties on this file (diagnostics and edge uncertainties,
    /// deduplicated).
    pub uncertainties: Vec<FileUncRec>,
    /// Config inputs that decide this file's records without being its query
    /// inputs (the framework configs governing it, [`crate::fw`]): indexes
    /// into [`Shard::configs`], sorted. With [`FileRec::found`] and
    /// [`FileRec::links`], what a refresh needs to tell whether this file's
    /// records still hold when its package is re-keyed
    /// ([`crate::filereuse`]).
    pub key_configs: Vec<u32>,
    /// The sets of key observations this file's records depend on: indexes into
    /// [`Shard::key_groups`], sorted.
    pub key_groups: Vec<u32>,
    /// Indexes into [`ObsRec::found`]: the files outside the shard's file list
    /// that this file's edges reached, sorted.
    pub found: Vec<u32>,
    /// Indexes into [`ObsRec::links`]: the package links this file's edges went
    /// through, sorted.
    pub links: Vec<u32>,
    /// Whether a `tsconfig.json` or `jsconfig.json` with `paths`, `baseUrl` or
    /// `rootDirs` governs the file, applied or not
    /// ([`turborepo_trace_core::record::aliased`]): below a
    /// directory called `node_modules` the resolver does not apply them, so
    /// where the checkout lies decides the file's records. The shard's key
    /// and the observation states then take whether the checkout lies below
    /// one ([`crate::fw::place_state`]).
    pub aliased: bool,
}

/// One reference and what its resolution produced and observed.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RefRec {
    /// Index of the reference in the extraction, or [`FILE_LEVEL`].
    pub index: u32,
    /// 1-based line, 0 if unknown.
    pub line: u32,
    /// Edges, one per concrete target (globs and directories fan out).
    pub edges: Vec<EdgeRec>,
    /// Indexes into [`Shard::probes`].
    pub probes: Vec<u32>,
    /// Indexes into [`Shard::memberships`].
    pub memberships: Vec<u32>,
    /// Indexes into [`Shard::configs`].
    pub configs: Vec<u32>,
}

/// [`RefRec::index`] of the file-level group.
pub const FILE_LEVEL: u32 = u32::MAX;

/// [`RefRec::index`] of a group contributed by a test framework adapter: a
/// test's framework inputs, or the module-mapper and mock edges of a file
/// loaded by tests.
pub const FRAMEWORK_LEVEL: u32 = u32::MAX - 1;

/// One edge.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EdgeRec {
    /// Kind, phase and flags packed as [`turborepo_trace_graph::EdgeAttr`].
    pub attr: u16,
    /// Byte span of the reference.
    pub span: [u32; 2],
    /// The specifier as written.
    pub spec: String,
    /// Where it points.
    pub target: TargetRec,
}

/// Edge target in symbolic form.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TargetRec {
    /// Unresolved, a builtin, or nothing concrete.
    None,
    /// A file of this shard, by index into [`Shard::files`].
    File(u32),
    /// A non-traced file inside this package (an asset, a manifest),
    /// package-relative.
    Intra(String),
    /// A file of another package.
    Cross(CrossRec),
    /// A file outside the repository and outside any package (absolute).
    Outside(String),
}

/// A cross-package edge target: `(target package name, resolved version,
/// subpath)`.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CrossRec {
    /// Target package name.
    pub name: String,
    /// Resolved version, or `None` for a workspace package.
    pub version: Option<String>,
    /// Path relative to the target package root.
    pub subpath: String,
}

/// A probe: one or more missing paths, compacted.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProbeRec {
    /// One missing path.
    Path(String),
    /// Extension and index probing: one path stem with suffixes.
    Stem(StemRec),
    /// A `node_modules` walk-up.
    WalkUp(WalkUpRec),
}

/// `path + s` for each suffix `s` (`""` stands for `path` itself).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StemRec {
    /// Common prefix.
    pub path: String,
    /// Suffixes, sorted.
    pub suffixes: Vec<String>,
}

/// For `levels` directories starting at `dir` and going up,
/// `<d>/<marker>/<name>` and `<d>/<marker>/<name><s>` for each suffix are
/// missing.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WalkUpRec {
    /// Deepest directory.
    pub dir: String,
    /// Number of directories (`dir` and its ancestors).
    pub levels: u32,
    /// Marker directory name (`node_modules`).
    pub marker: String,
    /// Package name.
    pub name: String,
    /// Suffixes probed next to the package directory (`.js`, ...), sorted.
    pub suffixes: Vec<String>,
}

/// A directory membership: the listing of `dir` filtered by `pattern` (store
/// glob syntax).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MembershipRec {
    /// Directory, package-relative.
    pub dir: String,
    /// Glob relative to `dir`.
    pub pattern: String,
}

/// An uncertainty.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UncRec {
    /// Index into [`crate::convert::UNCERTAINTY_KINDS`].
    pub kind: u8,
    /// 1-based line, 0 if unknown.
    pub line: u32,
    /// Byte span.
    pub span: [u32; 2],
    /// Detail text.
    pub detail: String,
    /// Index into [`turborepo_trace_core::Origin::ALL`].
    pub origin: u8,
}

/// An uncertainty of a shard file: [`UncRec`] with its detail interned in
/// [`Shard::details`].
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileUncRec {
    /// Index into [`crate::convert::UNCERTAINTY_KINDS`].
    pub kind: u8,
    /// 1-based line, 0 if unknown.
    pub line: u32,
    /// Byte span.
    pub span: [u32; 2],
    /// Index into [`Shard::details`].
    pub detail: u32,
    /// The scope of a covered uncertainty
    /// ([`turborepo_trace_core::Uncertainty::scope`]), or `None` for an
    /// open one.
    pub scope: Option<ScopeRec>,
    /// What the reference that raised it does with its target: [`EFFECT_NONE`]
    /// (not raised by a reference), [`EFFECT_LOAD`] or [`EFFECT_READ`].
    pub effect: u8,
    /// Index into [`turborepo_trace_core::Origin::ALL`].
    pub origin: u8,
}

/// [`FileUncRec::effect`]: not raised by a reference.
pub const EFFECT_NONE: u8 = 0;
/// [`FileUncRec::effect`]: the reference loads its target as code.
pub const EFFECT_LOAD: u8 = 1;
/// [`FileUncRec::effect`]: the reference reads its target's contents.
pub const EFFECT_READ: u8 = 2;

/// The scope of a covered uncertainty: every file below `dir` matching
/// `pattern`.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeRec {
    /// Directory, repo-relative (`""` for the root).
    pub dir: String,
    /// Glob relative to `dir` (query glob syntax).
    pub pattern: String,
}

/// Observations validated by the key beyond probes, memberships and configs.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ObsRec {
    /// Edge targets that are not files of this shard and must still exist
    /// (package-relative or absolute).
    pub found: Vec<String>,
    /// Third-party packages this shard's edges reached.
    pub links: Vec<LinkRec>,
    /// The state of every observation when the shard was traced.
    pub states: StatesRec,
}

/// The state of each observation of a shard at trace time, per table entry
/// ([`crate::filereuse::states`]): 64 bits of the digest the shard key hashes
/// for it. A refresh that finds another key compares them with the current
/// states to learn **which** observations changed, and re-traces only the files
/// whose own references read one of those ([`crate::filereuse`]).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct StatesRec {
    /// Per probe of [`Shard::probes`] with something present (an expanded path
    /// or a guard directory): index and state, sorted by index. Every other
    /// probe has state 0.
    pub probes: Vec<ProbeStateRec>,
    /// Per membership of [`Shard::memberships`]: its listing (0 for a listing
    /// only hubs reference, [`Shard::hub_members`]).
    pub memberships: Vec<u64>,
    /// Per config of [`Shard::configs`]: its contents (existence for a
    /// path-only input).
    pub configs: Vec<u64>,
    /// Per file of [`ObsRec::found`]: present, and its owning package.
    pub found: Vec<u64>,
    /// Per link of [`ObsRec::links`]: the identity found there.
    pub links: Vec<u64>,
    /// The package's framework config paths
    /// ([`crate::fw::ConfigPaths::digest_for`]).
    pub fw: [u64; 2],
}

/// The state of one probe that is not 0 ([`StatesRec::probes`]).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProbeStateRec {
    /// Index into [`Shard::probes`].
    pub probe: u32,
    /// The digest of the expanded paths and guard directories that exist.
    pub state: u64,
}

/// A third-party package as seen from this shard: the directory resolution
/// found it at (a `node_modules/<name>` link or directory) and its identity
/// there.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkRec {
    /// Where the package was found, package-relative
    /// (`../../node_modules/react`).
    pub location: String,
    /// Package name from its `package.json`.
    pub name: String,
    /// Version from its `package.json`.
    pub version: String,
    /// Integrity from the lockfile.
    pub integrity: Option<String>,
    /// The package's real root, repo-relative (absolute outside the
    /// repository).
    pub root: String,
}

/// A cached extraction.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExtractionRec {
    /// Language id.
    pub lang: String,
    /// References.
    pub refs: Vec<RawRefRec>,
    /// File-level diagnostics.
    pub diagnostics: Vec<UncRec>,
    /// Facts for other files' analyses (`turborepo_trace_core::RefSink::fact`),
    /// as tagged strings.
    pub facts: Vec<String>,
}

/// A raw reference.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RawRefRec {
    /// Index into `RefKind::ALL`.
    pub kind: u8,
    /// The specifier in prefix form (see [`SpecTok`]).
    pub spec: Vec<SpecTok>,
    /// The base.
    pub base: BaseRec,
    /// Byte span.
    pub span: [u32; 2],
    /// `RefFlags` bits.
    pub flags: u16,
    /// 0 runtime, 1 build.
    pub phase: u8,
    /// Extractor uncertainty.
    pub uncertainty: Option<UncRec>,
    /// 1-based line.
    pub line: u32,
}

/// A specifier token, in prefix order: `Alternatives` is `Alt(n)` followed by
/// `n` specifiers, `Pattern` is `Pattern(n)` followed by `n` parts
/// (`Lit`/`Wild`).
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SpecTok {
    /// `Specifier::Literal`.
    Literal(String),
    /// `Specifier::Pattern` with this many parts.
    Pattern(u32),
    /// `Part::Lit`.
    Lit(String),
    /// `Part::Wild` with its reason.
    Wild(String),
    /// `Specifier::Alternatives` with this many alternatives.
    Alt(u32),
    /// `Specifier::Unknown`.
    Unknown,
}

/// A reference base.
#[derive(Archive, Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum BaseRec {
    /// `Base::FileDir`.
    FileDir,
    /// `Base::Cwd`.
    Cwd,
    /// `Base::ProjectRoot`.
    ProjectRoot,
    /// `Base::Dir`, as an absolute path.
    Dir(String),
    /// `Base::Language`.
    Language(String),
}

/// Errors reading a shard.
#[derive(Debug, thiserror::Error)]
pub enum ShardError {
    /// I/O.
    #[error("{0}: {1}")]
    Io(String, std::io::Error),
    /// Not a shard of this format.
    #[error("{0}: not a shard of format {FORMAT_VERSION}")]
    Format(String),
    /// Checksum mismatch.
    #[error("{0}: checksum mismatch (corrupt shard)")]
    Checksum(String),
    /// The archive does not validate.
    #[error("{0}: invalid archive: {1}")]
    Archive(String, String),
}

/// Serializes a shard into the file format.
// Invariant: serializing our own types into memory cannot fail.
#[allow(clippy::expect_used)]
pub fn encode(shard: &Shard) -> Vec<u8> {
    let archive = rkyv::to_bytes::<rkyv::rancor::Error>(shard).expect("shard serialization");
    let mut out = Vec::with_capacity(HEADER + archive.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(&archive).to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&archive);
    out
}

/// Shard bytes: an aligned heap buffer, or a read-only map of a pack file or
/// log (page aligned; the files are only ever replaced by rename or appended
/// to, and a log's torn tail beyond every indexed record is the only part ever
/// cut off).
pub(crate) enum Buf {
    Heap(AlignedVec),
    Map(memmap2::Mmap),
}

impl std::ops::Deref for Buf {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            Buf::Heap(v) => v,
            Buf::Map(m) => m,
        }
    }
}

/// An archived shard in an aligned buffer (its own, or a range of a [`Pack`]).
pub struct LoadedShard {
    buf: std::sync::Arc<Buf>,
    start: usize,
    end: usize,
}

impl std::fmt::Debug for LoadedShard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedShard")
            .field("bytes", &self.len())
            .finish()
    }
}

impl LoadedShard {
    /// The archived shard.
    #[inline]
    pub fn get(&self) -> &ArchivedShard {
        // SAFETY: `decode` validated the archive (checked access in debug builds, the
        // checksum of bytes we wrote ourselves in release builds), the range starts at
        // a 16-byte aligned offset of an aligned buffer, and the buffer is
        // never mutated afterwards.
        unsafe { rkyv::access_unchecked::<ArchivedShard>(&self.buf[self.start..self.end]) }
    }

    /// Deserializes into an owned [`Shard`].
    // Invariant: the archive was validated when the shard was opened.
    #[allow(clippy::expect_used)]
    pub fn to_owned(&self) -> Shard {
        rkyv::deserialize::<Shard, rkyv::rancor::Error>(self.get()).expect("validated shard")
    }

    /// Archive size in bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the archive is empty (never, for a valid shard).
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The archive's bytes.
    pub fn archive(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    /// The shard in the file format (header and archive).
    pub fn file_bytes(&self) -> Vec<u8> {
        let archive = &self.buf[self.start..self.end];
        let mut out = Vec::with_capacity(HEADER + archive.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&xxhash_rust::xxh3::xxh3_64(archive).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(archive);
        out
    }
}

/// Checks the header and checksum of shard file bytes; the archive's range in
/// `bytes`.
// Invariant: a fixed-width range of the slice converts to an array of that width.
#[allow(clippy::expect_used)]
fn check(bytes: &[u8], name: &str) -> Result<std::ops::Range<usize>, ShardError> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC || bytes[8..12] != FORMAT_VERSION.to_le_bytes()
    {
        return Err(ShardError::Format(name.into()));
    }
    let sum = u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes"));
    if xxhash_rust::xxh3::xxh3_64(&bytes[HEADER..]) != sum {
        return Err(ShardError::Checksum(name.into()));
    }
    Ok(HEADER..bytes.len())
}

fn validate(s: LoadedShard, name: &str) -> Result<LoadedShard, ShardError> {
    if cfg!(debug_assertions) {
        rkyv::access::<ArchivedShard, rkyv::rancor::Error>(&s.buf[s.start..s.end])
            .map_err(|e| ShardError::Archive(name.into(), e.to_string()))?;
    }
    Ok(s)
}

/// Parses shard file bytes.
pub fn decode(bytes: &[u8], name: &str) -> Result<LoadedShard, ShardError> {
    let r = check(bytes, name)?;
    let archive = &bytes[r];
    let mut buf = AlignedVec::with_capacity(archive.len());
    buf.extend_from_slice(archive);
    let end = buf.len();
    validate(
        LoadedShard {
            buf: std::sync::Arc::new(Buf::Heap(buf)),
            start: 0,
            end,
        },
        name,
    )
}

/// Reads and parses a shard file. A store path `<store>/<key>` with no such
/// file (the shard lives in the store's pack, [`Pack`]) is read from the pack.
pub fn load(path: &Path) -> Result<LoadedShard, ShardError> {
    let name = path.display().to_string();
    match std::fs::read(path) {
        Ok(bytes) => decode(&bytes, &name),
        Err(e) => {
            let packed = path
                .parent()
                .zip(path.file_name().and_then(|n| n.to_str()))
                .and_then(|(store, key)| Pack::open(store)?.get(key));
            packed.ok_or(ShardError::Io(name, e))
        }
    }
}

/// Loads shard `key` from `pack` if it holds it, else from `<store>/<key>`.
pub fn load_from(store: &Path, pack: Option<&Pack>, key: &str) -> Result<LoadedShard, ShardError> {
    if let Some(s) = pack.and_then(|p| p.get(key)) {
        return Ok(s);
    }
    load(&store.join(key))
}

/// The shards of a store in two files, `<store>/pack` and the append log
/// `<store>/pack.log`, so a load opens two files instead of thousands (endpoint
/// security on macOS serializes file opens) and a build writes one. Shard keys
/// are content addresses, so the pack can never hold a stale version of a
/// shard; every entry keeps its header and checksum, and shards it lacks are
/// read from loose files `<store>/<key>` (older stores). Archives sit
/// at 16-byte aligned offsets of aligned buffers and are accessed in place.
///
/// A build appends the shards it wrote to the log ([`Pack::append`]); loading
/// compacts the log into the pack when it grows ([`Pack::compact`]), and `gc`
/// rewrites the pack with only the referenced shards.
///
/// The order of the log's records is the order in which the build's threads
/// finished their packages: the log's bytes are not a function of the tree, and
/// two builds of one tree may leave logs that differ. Nothing reads a shard by
/// its position: every reader goes through the index (key to range), a key
/// occurs once in the log a build writes, and the pack file and a packed graph
/// ([`crate::artifact`]) hold the shards in the manifest's order. What is the
/// same for every build of a tree is the set of keys and the bytes under each
/// key (the command tests check it).
///
/// The pack file:
///
/// | Bytes | Content |
/// |---|---|
/// | 0..8 | magic `DTPACK1\n` |
/// | 8..16 | entry count, little endian |
/// | then | per entry: key length (u32), key, offset (u64), length (u64) of the shard file bytes |
/// | offsets | the shard files (header and archive), each archive 16-byte aligned |
///
/// The log: magic `DTPLOG1\n`, then per record: key length (u32), key, length
/// (u64) of the shard file bytes, zero padding to a 16-byte file offset, the
/// shard file bytes. A torn final record (an interrupted append) is ignored.
pub struct Pack {
    /// The pack file, then the log.
    bufs: Vec<std::sync::Arc<Buf>>,
    /// Key to (buffer, offset, length of the shard file bytes).
    index: std::collections::HashMap<String, (u8, usize, usize)>,
    /// Bytes of the pack file and of the log.
    sizes: [u64; 2],
    /// End of the log's last complete record (0 without a log).
    log_end: usize,
}

const PACK_MAGIC: &[u8; 8] = b"DTPACK1\n";
const LOG_MAGIC: &[u8; 8] = b"DTPLOG1\n";
/// File name of the pack in the store directory.
pub const PACK_FILE: &str = "pack";
/// File name of the pack's append log in the store directory.
pub const LOG_FILE: &str = "pack.log";

/// Maps a pack file or log read-only (a heap copy where mapping fails, or for
/// empty files).
fn map_file(path: &Path) -> Option<Buf> {
    let f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    if len > 0 {
        // SAFETY: store files are written by rename (pack) or appended to (log); the
        // only truncation cuts a log's torn tail, beyond every record a reader
        // indexed.
        if let Ok(m) = unsafe { memmap2::Mmap::map(&f) } {
            return Some(Buf::Map(m));
        }
    }
    read_aligned(path).map(Buf::Heap)
}

fn read_aligned(path: &Path) -> Option<AlignedVec> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len() as usize;
    let mut buf = AlignedVec::with_capacity(len);
    buf.resize(len, 0);
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Parses the index of a pack file.
fn pack_index(buf: &[u8]) -> Option<Vec<(String, usize, usize)>> {
    let len = buf.len();
    if len < 16 || &buf[..8] != PACK_MAGIC {
        return None;
    }
    let n = u64::from_le_bytes(buf[8..16].try_into().ok()?) as usize;
    let mut pos = 16;
    let mut out = Vec::with_capacity(n.min(1 << 20));
    for _ in 0..n {
        let kl = u32::from_le_bytes(buf.get(pos..pos + 4)?.try_into().ok()?) as usize;
        pos += 4;
        let key = std::str::from_utf8(buf.get(pos..pos + kl)?)
            .ok()?
            .to_owned();
        pos += kl;
        let off = u64::from_le_bytes(buf.get(pos..pos + 8)?.try_into().ok()?) as usize;
        let l = u64::from_le_bytes(buf.get(pos + 8..pos + 16)?.try_into().ok()?) as usize;
        pos += 16;
        if off.checked_add(l)? > len || !(off + HEADER).is_multiple_of(16) {
            return None;
        }
        out.push((key, off, l));
    }
    Some(out)
}

/// Parses the records of a log; stops at a torn or malformed record. Also
/// returns the end of the last complete record (0 for a missing or foreign
/// header).
fn log_index(buf: &[u8]) -> (Vec<(String, usize, usize)>, usize) {
    let mut out = Vec::new();
    if buf.len() < 8 || &buf[..8] != LOG_MAGIC {
        return (out, 0);
    }
    let mut pos = 8;
    loop {
        let rec = (|| {
            let kl = u32::from_le_bytes(buf.get(pos..pos + 4)?.try_into().ok()?) as usize;
            let key = std::str::from_utf8(buf.get(pos + 4..pos + 4 + kl)?).ok()?;
            let at = pos + 4 + kl;
            let l = u64::from_le_bytes(buf.get(at..at + 8)?.try_into().ok()?) as usize;
            let off = (at + 8).next_multiple_of(16);
            (off.checked_add(l)? <= buf.len()).then(|| (key.to_owned(), off, l))
        })();
        let Some((key, off, l)) = rec else { break };
        pos = off + l;
        out.push((key, off, l));
    }
    (out, pos)
}

impl Pack {
    /// Opens the pack and log of a store directory (`None` if it has neither).
    pub fn open(store: &Path) -> Option<Pack> {
        let main = map_file(&store.join(PACK_FILE));
        let log = map_file(&store.join(LOG_FILE));
        if main.is_none() && log.is_none() {
            return None;
        }
        let mut index = std::collections::HashMap::new();
        let mut bufs = Vec::new();
        let mut sizes = [0u64; 2];
        if let Some(b) = main {
            if let Some(ix) = pack_index(&b) {
                sizes[0] = b.len() as u64;
                for (k, off, l) in ix {
                    index.insert(k, (0u8, off, l));
                }
            }
            bufs.push(std::sync::Arc::new(b));
        } else {
            bufs.push(std::sync::Arc::new(Buf::Heap(AlignedVec::new())));
        }
        let mut log_end = 0;
        if let Some(b) = log {
            sizes[1] = b.len() as u64;
            let (ix, end) = log_index(&b);
            log_end = end;
            for (k, off, l) in ix {
                index.entry(k).or_insert((1u8, off, l));
            }
            bufs.push(std::sync::Arc::new(b));
        }
        Some(Pack {
            bufs,
            index,
            sizes,
            log_end,
        })
    }

    /// The keys held by the store's pack and log and the two files' sizes,
    /// reading only their indexes and record headers.
    pub fn index_only(store: &Path) -> (std::collections::HashSet<String>, [u64; 2]) {
        use std::io::{Read, Seek, SeekFrom};
        let mut keys = std::collections::HashSet::new();
        let mut sizes = [0u64; 2];
        let read_at = |f: &mut std::fs::File, at: u64, n: usize| -> Option<Vec<u8>> {
            let mut b = vec![0; n];
            f.seek(SeekFrom::Start(at)).ok()?;
            f.read_exact(&mut b).ok()?;
            Some(b)
        };
        if let Ok(mut f) = std::fs::File::open(store.join(PACK_FILE)) {
            let len = f.metadata().map_or(0, |m| m.len());
            let ok = (|| {
                let head = read_at(&mut f, 0, 16)?;
                if &head[..8] != PACK_MAGIC {
                    return None;
                }
                let n = u64::from_le_bytes(head[8..16].try_into().ok()?) as usize;
                let mut pos = 16u64;
                let mut found = Vec::with_capacity(n.min(1 << 20));
                for _ in 0..n {
                    let kl = u32::from_le_bytes(read_at(&mut f, pos, 4)?.try_into().ok()?);
                    let key = String::from_utf8(read_at(&mut f, pos + 4, kl as usize)?).ok()?;
                    found.push(key);
                    pos += 4 + kl as u64 + 16;
                    if pos > len {
                        return None;
                    }
                }
                Some(found)
            })();
            if let Some(found) = ok {
                sizes[0] = len;
                keys.extend(found);
            }
        }
        if let Ok(mut f) = std::fs::File::open(store.join(LOG_FILE)) {
            let len = f.metadata().map_or(0, |m| m.len());
            sizes[1] = len;
            if read_at(&mut f, 0, 8).is_some_and(|m| m == LOG_MAGIC) {
                let mut pos = 8u64;
                while let Some(kl) = read_at(&mut f, pos, 4)
                    .and_then(|b| b.try_into().ok())
                    .map(u32::from_le_bytes)
                {
                    let Some(key) = read_at(&mut f, pos + 4, kl as usize)
                        .and_then(|b| String::from_utf8(b).ok())
                    else {
                        break;
                    };
                    let at = pos + 4 + kl as u64;
                    let Some(l) = read_at(&mut f, at, 8)
                        .and_then(|b| b.try_into().ok())
                        .map(u64::from_le_bytes)
                    else {
                        break;
                    };
                    let off = (at + 8).next_multiple_of(16);
                    if off + l > len {
                        break;
                    }
                    keys.insert(key);
                    pos = off + l;
                }
            }
        }
        (keys, sizes)
    }

    /// End of the log's last complete record, for [`Pack::append`].
    pub fn log_end(&self) -> usize {
        self.log_end
    }

    /// Bytes of the pack file and of its log.
    pub fn sizes(&self) -> [u64; 2] {
        self.sizes
    }

    /// Number of shards held.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the pack holds no shard.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Appends shard file bytes `(key, bytes)` to the store's log in one write
    /// (creating it as needed). `valid_end` is the end of the log's last
    /// complete record when known ([`Pack::log_end`]); otherwise the log is
    /// read to find it. A torn tail is cut off first, so records after it
    /// stay readable.
    pub fn append(
        store: &Path,
        shards: &[(String, Vec<u8>)],
        valid_end: Option<usize>,
    ) -> std::io::Result<()> {
        use std::io::{Seek, Write};
        if shards.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(store)?;
        let path = store.join(LOG_FILE);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        let len = f.seek(std::io::SeekFrom::End(0))? as usize;
        let valid = match valid_end {
            Some(e) if e <= len => e,
            _ => read_aligned(&path).map_or(0, |b| log_index(&b).1),
        };
        if valid != len {
            f.set_len(valid as u64)?;
        }
        let mut end = f.seek(std::io::SeekFrom::Start(valid as u64))? as usize;
        let mut out = Vec::new();
        if end < LOG_MAGIC.len() {
            // New (or a torn header): start over.
            f.set_len(0)?;
            f.seek(std::io::SeekFrom::Start(0))?;
            end = 0;
            out.extend_from_slice(LOG_MAGIC);
        }
        for (k, bytes) in shards {
            out.extend_from_slice(&(k.len() as u32).to_le_bytes());
            out.extend_from_slice(k.as_bytes());
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            let at = end + out.len();
            out.resize(out.len() + (at.next_multiple_of(16) - at), 0);
            out.extend_from_slice(bytes);
        }
        f.write_all(&out)
    }

    /// Rewrites the store's pack with exactly `shards` and removes the log and
    /// the loose files of those shards.
    pub fn compact(store: &Path, shards: &[(&str, &LoadedShard)]) -> std::io::Result<()> {
        Pack::write(&store.join(PACK_FILE), shards)?;
        let _ = std::fs::remove_file(store.join(LOG_FILE));
        for (k, _) in shards {
            let _ = std::fs::remove_file(store.join(k));
        }
        Ok(())
    }

    /// Makes the pack file at `path` forget the shards `keys` (sorted): their
    /// index entries are renamed in place to names no key has, so the store
    /// no longer holds those keys and a build writes them again (their
    /// bytes stay until the pack is compacted). For shards that must not be
    /// reused where a pack is installed ([`crate::artifact`]). Returns how
    /// many were found.
    // Invariant: a fixed-width range of the slice converts to an array of that width.
    #[allow(clippy::expect_used)]
    pub fn forget(path: &Path, keys: &[String]) -> std::io::Result<usize> {
        use turborepo_trace_core::os::FileExt;
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let mut head = [0u8; 16];
        f.read_exact_at(&mut head, 0)?;
        if &head[..8] != PACK_MAGIC {
            return Ok(0);
        }
        let n = u64::from_le_bytes(head[8..16].try_into().expect("8 bytes"));
        let mut pos = 16u64;
        let mut found = 0;
        for _ in 0..n {
            let mut kl = [0u8; 4];
            f.read_exact_at(&mut kl, pos)?;
            let kl = u32::from_le_bytes(kl) as usize;
            let mut key = vec![0u8; kl];
            f.read_exact_at(&mut key, pos + 4)?;
            let listed = std::str::from_utf8(&key)
                .is_ok_and(|k| keys.binary_search_by(|x| x.as_str().cmp(k)).is_ok());
            if kl > 0 && listed {
                // Keys are hex digests: no key starts with `~`.
                f.write_all_at(b"~", pos + 4)?;
                found += 1;
            }
            pos += 4 + kl as u64 + 16;
        }
        Ok(found)
    }

    /// Reads a pack file alone (`None` if missing or malformed).
    pub fn read(path: &Path) -> Option<Pack> {
        let b = map_file(path)?;
        let ix = pack_index(&b)?;
        let sizes = [b.len() as u64, 0];
        Some(Pack {
            index: ix.into_iter().map(|(k, o, l)| (k, (0u8, o, l))).collect(),
            bufs: vec![std::sync::Arc::new(b)],
            sizes,
            log_end: 0,
        })
    }

    /// Whether the pack holds `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    /// Shard `key`, in place (`None` if absent or corrupt).
    pub fn get(&self, key: &str) -> Option<LoadedShard> {
        let &(b, off, len) = self.index.get(key)?;
        let buf = &self.bufs[b as usize];
        let r = check(&buf[off..off + len], key).ok()?;
        validate(
            LoadedShard {
                buf: std::sync::Arc::clone(buf),
                start: off + r.start,
                end: off + r.end,
            },
            key,
        )
        .ok()
    }

    /// Writes a pack of `shards` (key, shard) to `path`.
    pub fn write(path: &Path, shards: &[(&str, &LoadedShard)]) -> std::io::Result<()> {
        let out = Pack::bytes(shards);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::write_atomic(path, &out)
    }

    /// The bytes of a pack file holding `shards` (key, shard) in the given
    /// order: a function of the keys and the shard bytes alone
    /// ([`crate::artifact`] ships them).
    pub fn bytes(shards: &[(&str, &LoadedShard)]) -> Vec<u8> {
        let files: Vec<Vec<u8>> = {
            use rayon::prelude::*;
            shards.par_iter().map(|(_, s)| s.file_bytes()).collect()
        };
        let mut head = PACK_MAGIC.to_vec();
        head.extend_from_slice(&(shards.len() as u64).to_le_bytes());
        let index_len: usize = shards.iter().map(|(k, _)| 4 + k.len() + 16).sum();
        let mut off = (head.len() + index_len).next_multiple_of(16);
        let mut offsets = Vec::with_capacity(shards.len());
        for ((k, _), f) in shards.iter().zip(&files) {
            head.extend_from_slice(&(k.len() as u32).to_le_bytes());
            head.extend_from_slice(k.as_bytes());
            head.extend_from_slice(&(off as u64).to_le_bytes());
            head.extend_from_slice(&(f.len() as u64).to_le_bytes());
            offsets.push(off);
            off = (off + f.len()).next_multiple_of(16);
        }
        let mut out = Vec::with_capacity(off);
        out.extend_from_slice(&head);
        for (f, o) in files.iter().zip(offsets) {
            out.resize(o, 0);
            out.extend_from_slice(f);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Shard {
        Shard {
            format: FORMAT_VERSION,
            key: "k".into(),
            package: PackageRec {
                kind: kind::FIRST_PARTY,
                name: "a".into(),
                version: None,
                integrity: None,
                root: "packages/a".into(),
            },
            files: vec![FileRec {
                path: "src/a.ts".into(),
                lang: Some("js".into()),
                hash: Some([1, 2]),
                extraction: None,
                refs: vec![RefRec {
                    index: 0,
                    line: 3,
                    edges: vec![EdgeRec {
                        attr: 7,
                        span: [1, 9],
                        spec: "./b".into(),
                        target: TargetRec::Cross(CrossRec {
                            name: "react".into(),
                            version: Some("18.0.0".into()),
                            subpath: "index.js".into(),
                        }),
                    }],
                    probes: vec![0],
                    memberships: vec![],
                    configs: vec![],
                }],
                uncertainties: vec![],
                key_configs: vec![],
                key_groups: vec![0],
                found: vec![],
                links: vec![],
                aliased: false,
            }],
            probes: vec![ProbeRec::Stem(StemRec {
                path: "src/b".into(),
                suffixes: vec!["".into(), ".js".into()],
            })],
            memberships: vec![],
            configs: vec!["package.json".into(), "../b/package.json".into()],
            path_configs: vec![1],
            dep_closures: vec!["packages/a".into()],
            obs: ObsRec::default(),
            details: Vec::new(),
            tests: vec![TestRec {
                file: 0,
                framework: "jest".into(),
                cwds: vec!["packages/a".into()],
            }],
            tasks: vec![TaskRec {
                file: 0,
                id: "task:packages/a#build".into(),
                name: "build".into(),
                sources: vec!["npm".into()],
                lines: vec!["tsc".into()],
                commands: vec![TaskCommandRec {
                    text: "tsc".into(),
                    program: Some("tsc".into()),
                    recognizer: Some("tsc".into()),
                    recognition: 0,
                }],
                outputs: vec![TaskOutputRec {
                    path: "dist".into(),
                    pattern: None,
                    exclude: Vec::new(),
                    package: None,
                }],
                env: Vec::new(),
                toolchain: vec!["npm:typescript".into()],
                deps: Vec::new(),
                uncertainties: Vec::new(),
                runs: vec!["tests:jest:packages/a".into()],
            }],
            hubs: vec![HubRec {
                file: 0,
                dir: String::new(),
                pattern: "**".into(),
                exclude: vec!["dist/**".into()],
                skip: Vec::new(),
                own: false,
            }],
            parts: vec![PartRec {
                file: 0,
                outputs: vec![TaskOutputRec {
                    path: "dist".into(),
                    pattern: Some("**/*.d.*ts".into()),
                    exclude: Vec::new(),
                    package: None,
                }],
            }],
            hub_members: Vec::new(),
            key_only: Vec::new(),
            key_groups: vec![KeyGroupRec {
                framework: false,
                probes: vec![0],
                memberships: vec![],
                configs: vec![1],
                links: vec![],
            }],
            located: vec![LocatedRec {
                config: "jest.config.js".into(),
                spec: "{}".into(),
                root: String::new(),
            }],
        }
    }

    #[test]
    fn roundtrip() {
        let shard = sample();
        let bytes = encode(&shard);
        let loaded = decode(&bytes, "t").unwrap();
        assert_eq!(loaded.to_owned(), shard);
        assert_eq!(loaded.get().files[0].refs[0].line.to_native(), 3);
        assert_eq!(loaded.file_bytes(), bytes);
        // A pack holds shards in place; absent keys fall back to the store.
        let t = tempfile::tempdir().unwrap();
        let mut other = shard.clone();
        other.key = "k2".into();
        other.package.name = "bb".into();
        let other = decode(&encode(&other), "k2").unwrap();
        let pack_path = t.path().join("cache/pack");
        Pack::write(&pack_path, &[("k", &loaded), ("k2", &other)]).unwrap();
        let pack = Pack::read(&pack_path).unwrap();
        assert!(pack.contains("k2") && !pack.contains("k3"));
        assert_eq!(pack.get("k").unwrap().to_owned(), shard);
        assert_eq!(pack.get("k2").unwrap().get().package.name.as_str(), "bb");
        std::fs::write(t.path().join("k3"), &bytes).unwrap();
        let from_store = load_from(t.path(), Some(&pack), "k3").unwrap();
        assert_eq!(from_store.to_owned(), shard);
        let mut bad = bytes.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(matches!(decode(&bad, "t"), Err(ShardError::Checksum(_))));
        assert!(matches!(
            decode(&bytes[..10], "t"),
            Err(ShardError::Format(_))
        ));
    }

    #[test]
    fn pack_log_appends_compacts_and_survives_a_torn_tail() {
        let t = tempfile::tempdir().unwrap();
        let store = t.path();
        let mk = |name: &str| {
            let mut s = sample();
            s.package.name = name.into();
            s.key = format!("k-{name}");
            s
        };
        let (a, b, c) = (mk("a"), mk("b"), mk("c"));
        Pack::append(store, &[(a.key.clone(), encode(&a))], None).unwrap();
        Pack::append(store, &[(b.key.clone(), encode(&b))], None).unwrap();
        let p = Pack::open(store).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.get("k-a").unwrap().to_owned(), a);
        assert_eq!(p.get("k-b").unwrap().to_owned(), b);
        let (keys, sizes) = Pack::index_only(store);
        assert_eq!(keys.len(), 2);
        assert_eq!(sizes[0], 0);
        assert!(sizes[1] > 0);
        // A torn record (an interrupted append) is ignored, then cut off by the next
        // append.
        let log = store.join(LOG_FILE);
        let end = p.log_end();
        let mut bytes = std::fs::read(&log).unwrap();
        bytes.extend_from_slice(&[9, 0, 0, 0, b'k']);
        std::fs::write(&log, &bytes).unwrap();
        assert_eq!(Pack::open(store).unwrap().len(), 2);
        Pack::append(store, &[(c.key.clone(), encode(&c))], Some(end)).unwrap();
        let p = Pack::open(store).unwrap();
        assert_eq!(p.get("k-c").unwrap().to_owned(), c);
        assert_eq!(Pack::index_only(store).0.len(), 3);
        // Compaction keeps exactly the listed shards, in the pack file.
        let (la, lc) = (p.get("k-a").unwrap(), p.get("k-c").unwrap());
        drop(p);
        Pack::compact(store, &[("k-a", &la), ("k-c", &lc)]).unwrap();
        assert!(!log.exists());
        let p = Pack::open(store).unwrap();
        assert_eq!(p.len(), 2);
        assert!(!p.contains("k-b"));
        assert_eq!(p.get("k-c").unwrap().to_owned(), c);
        // A store path resolves through the pack.
        assert_eq!(load(&store.join("k-a")).unwrap().to_owned(), a);
    }

    #[test]
    fn the_order_of_a_log_is_not_seen_through_the_pack() {
        // A build stages its shards in the order its threads finish them, so two builds
        // of one tree may write their logs in another order. The files differ;
        // a reader gets the same bytes under the same keys, and the same pack
        // once the log is compacted.
        let mk = |name: &str| {
            let mut s = sample();
            s.package.name = name.into();
            s.key = format!("k-{name}");
            (s.key.clone(), encode(&s))
        };
        let records = [mk("a"), mk("b"), mk("c")];
        let reversed: Vec<_> = records.iter().rev().cloned().collect();
        let (t1, t2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        Pack::append(t1.path(), &records, None).unwrap();
        Pack::append(t2.path(), &reversed, None).unwrap();
        let log = |t: &tempfile::TempDir| std::fs::read(t.path().join(LOG_FILE)).unwrap();
        assert_ne!(log(&t1), log(&t2));
        let (p1, p2) = (
            Pack::open(t1.path()).unwrap(),
            Pack::open(t2.path()).unwrap(),
        );
        assert_eq!(Pack::index_only(t1.path()).0, Pack::index_only(t2.path()).0);
        let keys = ["k-a", "k-b", "k-c"];
        for (k, bytes) in &records {
            assert_eq!(&p1.get(k).unwrap().file_bytes(), bytes);
            assert_eq!(&p2.get(k).unwrap().file_bytes(), bytes);
        }
        let packed = |p: &Pack| {
            let shards: Vec<LoadedShard> = keys.iter().map(|k| p.get(k).unwrap()).collect();
            let list: Vec<(&str, &LoadedShard)> = keys.iter().copied().zip(&shards).collect();
            Pack::bytes(&list)
        };
        assert_eq!(packed(&p1), packed(&p2));
    }
}
