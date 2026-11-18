//! Test-framework inputs in the graph.
//!
//! During a build, every file of a dirty first-party package is offered to the
//! graph frameworks ([`GRAPH_FRAMEWORKS`]: jest, vitest, pytest, go test, cargo
//! test). Whatever an adapter contributes is stored in the file's shard like
//! any other observation, so it is cached, keyed and invalidated with the
//! shard:
//!
//! | Contribution | Stored as |
//! |---|---|
//! | the governing config file(s) (and, through their own edges, everything a code config imports) | edges from the test to each config file, plus config inputs of the test |
//! | setup files, environments, transformers, presets, conftests, plugins, linked crates, the files compiled with a Go test | edges from the test (module requests resolved with the tracer's language resolver from the adapter's directory) |
//! | mocks registered in setup files | the setup file's own [`Framework::test_inputs`], added to every test |
//! | snapshots, manual mocks, `testdata` files | edges from the test |
//! | what a file that is not a test does with the mock APIs (a helper, a file a setup file imports) | on that file: edges to what it loads itself (`requireActual`) and `mock:` marker edges; a manual mock it registers without a factory is the marker `mock:manual <fw> <mock> for <module>`, which [`crate::compose::mocks`] turns into an edge from the module to the mock |
//! | missing config candidates (`jest.config.ts` next to and above the test), missing mock files, missing `conftest.py` | probes of the test |
//! | `__mocks__` directories, `__snapshots__`, `testdata/**`, package directories | memberships of the test |
//! | module mappers / aliases (jest `moduleNameMapper`, vite `resolve.alias`) | extra edges on every file under the project's root whose import specifier an alias rewrites (unresolved imports the alias resolves lose their uncertainty) |
//! | jest's haste mocks (`<root>/__mocks__/<pkg>`) | extra edges on every file under the project's root that imports `<pkg>` |
//! | which files are tests | [`crate::shard::TestRec`]s (`--tests <framework>`) |
//! | a test that several projects of one config match (the runner runs it once in each) | the inputs of every one of those projects; a `mock:hazard` marker when their runs replace other modules |
//! | per-test and config uncertainties (computed mock paths, unreadable config values) | uncertainties of the test |
//!
//! All of it lives in one group per file, with
//! [`crate::shard::FRAMEWORK_LEVEL`] as its reference index. Adapters read the
//! filesystem through the tracer's recording dir cache
//! ([`turborepo_trace_frameworks::fsx::with_fs`]), so config discovery probes
//! and config reads are recorded too.
//!
//! Invalidation: the test's group lists its config files as config inputs
//! (contents are part of the shard key, and `affected` treats them as inputs of
//! the test). Files under a project root and every test candidate also record
//! the governing config files as key-only inputs (in the shard's config table,
//! not referenced by any group): editing a module mapper or a test pattern
//! re-traces the package without making every file of the project depend on the
//! config in queries. Adding or removing a config file changes the first-party
//! keys of the packages it can govern: the package containing it and the
//! packages below its directory (every package below it if it can list
//! projects, [`ConfigPaths::digest_for`]).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use turborepo_trace_core::{
    AnalyzedEdge, Edge, FileAnalysis, FileId, Phase, RefFlags, RefKind, Span, Target, Tracer,
    Uncertainty,
    record::{self, Observations},
};
use turborepo_trace_frameworks::{
    DiscoveryMatcher, EdgeKind, Framework, FrameworkInputs, InputRole, ModuleResolver, Project,
    Target as FwTarget, TestInputs,
    common::is_path_request,
    fsx, location,
    mocks::{MockScan, mock_inputs},
};

use crate::shard::{FRAMEWORK_LEVEL, LocatedRec};

/// Adapters integrated into the graph, with the language of their test files.
/// Playwright (end-to-end tests that reach the app through URLs, not imports)
/// is selected by its own route-based adapter and is not part of the import
/// graph.
pub const GRAPH_FRAMEWORKS: &[(&str, &str)] = &[
    ("jest", "js"),
    ("vitest", "js"),
    ("pytest", "python"),
    ("gotest", "go"),
    ("cargo-test", "rust"),
];

/// Version of the framework integration, part of the options digest.
pub const INTEGRATION_VERSION: &str = "frameworks-17";

/// The adapters of [`GRAPH_FRAMEWORKS`] that are compiled in.
fn adapters() -> Vec<(&'static dyn Framework, &'static str)> {
    GRAPH_FRAMEWORKS
        .iter()
        .filter_map(|(id, lang)| turborepo_trace_frameworks::framework(id).map(|f| (f, *lang)))
        .collect()
}

/// Config file names of the graph frameworks (except `package.json`, a package
/// manifest keyed per package), sorted.
fn config_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = adapters()
        .iter()
        .flat_map(|(f, _)| f.config_file_names().iter().copied())
        .filter(|n| *n != "package.json")
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The framework config files of the walked tree, for the per-package part of
/// first-party keys ([`ConfigPaths::digest_for`]).
pub struct ConfigPaths {
    repo: PathBuf,
    /// Repo-relative config paths, sorted.
    paths: Vec<String>,
    /// Indexes into `paths` by parent directory (`""` for the root).
    by_dir: HashMap<String, Vec<usize>>,
    /// Whether a config can declare projects or workspace members elsewhere,
    /// memoized.
    multi: Vec<std::sync::OnceLock<bool>>,
    /// [`ConfigPaths::digest_for`] by the highest directory above with a multi
    /// config.
    digests: Mutex<HashMap<String, u128>>,
    /// Whether the checkout lies below a directory called `node_modules`.
    below_node_modules: bool,
}

impl ConfigPaths {
    /// The config files among `files` (repo-relative, sorted).
    pub fn new(repo: &Path, files: &[String]) -> Self {
        let names = config_names();
        let paths: Vec<String> = files
            .iter()
            .filter(|f| {
                names
                    .binary_search(&f.rsplit('/').next().unwrap_or(f))
                    .is_ok()
            })
            .cloned()
            .collect();
        let mut by_dir: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, p) in paths.iter().enumerate() {
            let dir = p.rsplit_once('/').map_or("", |(d, _)| d);
            by_dir.entry(dir.to_owned()).or_default().push(i);
        }
        let multi = (0..paths.len())
            .map(|_| std::sync::OnceLock::new())
            .collect();
        ConfigPaths {
            repo: repo.to_path_buf(),
            paths,
            by_dir,
            multi,
            digests: Mutex::default(),
            below_node_modules: repo.components().any(|c| c.as_os_str() == "node_modules"),
        }
    }

    /// Whether the checkout lies below a directory called `node_modules`.
    pub fn below_node_modules(&self) -> bool {
        self.below_node_modules
    }

    /// The configs below `dir` (repo-relative, `""` for every config).
    fn below(&self, dir: &str) -> &[String] {
        if dir.is_empty() {
            return &self.paths;
        }
        let prefix = format!("{dir}/");
        let lo = self.paths.partition_point(|p| p.as_str() < prefix.as_str());
        let n = self.paths[lo..].partition_point(|p| p.starts_with(prefix.as_str()));
        &self.paths[lo..lo + n]
    }

    /// Whether config `i` may name projects or workspace members in other
    /// directories (jest and vitest `projects`, Cargo and uv workspaces).
    /// Conservative: any mention counts.
    fn is_multi(&self, i: usize) -> bool {
        *self.multi[i].get_or_init(|| {
            std::fs::read(self.repo.join(&self.paths[i])).map_or(true, |b| {
                let has = |w: &[u8]| b.windows(w.len()).any(|x| x == w);
                has(b"projects") || has(b"workspace")
            })
        })
    }

    /// The digest of the config paths that can decide how files of the package
    /// rooted at `root` (repo-relative, `""` for the root package) are
    /// tested: configs in its directory tree, and in the directories above
    /// it (config discovery walks up from a file's directory, so a config
    /// elsewhere cannot govern the package's files). A config above that
    /// may list projects or workspace members (`projects`, `workspace`) adds
    /// every config below its directory, since adding or removing one can
    /// change the project list. Part of the package's first-party key:
    /// adding or removing a config elsewhere leaves the key alone.
    ///
    /// Where the checkout lies is no part of it: a shard whose records depend
    /// on that says so itself ([`place_state`]).
    pub fn digest_for(&self, root: &str) -> u128 {
        // With a multi config above, the set is every config below the highest
        // directory that has one plus the configs directly in the directories
        // above that one: a function of that directory alone (memoized; with a
        // root `projects` config, every package would otherwise sort and hash
        // the whole config list).
        let mut top: Option<&str> = None;
        let mut dir = root;
        loop {
            if self
                .by_dir
                .get(dir)
                .is_some_and(|ix| ix.iter().any(|&i| self.is_multi(i)))
            {
                top = Some(dir);
            }
            if dir.is_empty() {
                break;
            }
            dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        }
        if let Some(top) = top
            && let Some(&d) = self
                .digests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(top)
        {
            return d;
        }
        let start = top.unwrap_or(root);
        let mut set: Vec<&str> = self.below(start).iter().map(String::as_str).collect();
        let mut dir = start;
        loop {
            if let Some(ix) = self.by_dir.get(dir) {
                for &i in ix {
                    set.push(&self.paths[i]);
                    if self.is_multi(i) {
                        set.extend(self.below(dir).iter().map(String::as_str));
                    }
                }
            }
            if dir.is_empty() {
                break;
            }
            dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        }
        set.sort_unstable();
        set.dedup();
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(INTEGRATION_VERSION.as_bytes());
        for f in set {
            h.update(&(f.len() as u64).to_le_bytes());
            h.update(f.as_bytes());
        }
        let d = h.digest128();
        if let Some(top) = top {
            self.digests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(top.to_owned(), d);
        }
        d
    }
}

/// The place of the checkout as an input of the shard `s`: whether it lies
/// below a directory called `node_modules`, when a file of the shard is
/// governed by a tsconfig with `paths`, `baseUrl` or `rootDirs`
/// ([`crate::shard::FileRec::aliased`]); `None` otherwise. Below such a
/// directory the resolver applies none of them to the repository's files and
/// those files get open records (`turborepo_trace_lang_js::resolve`,
/// `aliases_not_applied`): a graph moved across that boundary traces such a
/// shard again, as a cold build there would. The fact is recorded by the
/// resolver for every config it consults, wherever it lies (ignored by
/// `.gitignore`, behind a link, above the repository root).
pub(crate) fn place_state(cx: &crate::build::Cx, s: &crate::shard::ArchivedShard) -> Option<bool> {
    s.files
        .iter()
        .any(|f| f.aliased)
        .then(|| cx.fw_configs.below_node_modules())
}

/// The state [`place_state`] names, as the key and the observation states hash
/// it.
pub(crate) fn place_word(below: bool) -> &'static str {
    if below {
        "below node_modules"
    } else {
        "elsewhere"
    }
}

/// One resolved framework edge.
#[derive(Clone, Debug)]
struct FwEdge {
    kind: RefKind,
    phase: Phase,
    /// Display form; the bare request for unresolved module requests (so the
    /// store can still link it through the lockfile).
    spec: String,
    succ: Vec<FileId>,
    links: Vec<FileId>,
    uncertainty: Option<Uncertainty>,
}

/// A module request resolved with the tracer.
#[derive(Clone, Debug, Default)]
struct Resolved {
    succ: Vec<FileId>,
    links: Vec<FileId>,
    obs: Observations,
}

/// A loaded config: the adapter's inputs, what loading observed, and compiled
/// per-project state.
struct Loaded {
    fw: &'static dyn Framework,
    lang: &'static str,
    /// Leaked once per config and build, so compiled discovery matchers can
    /// borrow it.
    inputs: &'static FrameworkInputs,
    /// Compiled test discovery per project.
    matchers: Vec<DiscoveryMatcher<'static>>,
    obs: Observations,
    /// What loading accessed besides the contents of `config_files`
    /// ([`Fw::key_of`]): every path it asked about, whatever the answer (a
    /// preset that is not there, a setup file that is, the candidates of a
    /// helper it imports), every listing it made (the directories a
    /// `projects` glob matches, the haste map's mocks) and what else it
    /// read. The load decides the framework inputs of every file it is
    /// consulted for, also of one that gets none (a file that is no test of
    /// the config as loaded, a file whose imports no mapper names), so each
    /// of them names this set as a key observation ([`Loaded::consulted`]).
    /// The tests of a project have the load's observations as
    /// inputs of their own too ([`ProjectState::obs`]).
    key: Arc<Observations>,
    /// The symbolic links the load crossed to a real path (inside the
    /// repository): where each one leads is a record of every file the load
    /// is consulted for ([`FileFw::reached`]).
    crossed: Vec<PathBuf>,
    /// Per project: the discovery that was not chosen, compiled, and what the
    /// choice was
    /// made from ([`turborepo_trace_frameworks::DiscoveryChoice`]): a key
    /// observation of the files whose answer the choice decides, and of no
    /// other. With the links that part of the load crossed, which are
    /// records of the same files.
    choices: Vec<Option<Choice>>,
    /// Config files of the whole config (all projects), absolute.
    config_files: Vec<PathBuf>,
    projects: Vec<OnceLock<ProjectState>>,
    /// The place of the checkout as an input of the discovery of each project
    /// whose runner matches absolute paths ([`crate::located`]): a record
    /// of every file the load is consulted for.
    located: Vec<Arc<LocatedRec>>,
}

impl Loaded {
    /// Records on a file that the load was consulted for it: the contents of
    /// the config files as key-only configs, what else the load accessed as
    /// a key observation of the file's framework group (`obs`), and the
    /// links it crossed.
    fn consulted(&self, out: &mut FileFw, obs: &mut Observations) {
        // (Once per file: a file that may be a test and the imports of the same file
        // ask the same loads.)
        let id = self as *const Loaded as usize;
        if out.consulted.contains(&id) {
            return;
        }
        out.consulted.push(id);
        out.key_reads.extend(self.config_files.iter().cloned());
        key_observation(obs, &self.key);
        if !self.crossed.is_empty() {
            out.reached.push((PathBuf::new(), self.crossed.clone()));
        }
        for l in &self.located {
            if !out.located.iter().any(|o| Arc::ptr_eq(o, l)) {
                out.located.push(Arc::clone(l));
            }
        }
    }
}

/// Names `set` as a key observation in `obs`, once.
fn key_observation(obs: &mut Observations, set: &Arc<Observations>) {
    if !set.is_empty() && !obs.key.iter().any(|k| Arc::ptr_eq(k, set)) {
        obs.key.push(Arc::clone(set));
    }
}

/// Per-project inputs shared by every test.
#[derive(Default)]
struct ProjectState {
    edges: Vec<FwEdge>,
    obs: Observations,
    uncertainties: Vec<FwUnc>,
    /// What the project's own resolutions found ([`FileFw::reached`]).
    reached: Vec<Reached>,
}

/// A file that a resolution of the framework pass found, with the symbolic
/// links it crossed to get there (absolute paths). An empty path: links that
/// were crossed without a file to name (the real path of a directory, asked for
/// by the load of a config).
pub(crate) type Reached = (PathBuf, Vec<PathBuf>);

type Memo<K, V> = Mutex<HashMap<K, Arc<OnceLock<V>>>>;

/// The walk that looks for the config of the files of one directory: the config
/// it found, what it looked for and read on the way (inputs of a test), and
/// everything it accessed ([`Fw::key_of`]), as one set per directory it looked
/// into ([`Fw::walk_levels`]). The walk decides which config is consulted for a
/// file, also for a file that is no test and for one that has no config: key
/// observations of every file that made it. (Shared: every file of the
/// directory takes it.)
type Walk = Arc<(Option<PathBuf>, Observations, Vec<Arc<Observations>>)>;

/// [`Fw::walk_levels`]: per adapter and directory, the distinct sets of what a walk
/// accessed there.
type WalkLevels = HashMap<(usize, PathBuf), Vec<Arc<Observations>>>;

/// [`Fw::js_loads_for`]: the loads, and what the walks that picked them accessed.
type JsLoads = Arc<(Vec<Arc<Loaded>>, Vec<Arc<Observations>>)>;

/// [`Fw::js_projects_for`]: the projects (config, project index) whose root contains a
/// directory, shared by the files of the directory.
type JsProjects = Arc<Vec<(Arc<Loaded>, usize)>>;

/// `(config, project index, specifier, from dir)`.
type AliasKey = (PathBuf, usize, String, PathBuf);

/// A choice of a discovery ([`Loaded::choices`]): the discovery that was not
/// chosen, the set of what the choice was made from, and the links it crossed.
type Choice = (DiscoveryMatcher<'static>, Arc<Observations>, Vec<PathBuf>);

/// [`Fw::carry`]: the set of this pass, and the tracer's sets.
type FileSets = (Arc<Observations>, Vec<Arc<Observations>>);

fn memo<K: std::hash::Hash + Eq + Clone, V>(m: &Memo<K, V>, k: &K) -> Arc<OnceLock<V>> {
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(g.entry(k.clone()).or_default())
}

/// Config loads computed (diagnostics for `TURBO_TRACE_DEBUG_TIMING`;
/// concurrent first callers of one config may each compute it).
pub(crate) static LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Tests that more than one project of their config matches (diagnostics for
/// `TURBO_TRACE_DEBUG_TIMING`).
pub(crate) static SHARED_TESTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Framework integration state for one build.
pub(crate) struct Fw<'a> {
    tracer: &'a Tracer,
    repo: &'a Path,
    files: &'a [String],
    adapters: Vec<(&'static dyn Framework, &'static str)>,
    /// `(adapter index, directory)` → governing config, the probes of finding
    /// it, and everything the walk accessed ([`Fw::key_of`]).
    config_for: Memo<(usize, PathBuf), Walk>,
    /// `(adapter index, config)` → loaded config.
    loaded: Memo<(usize, PathBuf), Arc<Loaded>>,
    /// `(lang, request, from dir)` → resolution.
    resolved: Memo<(&'static str, String, PathBuf), Resolved>,
    /// jest/vitest config candidates by directory (from the walked files).
    js_config_index: OnceLock<HashMap<PathBuf, Vec<(usize, PathBuf)>>>,
    /// Directory → the loaded js configs that govern it or a directory above
    /// it, and the walks that said which config governs each of those
    /// directories.
    js_loads_for: Memo<PathBuf, JsLoads>,
    /// `(adapter index, directory)` → what a walk for a config accessed in that
    /// directory ([`Fw::walk_levels`]): one shared set per distinct record.
    walk_levels: Mutex<WalkLevels>,
    /// Directory → the js projects (config, project index) whose root contains
    /// it.
    js_projects_for: Memo<PathBuf, JsProjects>,
    /// `(config, project, specifier, from dir)` → alias and haste-mock edges.
    alias_memo: Memo<AliasKey, (Vec<FwEdge>, Observations)>,
    /// The sets of key observations this pass made ([`Fw::own`]), by identity
    /// (each kept alive here, so that no other set can take its place in
    /// memory). A set that reaches a file's framework observations from
    /// elsewhere (the tsconfig chain a request was resolved under: the
    /// tracer's) is not one of them ([`Fw::augment`]).
    own_sets: dashmap::DashMap<usize, Arc<Observations>>,
    /// `(lang, directory)` → what the tracer consulted for the file that the
    /// requests of the framework pass from that directory are resolved as
    /// ([`Fw::carry`]): one set of this pass for every such request, and
    /// the sets the tracer made for the file.
    file_sets: Memo<(&'static str, PathBuf), FileSets>,
    /// The states of the location records ([`crate::located`]).
    places: &'a crate::located::Places,
}

/// What [`Fw::augment`] found for one file besides the edges and group it
/// appended.
#[derive(Debug, Default)]
pub(crate) struct FileFw {
    /// Frameworks that run the file as a test, each with the working
    /// directories it may run the test in (absolute; see
    /// [`crate::shard::TestRec::cwds`]).
    pub tests: Vec<(&'static str, Vec<PathBuf>)>,
    /// Config files whose contents decide the file's framework inputs without
    /// being query inputs (absolute).
    pub key_reads: Vec<PathBuf>,
    /// What the resolutions made for the file found. A resolution whose result
    /// becomes an edge is recorded by the edge's target; one whose result
    /// is only named (a module a `jest.mock` replaces, in a `mock:` marker)
    /// or only compared would leave no trace of what it found: the file's
    /// records would not move when the found package is uninstalled or the
    /// found file deleted, although a new trace resolves differently.
    /// The store records each of these as reached by the file, like an edge
    /// target, without an edge ([`crate::build::TargetCx::reach`]).
    pub reached: Vec<Reached>,
    /// The loads that were consulted for the file ([`Loaded::consulted`]), by
    /// identity.
    consulted: Vec<usize>,
    /// The location records of the projects whose config was consulted for the
    /// file ([`crate::located`]).
    pub located: Vec<Arc<LocatedRec>>,
}

impl<'a> Fw<'a> {
    pub(crate) fn new(
        tracer: &'a Tracer,
        repo: &'a Path,
        files: &'a [String],
        places: &'a crate::located::Places,
    ) -> Self {
        Fw {
            tracer,
            repo,
            files,
            places,
            adapters: adapters(),
            config_for: Mutex::default(),
            loaded: Mutex::default(),
            resolved: Mutex::default(),
            js_config_index: OnceLock::new(),
            js_loads_for: Mutex::default(),
            walk_levels: Mutex::default(),
            js_projects_for: Mutex::default(),
            alias_memo: Mutex::default(),
            own_sets: dashmap::DashMap::new(),
            file_sets: Mutex::default(),
        }
    }

    /// A set of key observations of this pass: what a load or a walk accessed.
    fn own(&self, set: Observations) -> Arc<Observations> {
        let set = Arc::new(set);
        self.own_sets
            .insert(Arc::as_ptr(&set) as usize, Arc::clone(&set));
        set
    }

    /// Adds to `obs` what the tracer consulted for the file that a request from
    /// `from_dir` is resolved as (`file`: the observations of that file's
    /// file-level group: the tsconfig chain, the scope of its manifest), as
    /// key observations.
    ///
    /// The request is resolved as that file's, under the `paths` of the
    /// tsconfig that governs the directory, and whoever takes the result
    /// may lie below another config and have no record of this one: when
    /// the chain changes, the result is computed again. The records are the
    /// same for every request from the directory, and a file
    /// takes the results of many (every import a module mapper rewrites, every
    /// request of its project's config): one set of this pass per
    /// directory, plus the sets the tracer made for that file (the projects
    /// its configs reference), which [`Fw::augment`] hands to the
    /// file-level group.
    ///
    /// Key observations, not inputs: the file is a `.js` file that is not
    /// there, and what its config discovery reads is every config above it
    /// that does not cover it. As inputs, an edit of a solution config at
    /// the root would select every test whose runner config makes a request
    /// (`affected.rs`, `only_the_governing_tsconfig_is_an_input`). So a
    /// query selects nothing through this chain, as before; a refresh keeps
    /// nothing that was computed from it.
    fn carry(
        &self,
        lang: &'static str,
        from_dir: &Path,
        file: &Observations,
        obs: &mut Observations,
    ) {
        if file.is_empty() {
            return;
        }
        let cell = memo(&self.file_sets, &(lang, from_dir.to_path_buf()));
        let (own, theirs) = crate::once::init_once(&cell, || {
            let mut set = file.clone();
            let theirs = std::mem::take(&mut set.key);
            (self.own(set), theirs)
        });
        for set in std::iter::once(own).chain(theirs) {
            key_observation(obs, set);
        }
    }

    /// The memo tables of the build, for the caller to free off its critical
    /// path: they hold one entry per request and alias the build resolved,
    /// which takes hundreds of milliseconds to free on a large repository.
    pub(crate) fn into_memos(self) -> Box<dyn Send> {
        Box::new((
            self.config_for,
            self.loaded,
            self.resolved,
            self.js_config_index,
            self.js_loads_for,
            self.walk_levels,
            self.js_projects_for,
            self.alias_memo,
            self.own_sets,
            self.file_sets,
        ))
    }

    /// Runs `f` with adapter file access routed through the tracer's recording
    /// dir cache, inside a fresh recording frame; returns its result and
    /// observations.
    ///
    /// Directory listings recorded by the adapter's own walks (the haste map
    /// crawl lists every directory below the roots) are dropped: adapters
    /// declare the memberships they depend on ([`TestInputs::memberships`],
    /// `GlobalInputs::memberships`, such as `**/__mocks__/**`), and a raw
    /// `**` listing would make every added file an input of every test.
    /// Probes and reads are kept.
    ///
    /// A result that is kept for others (the load of a config, the walk that
    /// finds one) is computed inside [`fsx::with_log`] as well, and its
    /// record is the log ([`Fw::key_of`]). The inputs of one file (a test's
    /// mocks and snapshot, the files compiled with a Go test, a project's
    /// setup files) are computed after the load, without a log, and are the
    /// file's own: what they find becomes an edge (its target is a record:
    /// a target that goes, or comes to be a directory or a link, traces the
    /// file again), what they do not find is a path that was looked for
    /// (recorded here by the cache), a directory they search is a
    /// membership the adapter declares, and what the cargo project model
    /// asks for one test it asks for the load too. A log of these accesses,
    /// kept as a set of the file, was tried and no case needed it (go test,
    /// cargo test and pytest sequences,
    /// `crates/turborepo-trace-store/tests/cutoff.rs`, `runners`; a file of
    /// the package that goes keeps no file of it anyway).
    fn recorded<R>(&self, f: impl FnOnce() -> R) -> (R, Observations) {
        let frame = record::start();
        let r = fsx::with_fs(self.tracer.fs_arc(), f);
        let mut obs = frame.finish();
        obs.listings.clear();
        (r, obs)
    }

    /// Distinct configs loaded (diagnostics).
    pub(crate) fn loaded_count(&self) -> usize {
        self.loaded.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn config_for(&self, ai: usize, dir: &Path) -> Walk {
        let cell = memo(&self.config_for, &(ai, dir.to_path_buf()));
        crate::once::init_once(&cell, || {
            let fw = self.adapters[ai].0;
            // `config_for` looks at the test's parent directory only.
            let ((config, obs), log) = fsx::with_log(|| {
                self.recorded(|| fw.config_for(&dir.join("__turbotrace_probe__")))
            });
            let levels = self.walk_levels(ai, self.key_of(&obs, &log, &[]));
            Arc::new((config, obs, levels))
        })
        .clone()
    }

    /// What a walk for a config accessed, as one set per directory it looked
    /// into, each shared with every other walk that looked into that
    /// directory and saw the same.
    ///
    /// A walk goes up from a file's directory and asks the same of every
    /// directory on the way, so the walks of the files of a subtree overlap
    /// in everything above them. As one set per walk, a file deep in the
    /// tree repeated every directory above it (in large repositories, a
    /// tenth of the time of a cold build's assembly); as one
    /// set per directory, a file names the directories its walk passed.
    fn walk_levels(&self, ai: usize, key: Observations) -> Vec<Arc<Observations>> {
        // Names that were looked up have no directory: such a walk stays one set.
        if !key.packages.is_empty() {
            return vec![self.own(key)];
        }
        let mut by_dir: std::collections::BTreeMap<PathBuf, Observations> = Default::default();
        let dir_of = |p: &Path| p.parent().unwrap_or(p).to_path_buf();
        for p in key.missing {
            by_dir.entry(dir_of(&p)).or_default().missing.push(p);
        }
        for p in key.reads {
            by_dir.entry(dir_of(&p)).or_default().reads.push(p);
        }
        for p in key.view_reads {
            by_dir.entry(dir_of(&p)).or_default().view_reads.push(p);
        }
        for p in key.loader_reads {
            by_dir.entry(dir_of(&p)).or_default().loader_reads.push(p);
        }
        for (d, pattern) in key.listings {
            by_dir
                .entry(d.clone())
                .or_default()
                .listings
                .push((d, pattern));
        }
        let mut known = self.walk_levels.lock().unwrap_or_else(|e| e.into_inner());
        by_dir
            .into_iter()
            .map(|(dir, set)| {
                let sets = known.entry((ai, dir)).or_default();
                match sets.iter().find(|s| ***s == set) {
                    Some(s) => Arc::clone(s),
                    None => {
                        let s = self.own(set);
                        sets.push(Arc::clone(&s));
                        s
                    }
                }
            })
            .collect()
    }

    /// What a computation of the framework pass accessed, as one set of
    /// observations: the record of a result that is kept for others (the
    /// load of a config, the walk that finds one). `obs`: what the cache
    /// recorded (the paths that were not there, the contents read); `log`:
    /// what the adapter asked of the file system
    /// ([`turborepo_trace_frameworks::fsx::Log`]); `config_files` (sorted):
    /// files whose contents the caller records itself.
    ///
    /// - A path that was asked about is in the set whatever the answer was, as
    ///   a path that was looked for: the record is the same with the file there
    ///   or not, and its state (nothing, a file, a directory) is what a refresh
    ///   compares. A lookup that succeeds leaves nothing in the cache, and two
    ///   trees with the file and without it would give the set other records: a
    ///   shard traced with the file would not move when it goes. A path that is
    ///   read is covered by its contents.
    /// - A listing is in the set as the directory and the pattern the adapter
    ///   asked for (the raw listings of the cache are not: [`Fw::recorded`]).
    fn key_of(&self, obs: &Observations, log: &fsx::Log, config_files: &[PathBuf]) -> Observations {
        let mut key = Observations {
            missing: obs.missing.clone(),
            listings: log.listed.clone(),
            reads: obs.reads.clone(),
            path_reads: Vec::new(),
            view_reads: obs.view_reads.clone(),
            loader_reads: obs.loader_reads.clone(),
            packages: obs.packages.clone(),
            // (The links an observed path went through: where each leads is part of
            // what was observed, `turborepo_trace_core::record::link`.)
            links: obs.links.clone(),
            key: Vec::new(),
            // (A fact of the file that is resolved, not of the load: no key set has it.)
            aliased: false,
        };
        key.reads.extend(log.read.iter().cloned());
        key.reads.retain(|r| config_files.binary_search(r).is_err());
        let read: std::collections::HashSet<&PathBuf> = key
            .reads
            .iter()
            .chain(&key.view_reads)
            .chain(&key.loader_reads)
            .chain(config_files)
            .collect();
        let asked: Vec<PathBuf> = log
            .asked
            .iter()
            .chain(&obs.path_reads)
            .filter(|p| !read.contains(p))
            .cloned()
            .collect();
        key.missing.extend(asked);
        key.dedup();
        key
    }

    fn load(&self, ai: usize, config: &Path) -> Arc<Loaded> {
        let cell = memo(&self.loaded, &(ai, config.to_path_buf()));
        Arc::clone(crate::once::init_once(&cell, || {
            LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (fw, lang) = self.adapters[ai];
            let ((inputs, obs), mut log) = fsx::with_log(|| self.recorded(|| fw.load(config)));
            let mut config_files: Vec<PathBuf> = inputs.config_file.iter().cloned().collect();
            for p in &inputs.projects {
                config_files.extend(p.config_file.iter().cloned());
                config_files.extend(p.global.config_files.iter().cloned());
            }
            config_files.sort();
            config_files.dedup();
            // The parts of the load that decide for some files only: the choice of a
            // project's discovery. A part that no project names decides for every file.
            let mut parts: Vec<(String, fsx::Log)> = Vec::new();
            for (name, part) in std::mem::take(&mut log.parts) {
                let named = inputs
                    .projects
                    .iter()
                    .any(|p| p.discovery.choice.as_ref().is_some_and(|c| c.part == name));
                if named {
                    parts.push((name, part));
                } else {
                    log.extend(part);
                }
            }
            // (Where a link leads that a part crossed is a record of the files the part
            // decides for, like its set: a file that does not name the set keeps its
            // records when the part's walk comes to cross a link.)
            let parts: Vec<(String, Arc<Observations>, Vec<PathBuf>)> = parts
                .into_iter()
                .map(|(name, part)| {
                    let set = self.key_of(&Observations::default(), &part, &config_files);
                    (name, self.own(set), self.crossed(&part))
                })
                .collect();
            let key = self.own(self.key_of(&obs, &log, &config_files));
            let crossed = self.crossed(&log);
            let projects = inputs.projects.iter().map(|_| OnceLock::new()).collect();
            let inputs: &'static FrameworkInputs = Box::leak(Box::new(inputs));
            let matchers = inputs
                .projects
                .iter()
                .map(|p| p.discovery.matcher().rooted(self.repo))
                .collect();
            let choices = inputs
                .projects
                .iter()
                .map(|p| {
                    let c = p.discovery.choice.as_deref()?;
                    let (_, set, crossed) = parts.iter().find(|(name, _, _)| *name == c.part)?;
                    Some((
                        c.other.matcher().rooted(self.repo),
                        Arc::clone(set),
                        crossed.clone(),
                    ))
                })
                .collect();
            // Whether the place of the checkout decides which files are tests, per project
            // whose runner matches absolute paths.
            let mut located: Vec<Arc<LocatedRec>> = Vec::new();
            for p in &inputs.projects {
                let Some((spec, _)) = location::ProjectSpec::of(&p.discovery) else {
                    continue;
                };
                let spec = spec.to_text();
                let rec = LocatedRec {
                    config: self.display(p.config_file.as_deref().unwrap_or(config)),
                    root: self.places.state(&spec).to_string(),
                    spec,
                };
                if !located.iter().any(|l| **l == rec) {
                    located.push(Arc::new(rec));
                }
            }
            Arc::new(Loaded {
                fw,
                lang,
                inputs,
                matchers,
                obs,
                key,
                crossed,
                choices,
                config_files,
                projects,
                located,
            })
        }))
    }

    /// The symbolic links inside the repository that the logged computation
    /// crossed to a real path, sorted. (A link above the repository is the
    /// machine's.)
    fn crossed(&self, log: &fsx::Log) -> Vec<PathBuf> {
        let mut links: Vec<PathBuf> = log
            .links
            .iter()
            .filter(|l| l.starts_with(self.repo))
            .cloned()
            .collect();
        links.sort();
        links.dedup();
        links
    }

    /// Resolves `request` from `from_dir` with the language resolver, as if
    /// written in a file in that directory.
    fn resolve(&self, lang: &'static str, request: &str, from_dir: &Path) -> Resolved {
        self.resolved_cell(lang, request, from_dir)
            .get()
            .cloned()
            .unwrap_or_default()
    }

    /// [`Fw::resolve`] as the memo cell (filled), for callers that only read
    /// it.
    fn resolved_cell(
        &self,
        lang: &'static str,
        request: &str,
        from_dir: &Path,
    ) -> Arc<OnceLock<Resolved>> {
        let cell = memo(
            &self.resolved,
            &(lang, request.to_owned(), from_dir.to_path_buf()),
        );
        crate::once::init_once(&cell, || {
            let quoted = serde_json::to_string(request).unwrap_or_default();
            let (name, src) = match lang {
                "js" => (
                    "__turbotrace_framework__.js",
                    format!("require({quoted});\n"),
                ),
                "python" => {
                    let ok = !request.is_empty()
                        && request.split('.').all(|s| {
                            !s.is_empty() && s.chars().all(|c| c == '_' || c.is_alphanumeric())
                        });
                    if !ok {
                        return Resolved::default();
                    }
                    ("__turbotrace_framework__.py", format!("import {request}\n"))
                }
                "go" => (
                    "__turbotrace_framework__.go",
                    format!("package p\n\nimport {quoted}\n"),
                ),
                _ => return Resolved::default(),
            };
            let a = self
                .tracer
                .analyze_source(&from_dir.join(name), src.as_bytes());
            let mut out = Resolved::default();
            for g in &a.groups {
                match g.ref_index {
                    // The request.
                    Some(0) => {
                        for e in &a.edges[g.edges.clone()] {
                            out.succ.extend(e.succ.iter().copied());
                            out.links.extend(e.links.iter().copied());
                        }
                        out.obs.extend_from(&g.observations);
                    }
                    // What the tracer consulted for the file the request was written in
                    // ([`Fw::carry`]). Its edges are that file's own.
                    None => self.carry(lang, from_dir, &g.observations, &mut out.obs),
                    Some(_) => {}
                }
            }
            out
        });
        cell
    }

    /// An edge to `target`: a path (an existing file, else a probe) or a module
    /// request.
    fn target_edge(
        &self,
        lang: &'static str,
        kind: RefKind,
        phase: Phase,
        label: &str,
        target: &FwTarget,
        obs: &mut Observations,
    ) -> Option<FwEdge> {
        match target {
            FwTarget::Path(p) => {
                let p = fsx::normalize(p);
                // A path outside the repository (a config's `<rootDir>/../../x` that leaves
                // it, an absolute path of the machine) is not looked up: neither an edge to
                // a host file nor a probe.
                if self.tracer.fs().is_host_path(&p) {
                    return None;
                }
                if self.tracer.fs().is_file(&p) {
                    Some(FwEdge {
                        kind,
                        phase,
                        spec: format!("{label} {}", self.display(&p)),
                        succ: vec![self.tracer.intern(&p)],
                        links: Vec::new(),
                        uncertainty: None,
                    })
                } else {
                    obs.missing.push(p);
                    None
                }
            }
            FwTarget::Module { request, from_dir } => {
                let r = self.resolve(lang, request, from_dir);
                obs.extend_from(&r.obs);
                let shown = self.shown_request(request);
                let spec = if r.succ.is_empty() {
                    shown.into_owned()
                } else {
                    format!("{label} {shown}")
                };
                Some(FwEdge {
                    kind,
                    phase,
                    spec,
                    succ: r.succ,
                    links: r.links,
                    uncertainty: None,
                })
            }
        }
    }

    /// A module request for an edge's specifier. A config value that leaves the
    /// repository by a step or two (`<rootDir>/../../tests/setup.ts` of a
    /// project at the root) is shown relative to the root, so the specifier
    /// does not hold the checkout's location.
    fn shown_request<'r>(&self, request: &'r str) -> std::borrow::Cow<'r, str> {
        let p = Path::new(request);
        if !p.is_absolute() || p.starts_with(self.repo) {
            return request.into();
        }
        for (up, dir) in self.repo.ancestors().enumerate().skip(1).take(2) {
            if dir.parent().is_some()
                && let Ok(rest) = p.strip_prefix(dir)
            {
                return format!("{}{}", "../".repeat(up), rest.display()).into();
            }
        }
        request.into()
    }

    fn display(&self, p: &Path) -> String {
        p.strip_prefix(self.repo)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string_lossy().into_owned())
    }

    /// Converts per-test (or per-setup-file) inputs to edges and observations.
    fn test_input_edges(
        &self,
        lang: &'static str,
        fw_id: &str,
        ti: &TestInputs,
        edges: &mut Vec<FwEdge>,
        obs: &mut Observations,
        uncs: &mut Vec<FwUnc>,
    ) {
        for e in &ti.entries {
            let (kind, phase) = role_kind(lang, e.role);
            let label = format!("{fw_id}:{}", role_name(e.role));
            if let Some(x) = self.target_edge(lang, kind, phase, &label, &e.target, obs) {
                edges.push(x);
            }
        }
        for e in &ti.edges {
            let (kind, label) = match e.kind {
                EdgeKind::Snapshot => (RefKind::FsRead, "snapshot"),
                EdgeKind::ManualMock => (code_kind(lang), "manual_mock"),
            };
            let label = format!("{fw_id}:{label}");
            if let Some(x) = self.target_edge(
                lang,
                kind,
                Phase::Runtime,
                &label,
                &FwTarget::Path(e.to.clone()),
                obs,
            ) {
                edges.push(x);
            }
        }
        obs.missing
            .extend(ti.probes.iter().map(|p| fsx::normalize(p)));
        obs.listings.extend(
            ti.memberships
                .iter()
                .map(|m| (fsx::normalize(&m.dir), m.pattern.clone())),
        );
        uncs.extend(ti.uncertainties.iter().map(fw_unc));
        self.mock_markers(fw_id, ti, edges);
    }

    /// What `ti`'s file replaces with a mock, loads for real and leaves open,
    /// as marker edges (unresolved edges whose specifier starts with
    /// `mock:`; they are read by `turborepo_trace_graph::barrel::mocks`,
    /// which documents them).
    fn mock_markers(&self, fw_id: &str, ti: &TestInputs, edges: &mut Vec<FwEdge>) {
        let at = self.display(&ti.test);
        for r in &ti.replaced {
            edges.push(marker(format!(
                "mock:replaces {fw_id} {} {} @ {at}:{}",
                if r.by_path { "path" } else { "name" },
                self.display(&r.module),
                r.line
            )));
        }
        for p in &ti.loaded_real {
            edges.push(marker(format!("mock:actual {}", self.display(p))));
        }
        for h in &ti.mock_hazards {
            edges.push(marker(format!("mock:hazard {} @ {at}:{}", h.what, h.line)));
        }
        // A manual mock that a file other than a test registers without a factory loads
        // when its module is requested: `crate::compose::mocks` adds the edge from the
        // module to the mock.
        for l in &ti.lazy_mocks {
            edges.push(marker(format!(
                "mock:manual {fw_id} {} for {}",
                self.display(&l.mock),
                self.display(&l.module)
            )));
        }
    }

    /// The inputs every test of `project` shares (computed once per project).
    fn project_state<'l>(&self, l: &'l Loaded, pi: usize) -> &'l ProjectState {
        crate::once::init_once(&l.projects[pi], || {
            let p = &l.inputs.projects[pi];
            let id = l.fw.id();
            let mut st = ProjectState::default();
            let mut obs = l.obs.clone();
            for c in &l.config_files {
                obs.reads.push(c.clone());
                let code = p.global.js_entries.contains(c);
                let kind = if code {
                    RefKind::Require
                } else {
                    RefKind::FsRead
                };
                if let Some(e) = self.target_edge(
                    l.lang,
                    kind,
                    Phase::Runtime,
                    &format!("{id}:config"),
                    &FwTarget::Path(c.clone()),
                    &mut obs,
                ) {
                    st.edges.push(e);
                }
            }
            for c in &p.global.js_entries {
                if let Some(e) = self.target_edge(
                    l.lang,
                    RefKind::Require,
                    Phase::Runtime,
                    &format!("{id}:config"),
                    &FwTarget::Path(c.clone()),
                    &mut obs,
                ) {
                    st.edges.push(e);
                }
            }
            for e in &p.global.entries {
                let (kind, phase) = role_kind(l.lang, e.role);
                let label = format!("{id}:{}", role_name(e.role));
                let Some(x) = self.target_edge(l.lang, kind, phase, &label, &e.target, &mut obs)
                else {
                    continue;
                };
                // Mocks registered in setup files apply to every test of the project.
                if matches!(
                    e.role,
                    InputRole::SetupFile | InputRole::SetupFileAfterEnv | InputRole::Conftest
                ) {
                    for &f in &x.succ {
                        let path = self.tracer.path_of(f);
                        if path.strip_prefix(self.repo).is_ok_and(|rel| {
                            !rel.components().any(|c| c.as_os_str() == "node_modules")
                        }) {
                            let res = Resolver::new(self, l.lang);
                            let (ti, o) = self.recorded(|| {
                                let src = fsx::read(&*path).unwrap_or_default();
                                l.fw.test_inputs(p, &path, &src, &res)
                            });
                            obs.extend(o);
                            st.reached.extend(res.reached.take());
                            obs.extend(res.obs.into_inner());
                            let mut setup_edges = Vec::new();
                            self.test_input_edges(
                                l.lang,
                                id,
                                &ti,
                                &mut setup_edges,
                                &mut obs,
                                &mut st.uncertainties,
                            );
                            st.edges.extend(setup_edges);
                        }
                    }
                }
                st.edges.push(x);
            }
            for e in &p.global.edges {
                if let Some(x) = self.target_edge(
                    l.lang,
                    code_kind(l.lang),
                    Phase::Runtime,
                    &format!("{id}:implicit"),
                    &FwTarget::Path(e.to.clone()),
                    &mut obs,
                ) {
                    st.edges.push(x);
                }
            }
            // The project's replacements of modules by mocks do not hold.
            if let Some(why) = &p.mocks.unreliable {
                let at = l.inputs.config_file.as_deref().map(|c| self.display(c));
                st.edges.push(marker(format!(
                    "mock:hazard {why} @ {}:0",
                    at.as_deref().unwrap_or(id)
                )));
            }
            // Read by the runner, not loaded by the test: an input by content, no edge.
            obs.reads
                .extend(p.global.reads.iter().map(|p| fsx::normalize(p)));
            obs.missing
                .extend(p.global.probes.iter().map(|p| fsx::normalize(p)));
            obs.listings.extend(
                p.global
                    .memberships
                    .iter()
                    .map(|m| (fsx::normalize(&m.dir), m.pattern.clone())),
            );
            st.uncertainties.extend(
                l.inputs
                    .uncertainties
                    .iter()
                    .chain(&p.uncertainties)
                    .filter(|u| counts_for_tests(u))
                    .map(fw_unc),
            );
            // (Every test of the project takes a copy.)
            obs.dedup();
            st.obs = obs;
            st
        })
    }

    /// jest/vitest config files of the repository by directory.
    fn js_config_index(&self) -> &HashMap<PathBuf, Vec<(usize, PathBuf)>> {
        crate::once::init_once(&self.js_config_index, || {
            let mut out: HashMap<PathBuf, Vec<(usize, PathBuf)>> = HashMap::new();
            for (ai, (fw, lang)) in self.adapters.iter().enumerate() {
                if *lang != "js" {
                    continue;
                }
                let names = fw.config_file_names();
                for f in self.files {
                    let base = f.rsplit('/').next().unwrap_or(f);
                    if names.contains(&base) {
                        let c = self.repo.join(f);
                        let dir = c.parent().unwrap_or(self.repo).to_path_buf();
                        out.entry(dir).or_default().push((ai, c));
                    }
                }
            }
            out
        })
    }

    /// The js projects whose root contains `dir`, among the configs in `dir`
    /// and its ancestors up to the repository root (the preferred config
    /// per directory). Projects whose `rootDir`/`roots` point outside their
    /// config's subtree are not seen from files there.
    fn js_projects_for(&self, dir: &Path) -> JsProjects {
        let cell = memo(&self.js_projects_for, &dir.to_path_buf());
        crate::once::init_once(&cell, || {
            let mut out = Vec::new();
            for l in &self.js_loads_for(dir).0 {
                for (pi, p) in l.inputs.projects.iter().enumerate() {
                    let under_root = dir.starts_with(&p.root_dir)
                        || p.resolution.roots.iter().any(|r| dir.starts_with(r))
                        || p.discovery.roots.iter().any(|r| dir.starts_with(r));
                    if under_root {
                        out.push((Arc::clone(l), pi));
                    }
                }
            }
            Arc::new(out)
        })
        .clone()
    }

    /// The loaded js configs in `dir` and its ancestors up to the repository
    /// root (the preferred config per directory): every config whose
    /// projects are asked whether their root contains `dir`
    /// ([`Fw::js_projects_for`]). The answer is a function of the config's
    /// load, also when it is no.
    ///
    /// Which config a directory has is the answer of the adapter's own walk
    /// from it ([`Fw::config_for`]), the walk that finds the config of a
    /// test there: a config file, or a config that no file name announces
    /// (the `jest` key of a `package.json`). So the module mapper, the
    /// haste mocks and the roots of a config in a manifest apply
    /// to the files below it like those of a config file, and every directory
    /// above the file is asked: the walks are records of the file
    /// ([`Walk`]), also for a directory that has no config yet.
    fn js_loads_for(&self, dir: &Path) -> JsLoads {
        let cell = memo(&self.js_loads_for, &dir.to_path_buf());
        crate::once::init_once(&cell, || {
            let mut out = Vec::new();
            let mut walks: Vec<Arc<Observations>> = Vec::new();
            let mut d = Some(dir);
            while let Some(cur) = d {
                for (ai, (_, lang)) in self.adapters.iter().enumerate() {
                    if *lang != "js" {
                        continue;
                    }
                    let walk = self.config_for(ai, cur);
                    let (gov, _, levels) = &*walk;
                    for level in levels {
                        if !walks.iter().any(|w| Arc::ptr_eq(w, level)) {
                            walks.push(Arc::clone(level));
                        }
                    }
                    // A config above `cur` is the answer of that directory's walk.
                    if let Some(c) = gov.as_ref().filter(|c| c.parent() == Some(cur)) {
                        out.push(self.load(ai, c));
                    }
                }
                if cur == self.repo {
                    break;
                }
                d = cur.parent();
            }
            Arc::new((out, walks))
        })
        .clone()
    }

    /// Alias and haste-mock edges for import `spec` of a file in `dir`, under
    /// one project.
    fn alias_edges(
        &self,
        l: &Loaded,
        pi: usize,
        spec: &str,
        dir: &Path,
    ) -> (Vec<FwEdge>, Observations) {
        let p = &l.inputs.projects[pi];
        let hit = if p.resolution.aliases.is_empty() {
            None
        } else {
            p.resolution.apply(spec)
        };
        let mut haste: Vec<(String, PathBuf)> = Vec::new();
        if p.mocks.implicit_package_mocks
            && !p.mocks.haste_mocks.is_empty()
            && !is_path_request(spec)
        {
            // `haste_mocks` is sorted by name.
            let pkg = package_name(spec);
            for name in [spec, pkg] {
                let lo = p
                    .mocks
                    .haste_mocks
                    .partition_point(|(k, _)| k.as_str() < name);
                for (k, m) in &p.mocks.haste_mocks[lo..] {
                    if k != name {
                        break;
                    }
                    haste.push((k.clone(), m.clone()));
                }
                if pkg == spec {
                    break;
                }
            }
        }
        if hit.is_none() && haste.is_empty() {
            return Default::default();
        }
        // Keyed by directory even for absolute replacements: the resolution's
        // successors include the requesting side's `package.json` (module-type
        // scope), so a memo shared across directories handed one package's
        // manifest to every other package, and which one depended on thread
        // timing (nondeterministic shard keys). Haste mocks alone do not depend
        // on the directory.
        let dir_key = match &hit {
            None => PathBuf::new(),
            Some(_) => dir.to_path_buf(),
        };
        let key = l.inputs.config_file.clone().unwrap_or_default();
        let cell = memo(&self.alias_memo, &(key, pi, spec.to_owned(), dir_key));
        crate::once::init_once(&cell, || {
            let id = l.fw.id();
            let mut edges = Vec::new();
            let mut obs = Observations::default();
            if let Some(hit) = &hit {
                let source = p.resolution.aliases[hit.alias].source;
                for req in &hit.requests {
                    let r = self.resolve("js", req, dir);
                    obs.extend_from(&r.obs);
                    if !r.succ.is_empty() {
                        edges.push(FwEdge {
                            kind: RefKind::Require,
                            phase: Phase::Runtime,
                            // A mapper target above the repository
                            // (`<rootDir>/../shared/$1`) by its path from the root, not
                            // by the directories of this machine.
                            spec: format!(
                                "{id}:{source} {spec} -> {}",
                                turborepo_trace_core::fs::from_base(self.repo, req)
                            ),
                            succ: r.succ,
                            links: r.links,
                            uncertainty: None,
                        });
                    }
                }
            }
            for (name, mock) in &haste {
                let mock = fsx::normalize(mock);
                if self.tracer.fs().is_file(&mock) {
                    edges.push(FwEdge {
                        kind: RefKind::Require,
                        phase: Phase::Runtime,
                        spec: format!("{id}:haste_mock {name}"),
                        succ: vec![self.tracer.intern(&mock)],
                        links: Vec::new(),
                        uncertainty: None,
                    });
                } else {
                    obs.missing.push(mock);
                }
            }
            (edges, obs)
        })
        .clone()
    }

    /// For a jest/vitest config file that governs its directory: removes from
    /// its analysis the references that are values the config hands to the
    /// runner ([`turborepo_trace_frameworks::ConfigValues`]).
    ///
    /// The config is traced as a module like any file, and the tracer has to
    /// guess what a path expression is for: `path.resolve(__dirname,
    /// './src')` becomes an asset reference to every file below `src` (with
    /// an uncertainty saying so), `new URL('./stub.ts', import.meta.url)` a
    /// read of the stub. Every test has an edge to its config, so every
    /// test of the project then depends on all of `src`, and on every uncertain
    /// file in it. When the expression is the target of an alias, a root or
    /// a setup file, the adapter already produces what tests really load
    /// through it (alias edges on the importing files, discovery,
    /// setup-file edges), and the config module itself reads nothing.
    ///
    /// A reference is removed only when all of this holds:
    /// - it is a path value, not a tracked file-system call: a deep-pass asset
    ///   ([`RefKind::Asset`]) or the `new URL(x, import.meta.url)` form;
    /// - its path is the value of fields the adapter models and of nothing else
    ///   in the exported config, and did not reach code the evaluator could not
    ///   follow;
    /// - it is not written inside a range the evaluator could not follow.
    ///
    /// References inside a function the adapter models (an alias
    /// `customResolver` that is Node resolution of its argument) are
    /// removed too: what the function resolves is the alias edge.
    ///
    /// Reporter references (the values of `reporters` and the like, and imports
    /// used only there) become data edges ([`RefFlags::DATA`]): the runner
    /// runs a reporter to report results, no test executes it, so its
    /// uncertainty is not the tests' and its changes still are.
    fn drop_config_values(&self, path: &Path, dir: &Path, a: &mut FileAnalysis, out: &mut FileFw) {
        let Some(cands) = self.js_config_index().get(dir) else {
            return;
        };
        let mut values: Vec<&'static turborepo_trace_frameworks::ConfigValues> = Vec::new();
        for (ai, c) in cands {
            if c != path || self.config_for(*ai, dir).0.as_deref() != Some(path) {
                continue;
            }
            let l = self.load(*ai, c);
            // The helpers the config loads decide its values: key inputs of the config
            // file.
            out.key_reads.extend(l.config_files.iter().cloned());
            values.extend(
                l.inputs
                    .projects
                    .iter()
                    .filter_map(|p| p.global.values.as_ref())
                    .filter(|v| v.file == path),
            );
        }
        if values.is_empty() {
            return;
        }
        let Some(ex) = a.extraction.as_ref() else {
            return;
        };
        let src = self.tracer.fs().read_cached(path).ok();
        let within = |ranges: &[(u32, u32)], s: Span| {
            ranges.iter().any(|&(a, b)| a <= s.start && s.end <= b)
        };
        let mut dropped: Vec<usize> = Vec::new();
        let mut data: Vec<usize> = Vec::new();
        for (gi, g) in a.groups.iter().enumerate() {
            let Some(r) = g
                .ref_index
                .filter(|&i| i != FRAMEWORK_LEVEL)
                .and_then(|i| ex.refs.get(i as usize))
            else {
                continue;
            };
            // A reporter: the runner loads it to report results, no test executes it.
            if values.iter().any(|v| {
                v.runner
                    .iter()
                    .any(|&(a, b)| a <= r.span.start && r.span.start < b)
            }) {
                data.push(gi);
                continue;
            }
            // Inside a function the adapter models.
            if values.iter().any(|v| within(&v.modelled, r.span)) {
                dropped.push(gi);
                continue;
            }
            let is_value = match r.kind {
                RefKind::Asset => true,
                RefKind::FsRead => src.as_deref().is_some_and(|s| {
                    let at = s.get(r.span.start as usize..).unwrap_or_default();
                    at.strip_prefix(b"new")
                        .is_some_and(|rest| rest.trim_ascii_start().starts_with(b"URL"))
                }),
                _ => false,
            };
            let turborepo_trace_core::Specifier::Literal(spec) = &r.spec else {
                continue;
            };
            if !is_value || r.base != turborepo_trace_core::Base::FileDir {
                continue;
            }
            let abs = fsx::normalize(&dir.join(spec.as_ref()));
            if values
                .iter()
                .any(|v| v.paths.contains(&abs) && !within(&v.opaque, r.span))
            {
                dropped.push(gi);
            }
        }
        for gi in dropped {
            let g = &mut a.groups[gi];
            g.edges = g.edges.start..g.edges.start;
            g.observations = Observations::default();
        }
        for gi in data {
            for e in &mut a.edges[a.groups[gi].edges.clone()] {
                e.edge.flags |= RefFlags::DATA;
            }
        }
    }

    /// Adds `path`'s framework inputs to its analysis `a`: appends edges and
    /// one [`FRAMEWORK_LEVEL`] group, and clears the uncertainty of
    /// unresolved imports that a module mapper resolves.
    pub(crate) fn augment(&self, path: &Path, a: &mut FileAnalysis) -> FileFw {
        let mut out = FileFw::default();
        let Some(lang) = a.extraction.as_ref().map(|e| e.lang.0) else {
            return out;
        };
        let Some(dir) = path.parent() else {
            return out;
        };
        let mut edges: Vec<FwEdge> = Vec::new();
        let mut obs = Observations::default();
        let mut uncs: Vec<FwUnc> = Vec::new();
        // The walks that said which configs are consulted for the file.
        let mut walks: Vec<Arc<Observations>> = Vec::new();

        // 0. A runner config: its path values that the adapter models are not its
        //    reads.
        if lang == "js" {
            self.drop_config_values(path, dir, a, &mut out);
        }

        // 1. Tests.
        for ai in 0..self.adapters.len() {
            let (fw, fw_lang) = self.adapters[ai];
            if fw_lang != lang || !candidate(fw.id(), path) {
                continue;
            }
            let walk = self.config_for(ai, dir);
            let (config, cobs, levels) = &*walk;
            // Which config it is (or that there is none) is the walk's answer.
            walks.extend(levels.iter().cloned());
            let Some(config) = config else { continue };
            let l = self.load(ai, config);
            // Whether the file is a test is the load's answer, also when it is no.
            l.consulted(&mut out, &mut obs);
            // A discovery that was chosen by the files that are there: the choice is a
            // record of the files it decides for.
            for (pi, choice) in l.choices.iter().enumerate() {
                if let Some((other, set, crossed)) = choice
                    && let Some(c) = l.inputs.projects[pi].discovery.choice.as_deref()
                    && c.decides(l.matchers[pi].is_test(path), other.is_test(path))
                {
                    key_observation(&mut obs, set);
                    if !crossed.is_empty() {
                        out.reached.push((PathBuf::new(), crossed.clone()));
                    }
                }
            }
            // The runner runs the file once in every project whose patterns match it, each
            // time with that project's setup files, transforms, environment and mocks: the
            // inputs of every one of them are inputs of the test.
            let mut matched: Vec<usize> = (0..l.matchers.len())
                .filter(|&pi| l.matchers[pi].is_test(path))
                .collect();
            if first_project_only() {
                matched.truncate(1);
            }
            if matched.is_empty() {
                continue;
            }
            if matched.len() > 1 && turborepo_trace_core::timing::enabled() {
                SHARED_TESTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let mut cwds: Vec<PathBuf> = Vec::new();
            for &pi in &matched {
                cwds.extend(test_cwds(
                    fw,
                    fw_lang,
                    l.inputs,
                    &l.inputs.projects[pi],
                    path,
                ));
            }
            cwds.sort();
            cwds.dedup();
            out.tests.push((fw.id(), cwds));
            obs.extend_from(cobs);
            // What the file does with the mock APIs is a fact of its extraction (none: it
            // uses none of them): the adapter does not scan the source for it again.
            let mocks = a.extraction.as_ref().map(|e| {
                e.facts
                    .iter()
                    .find_map(|f| MockScan::decode(f))
                    .unwrap_or_default()
            });
            // Per project: the modules its run of the test replaces with mocks.
            let mut replaced: Vec<std::collections::BTreeSet<String>> = Vec::new();
            for &pi in &matched {
                let st = self.project_state(&l, pi);
                let mut own: Vec<FwEdge> = st.edges.to_vec();
                obs.extend_from(&st.obs);
                uncs.extend(st.uncertainties.iter().cloned());
                out.reached.extend(st.reached.iter().cloned());
                let project: &Project = &l.inputs.projects[pi];
                let res = Resolver::new(self, fw_lang);
                let (ti, tobs) = self.recorded(|| {
                    let src = fsx::read(path).unwrap_or_default();
                    fw.test_inputs_scanned(project, path, &src, &res, mocks.as_ref())
                });
                obs.extend(tobs);
                out.reached.extend(res.reached.take());
                obs.extend(res.obs.into_inner());
                self.test_input_edges(fw_lang, fw.id(), &ti, &mut own, &mut obs, &mut uncs);
                replaced.push(
                    own.iter()
                        .filter_map(|e| e.spec.strip_prefix("mock:replaces "))
                        .map(|m| m.split(" @ ").next().unwrap_or(m).to_owned())
                        .collect(),
                );
                edges.append(&mut own);
            }
            // A module that one project's run replaces and another's loads for real is
            // loaded: the replacements of a test hold only when every run makes them.
            if replaced.windows(2).any(|w| w[0] != w[1]) {
                edges.push(marker(format!(
                    "mock:hazard the projects that run the test replace other modules @ {}:0",
                    self.display(path)
                )));
            }
        }

        // 2. Module mappers and haste mocks of the js projects whose root contains the
        //    file.
        if lang == "js" {
            // Every config that was asked whether one of its projects contains the file:
            // the answer is a function of its load, also when it is no (a `rootDir`
            // elsewhere).
            let loads = self.js_loads_for(dir);
            walks.extend(loads.1.iter().cloned());
            for l in &loads.0 {
                l.consulted(&mut out, &mut obs);
            }
            let projects = self.js_projects_for(dir);
            if !projects.is_empty() {
                for i in 0..a.edges.len() {
                    let e = &a.edges[i].edge;
                    if !matches!(
                        e.kind,
                        RefKind::Import
                            | RefKind::Require
                            | RefKind::DynamicImport
                            | RefKind::ReExport
                    ) || e.spec.is_empty()
                        || e.spec.contains('\n')
                    {
                        continue;
                    }
                    // A reference with several values (`a | b`: a table of module names, a
                    // conditional) is mapped value by value: each one is a request the
                    // runner's resolver sees on its own.
                    let whole = e.spec.to_string();
                    let unresolved = a.edges[i].succ.is_empty();
                    let mut all_by_alias = true;
                    for spec in whole.split(" | ") {
                        if spec.is_empty() || spec.contains(' ') {
                            all_by_alias = false;
                            continue;
                        }
                        if spec.contains('*') {
                            all_by_alias = false;
                            // A pattern (`'@shared/routes/' + name`, not a literal with a
                            // `*` in it) that nothing resolved, in a project with module
                            // mappers: a mapper may apply to what the pattern stands for,
                            // and mappers are not applied to patterns, so the load is open.
                            if unresolved
                                && !spec.starts_with(['.', '/'])
                                && !spec.contains(':')
                                && is_pattern_of(a, i, spec)
                                && let Some((l, _)) = projects.iter().find(|(l, pi)| {
                                    !l.inputs.projects[*pi].resolution.aliases.is_empty()
                                })
                            {
                                // A load: the module the mapper would name.
                                uncs.push(FwUnc::load(Uncertainty::new(
                                    turborepo_trace_core::UncertaintyKind::ConditionalResolution,
                                    Span::DUMMY,
                                    format!(
                                        "pattern '{spec}': a module mapper of the {} \
                                         configuration may apply to what it names; not expanded",
                                        l.fw.id()
                                    ),
                                )));
                            }
                            continue;
                        }
                        let spec = spec.to_string();
                        let mut resolved_by_alias = false;
                        for (l, pi) in projects.iter() {
                            let (es, o) = self.alias_edges(l, *pi, &spec, dir);
                            resolved_by_alias |= es.iter().any(|x| x.spec.contains(" -> "));
                            edges.extend(es);
                            obs.extend(o);
                            // A module a loader plugin of the runner compiles: what the plugin
                            // reads for it (a shader's own imports) is traced when a language
                            // plugin reads the module (its import lines are its references,
                            // and one it cannot read or resolve is its own record). When none
                            // does, this importer is uncertain. Nothing else is, because of
                            // the plugin.
                            // A request whose export conditions a custom resolver changes: it
                            // matters only when the request resolves into the repository (an
                            // installed package is one unit whatever file of it is picked).
                            let proj = &l.inputs.projects[*pi];
                            if proj.resolution.condition_requests.contains(&spec)
                                && a.edges[i].succ.iter().any(|&f| {
                                    let p = self.tracer.path_of(f);
                                    p.strip_prefix(self.repo).is_ok_and(|rel| {
                                        !rel.components().any(|c| c.as_os_str() == "node_modules")
                                    }) && p.file_name().is_some_and(|n| n != "package.json")
                                })
                            {
                                uncs.push(FwUnc::load(Uncertainty::new(
                                    turborepo_trace_core::UncertaintyKind::ConditionalResolution,
                                    Span::DUMMY,
                                    format!(
                                        "'{spec}': the {} resolver changes its export conditions; \
                                         another file of the package may be loaded",
                                        l.fw.id()
                                    ),
                                )));
                            }
                            let id = spec.split_once('?').map_or(spec.as_str(), |(p, _)| p);
                            let traced = !a.edges[i].succ.is_empty()
                                && a.edges[i].succ.iter().all(|&f| {
                                    self.tracer
                                        .languages()
                                        .by_extension(&self.tracer.path_of(f))
                                        .is_some()
                                });
                            for ld in &l.inputs.projects[*pi].resolution.loaders {
                                if !traced && ld.suffixes.iter().any(|s| id.ends_with(s.as_str())) {
                                    uncs.push(FwUnc::read(Uncertainty::new(
                                    turborepo_trace_core::UncertaintyKind::ConditionalResolution,
                                    Span::DUMMY,
                                    format!(
                                        "'{spec}' is compiled by the {} plugin '{}'; files it reads for the module are not traced",
                                        l.fw.id(),
                                        ld.plugin
                                    ),
                                )));
                                }
                            }
                        }
                        all_by_alias &= resolved_by_alias;
                    }
                    if unresolved && all_by_alias {
                        a.edges[i].edge.uncertainty = None;
                    }
                }
            }
        }
        // 3. What a file that is not a test does with the mock APIs of the runners
        //    (from its
        // extraction's fact): the real loads and hazards that undo the replacements of
        // the tests that load it, and the mocks and real modules it loads or
        // registers.
        if lang == "js"
            && out.tests.is_empty()
            && let Some(scan) = a
                .extraction
                .as_ref()
                .and_then(|e| e.facts.iter().find_map(|f| MockScan::decode(f)))
        {
            let projects = self.js_projects_for(dir);
            let res = Resolver::new(self, "js");
            let (ti, tobs) = self.recorded(|| {
                let mut ti = TestInputs {
                    test: path.to_path_buf(),
                    ..TestInputs::default()
                };
                if projects.is_empty() {
                    mock_inputs(None, "", path, &scan, false, &res, &mut ti);
                }
                for (l, pi) in projects.iter() {
                    let p = &l.inputs.projects[*pi];
                    mock_inputs(Some(p), l.fw.id(), path, &scan, false, &res, &mut ti);
                }
                ti
            });
            obs.extend(tobs);
            out.reached.extend(res.reached.take());
            // A manual mock the file registers is found by looking next to its module, and
            // only named in a marker: reached, like what a resolution found.
            out.reached
                .extend(ti.lazy_mocks.iter().map(|l| (l.mock.clone(), Vec::new())));
            obs.extend(res.obs.into_inner());
            let id = projects.first().map_or("jest", |(l, _)| l.fw.id());
            // The markers (among them the manual mocks the file registers without a
            // factory, which the derived layer turns into edges from their modules), and
            // the mocks the file loads itself (`requireMock`, a mock found by name).
            self.test_input_edges("js", id, &ti, &mut edges, &mut obs, &mut uncs);
            // A real load when the file's code runs is a dependency of the file (a test's
            // own are edges of its framework inputs).
            for p in &ti.loaded_eager {
                if self.tracer.fs().is_file(p) {
                    edges.push(FwEdge {
                        kind: RefKind::Require,
                        phase: Phase::Runtime,
                        spec: format!("{id}:require_actual {}", self.display(p)),
                        succ: vec![self.tracer.intern(p)],
                        links: Vec::new(),
                        uncertainty: None,
                    });
                }
            }
        }
        // What each walk accessed decides the file's framework inputs, also when it
        // found no config and when the file is no test: a key observation of
        // the file.
        for walk in &walks {
            key_observation(&mut obs, walk);
        }
        // A set that this pass did not make reached the file through a resolution it
        // asked of the tracer (the tsconfig chain of the directory a config's
        // request is resolved from): the tracer's set, which the file names
        // like the sets of its own resolutions, in its file-level group. The
        // store labels a set by the group that
        // names it ([`crate::shard::KeyGroupRec::framework`]), and a tsconfig set that
        // was labelled as this pass's would be none for a query that asks which
        // files observed a config ([`crate::Graph::key_observers`]).
        let mut theirs: Vec<Arc<Observations>> = Vec::new();
        obs.key.retain(|k| {
            let own = self.own_sets.contains_key(&(Arc::as_ptr(k) as usize));
            if !own && !theirs.iter().any(|t| Arc::ptr_eq(t, k)) {
                theirs.push(Arc::clone(k));
            }
            own
        });
        if !theirs.is_empty() {
            let at = match a.groups.iter().position(|g| g.ref_index.is_none()) {
                Some(at) => at,
                None => {
                    let end = a.edges.len();
                    a.groups.push(turborepo_trace_core::RefGroup {
                        ref_index: None,
                        edges: end..end,
                        observations: Observations::default(),
                    });
                    a.groups.len() - 1
                }
            };
            let named = &mut a.groups[at].observations.key;
            for set in theirs {
                if !named.iter().any(|k| Arc::ptr_eq(k, &set)) {
                    named.push(set);
                }
            }
        }
        out.key_reads.sort();
        out.key_reads.dedup();
        out.reached.sort();
        out.reached.dedup();

        if edges.is_empty() && obs.is_empty() && uncs.is_empty() {
            return out;
        }
        let file = self.tracer.intern(path);
        let first = a.edges.len();
        let mut seen = std::collections::HashSet::new();
        for e in edges {
            if !seen.insert((e.spec.clone(), e.succ.clone())) {
                continue;
            }
            a.edges.push(AnalyzedEdge {
                edge: Edge {
                    from: file,
                    to: e
                        .succ
                        .first()
                        .map_or(Target::Unresolved, |&f| Target::File(f)),
                    kind: e.kind,
                    phase: e.phase,
                    // `<framework>:config <path>`: loaded by the runner, not by the test.
                    flags: if e
                        .spec
                        .split_once(' ')
                        .is_some_and(|(label, _)| label.ends_with(":config"))
                    {
                        RefFlags::CONFIG
                    } else {
                        RefFlags::empty()
                    },
                    span: Span::DUMMY,
                    spec: e.spec.into(),
                    uncertainty: e.uncertainty,
                },
                succ: e.succ,
                links: e.links,
            });
        }
        // Uncertainties travel on unresolved marker edges (the store records edge
        // uncertainties on the file, with the effect of the edge's kind: a record about
        // which module the runner loads is a load, every other one a read).
        let mut useen = std::collections::HashSet::new();
        for FwUnc { unc: u, load } in uncs {
            if !useen.insert((u.kind, u.detail.clone())) {
                continue;
            }
            a.edges.push(AnalyzedEdge {
                edge: Edge {
                    from: file,
                    to: Target::Unresolved,
                    kind: if load {
                        RefKind::Require
                    } else {
                        RefKind::FsRead
                    },
                    phase: Phase::Runtime,
                    flags: RefFlags::empty(),
                    span: Span::DUMMY,
                    spec: format!("framework: {}", u.detail).into(),
                    uncertainty: Some(u),
                },
                succ: Vec::new(),
                links: Vec::new(),
            });
        }
        obs.dedup();
        a.groups.push(turborepo_trace_core::RefGroup {
            ref_index: Some(FRAMEWORK_LEVEL),
            edges: first..a.edges.len(),
            observations: obs,
        });
        out
    }
}

/// The tracer's language resolver behind the adapters' [`ModuleResolver`]: the
/// resolutions' observations are collected in `obs` (kept apart from the
/// adapter's own recording, whose listings are dropped).
struct Resolver<'f, 'a> {
    fw: &'f Fw<'a>,
    lang: &'static str,
    obs: std::cell::RefCell<Observations>,
    /// The resolutions whose observations are in `obs` already (a file resolves
    /// the same request for its mock, for the manual mock next to it and
    /// for `requireActual`).
    seen: std::cell::RefCell<Vec<(String, PathBuf)>>,
    /// What those resolutions found ([`FileFw::reached`]).
    reached: std::cell::RefCell<Vec<Reached>>,
}

impl<'f, 'a> Resolver<'f, 'a> {
    fn new(fw: &'f Fw<'a>, lang: &'static str) -> Self {
        Resolver {
            fw,
            lang,
            obs: Default::default(),
            seen: Default::default(),
            reached: Default::default(),
        }
    }

    /// The memoized resolution of `request` from `from_dir`, read in place,
    /// with its observations added to `obs` once.
    fn with<R>(&self, request: &str, from_dir: &Path, f: impl FnOnce(&Resolved) -> R) -> R {
        let cell = self.fw.resolved_cell(self.lang, request, from_dir);
        let empty = Resolved::default();
        let r = cell.get().unwrap_or(&empty);
        let known = self
            .seen
            .borrow()
            .iter()
            .any(|(q, d)| q == request && d == from_dir);
        if !known {
            self.obs.borrow_mut().extend_from(&r.obs);
            // Whatever the caller does with the result, the file depends on what was found.
            let path = |f: &FileId| self.fw.tracer.path_of(*f).to_path_buf();
            let links: Vec<PathBuf> = r.links.iter().map(path).collect();
            self.reached
                .borrow_mut()
                .extend(r.succ.iter().map(|f| (path(f), links.clone())));
            self.seen
                .borrow_mut()
                .push((request.to_owned(), from_dir.to_path_buf()));
        }
        f(r)
    }
}

impl ModuleResolver for Resolver<'_, '_> {
    fn resolve(&self, request: &str, from_dir: &Path) -> Option<PathBuf> {
        self.with(request, from_dir, |r| {
            r.succ
                .first()
                .map(|&f| self.fw.tracer.path_of(f).to_path_buf())
        })
    }

    fn resolve_all(&self, request: &str, from_dir: &Path) -> Vec<PathBuf> {
        self.with(request, from_dir, |r| self.all(r))
    }
}

impl Resolver<'_, '_> {
    fn all(&self, r: &Resolved) -> Vec<PathBuf> {
        r.succ
            .iter()
            .map(|&f| self.fw.tracer.path_of(f).to_path_buf())
            .collect()
    }
}

/// Whether `text`, one value of the specifier of edge `edge` of `a` as
/// displayed, is a pattern of the reference (a specifier known in part), not a
/// literal that happens to contain `*` (a glob string a plugin of the runner
/// expands).
fn is_pattern_of(a: &FileAnalysis, edge: usize, text: &str) -> bool {
    fn any(spec: &turborepo_trace_core::Specifier<'_>, text: &str) -> bool {
        match spec {
            turborepo_trace_core::Specifier::Pattern(_) => spec.to_string() == text,
            turborepo_trace_core::Specifier::Alternatives(alts) => {
                alts.iter().any(|s| any(s, text))
            }
            _ => false,
        }
    }
    let Some(ex) = a.extraction.as_ref() else {
        return false;
    };
    a.groups
        .iter()
        .find(|g| g.edges.contains(&edge))
        .and_then(|g| g.ref_index)
        .and_then(|ri| ex.refs.get(ri as usize))
        .is_some_and(|r| any(&r.spec, text))
}

/// The working directories a test of `project` may run in: the framework's rule
/// ([`Framework::test_cwds`]: the package directory of the test file for `go
/// test`, the config directory and project root for the others), and for jest
/// and vitest:
///
/// The runner does not change directory per test file, so `process.cwd()` in a
/// test is the directory the runner was started in:
/// - the directory of the project's config file: `jest` / `vitest` started in
///   the package (package scripts, `pnpm --filter`, `turbo run test`), the
///   usual monorepo setup;
/// - the project's `rootDir` / `root` when it differs (vitest's forks pool
///   changes into the project root; a config with `rootDir: '..'` is started
///   above its directory);
/// - the directory of the config that lists the project (`projects`, a vitest
///   workspace): one runner started there runs every project.
///
/// The repository root (a runner started there with `--projects` or a path
/// filter) is not listed: the tracer already resolves
/// working-directory-relative references against it on the referencing file
/// itself. A runner started anywhere else (a subdirectory of the package) is
/// not modelled.
fn test_cwds(
    fw: &dyn Framework,
    lang: &str,
    inputs: &FrameworkInputs,
    project: &Project,
    test: &Path,
) -> Vec<PathBuf> {
    let mut out = fw.test_cwds(project, test);
    if lang == "js"
        && let Some(dir) = inputs.config_file.as_deref().and_then(Path::parent)
    {
        out.push(dir.to_path_buf());
    }
    for d in &mut out {
        *d = fsx::normalize(d);
    }
    out.sort();
    out.dedup();
    out
}

/// `TURBO_TRACE_DEBUG_FW_PROJECTS=first`: a test that several projects of its
/// config match gets the inputs of the first one only, as before (for A/B
/// measurements; graphs built either way are not interchangeable).
fn first_project_only() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("FW_PROJECTS").is_some_and(|v| v == "first")
    });
    *ON
}

/// Whether `path` can be a test of framework `id` (cheap name filter before
/// config lookup).
fn candidate(id: &str, path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match id {
        "gotest" => name.ends_with("_test.go"),
        _ => !name.starts_with("__turbotrace_"),
    }
}

/// `@scope/name` or `name` of a bare specifier.
fn package_name(spec: &str) -> &str {
    let mut it = spec.match_indices('/');
    let cut = if spec.starts_with('@') {
        it.nth(1)
    } else {
        it.next()
    };
    match cut {
        Some((i, _)) => &spec[..i],
        None => spec,
    }
}

/// A marker edge: unresolved, read by its specifier only.
fn marker(spec: String) -> FwEdge {
    FwEdge {
        kind: RefKind::Manifest,
        phase: Phase::Runtime,
        spec,
        succ: Vec::new(),
        links: Vec::new(),
        uncertainty: None,
    }
}

fn code_kind(lang: &str) -> RefKind {
    if lang == "js" {
        RefKind::Require
    } else {
        RefKind::Import
    }
}

fn role_kind(lang: &str, role: InputRole) -> (RefKind, Phase) {
    match role {
        InputRole::TestData => (RefKind::FsRead, Phase::Runtime),
        InputRole::CompiledWithTest | InputRole::LinkedCrate | InputRole::BuildScript => {
            (code_kind(lang), Phase::Build)
        }
        _ => (code_kind(lang), Phase::Runtime),
    }
}

fn role_name(role: InputRole) -> String {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{role:?}"))
}

/// Whether a config uncertainty is attached to every test of the project.
///
/// Not attached: environment reads the config evaluator noted without a field
/// (`verbose: process.env.CI === "1"`). They are redundant: an
/// environment-dependent value that reaches a field the adapter consumes (setup
/// files, mappers, roots, test patterns, transforms, ...) is reported again by
/// the adapter's reader with that field (`Reader::report_unknown`), and
/// environment-dependent branches are evaluated as alternatives whose inputs
/// are all recorded. A field-less note on a reporting option like `verbose` or
/// `silent` cannot change which files a test loads, yet attached to every test
/// it made every test of the project "affected" under the default policy
/// (turborepo's shared jest preset). Every other kind, and every field-tagged
/// report, still counts.
fn counts_for_tests(u: &turborepo_trace_frameworks::FrameworkUncertainty) -> bool {
    !(u.field.is_none() && u.kind == turborepo_trace_core::UncertaintyKind::EnvDependent)
}

/// An uncertainty of the framework pass, and whether it is the record of a
/// module load with an unknown target (the runner may load another module than
/// the graph has: a mapper applied to a pattern, plugins, a custom resolver;
/// [`turborepo_trace_frameworks::FrameworkUncertainty::is_load`]) instead of a
/// file read. The bound for such loads then applies to it
/// (`turborepo_trace_graph::loads`: the packages the test's package declares);
/// as a read it is open.
#[derive(Clone)]
struct FwUnc {
    unc: Uncertainty,
    load: bool,
}

impl FwUnc {
    fn load(unc: Uncertainty) -> Self {
        // `TURBO_TRACE_DEBUG_FW_LOADS=0`: every record of the framework pass is a read,
        // as before (for A/B measurements; graphs built either way are not
        // interchangeable).
        static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            turborepo_trace_core::debug::var_os("FW_LOADS").is_none_or(|v| v != "0")
        });
        FwUnc { unc, load: *ON }
    }

    fn read(unc: Uncertainty) -> Self {
        FwUnc { unc, load: false }
    }
}

fn fw_unc(u: &turborepo_trace_frameworks::FrameworkUncertainty) -> FwUnc {
    let field = u
        .field
        .as_deref()
        .map(|f| format!("{f}: "))
        .unwrap_or_default();
    let unc = Uncertainty::new(
        u.kind,
        Span::DUMMY,
        format!("{}:{} {field}{}", u.file.display(), u.line, u.detail),
    );
    if u.is_load() {
        FwUnc::load(unc)
    } else {
        FwUnc::read(unc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The set [`ConfigPaths::digest_for`] hashes, computed without
    /// memoization.
    fn reference_digest(c: &ConfigPaths, root: &str) -> u128 {
        let mut set: Vec<&str> = c.below(root).iter().map(String::as_str).collect();
        let mut dir = root;
        loop {
            if let Some(ix) = c.by_dir.get(dir) {
                for &i in ix {
                    set.push(&c.paths[i]);
                    if c.is_multi(i) {
                        set.extend(c.below(dir).iter().map(String::as_str));
                    }
                }
            }
            if dir.is_empty() {
                break;
            }
            dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
        }
        set.sort_unstable();
        set.dedup();
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        h.update(INTEGRATION_VERSION.as_bytes());
        for f in set {
            h.update(&(f.len() as u64).to_le_bytes());
            h.update(f.as_bytes());
        }
        h.digest128()
    }

    #[test]
    fn memoized_config_digests_match_the_reference() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let w = |rel: &str, body: &str| {
            let p = r.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        // A root multi config, a nested multi config, plain configs at several depths.
        w(
            "jest.config.js",
            "module.exports = { projects: ['apps/*'] };",
        );
        w("apps/jest.config.js", "module.exports = {};");
        w("apps/a/jest.config.js", "module.exports = {};");
        w(
            "apps/b/vitest.config.ts",
            "export default { test: { workspace: [] } };",
        );
        w("apps/b/c/jest.config.js", "module.exports = {};");
        w("libs/x/jest.config.js", "module.exports = {};");
        let mut files: Vec<String> = [
            "jest.config.js",
            "apps/jest.config.js",
            "apps/a/jest.config.js",
            "apps/b/vitest.config.ts",
            "apps/b/c/jest.config.js",
            "libs/x/jest.config.js",
        ]
        .map(String::from)
        .to_vec();
        files.sort();
        let roots = [
            "",
            "apps",
            "apps/a",
            "apps/b",
            "apps/b/c",
            "apps/b/c/d",
            "libs/x",
            "libs",
            "other",
        ];
        for with_root_multi in [true, false] {
            if !with_root_multi {
                w("jest.config.js", "module.exports = {};");
            }
            let c = ConfigPaths::new(r, &files);
            assert_eq!(c.paths.len(), 6, "every fixture file is a config");
            // Twice: computed, then memoized.
            for _ in 0..2 {
                for root in roots {
                    assert_eq!(
                        c.digest_for(root),
                        reference_digest(&c, root),
                        "root {root:?}, root multi {with_root_multi}"
                    );
                }
            }
        }
    }

    #[test]
    fn package_names() {
        assert_eq!(package_name("lodash/fp"), "lodash");
        assert_eq!(package_name("@a/b/c"), "@a/b");
        assert_eq!(package_name("x"), "x");
    }
}
