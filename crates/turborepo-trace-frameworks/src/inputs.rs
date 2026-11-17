//! The neutral data model adapters produce ([`FrameworkInputs`],
//! [`TestInputs`]).
//!
//! Everything here is language-neutral: paths are absolute, module requests
//! carry the directory they are resolved from, and patterns say which syntax
//! they use. The graph store and `affected` turn these into nodes, edges,
//! probes and memberships.

use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};
use turborepo_trace_core::UncertaintyKind;

use crate::pattern::{CompiledPattern, JsRegex};

/// Everything one adapter contributes for one config file: one or more
/// projects.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FrameworkInputs {
    /// Adapter id (`"jest"`, `"vitest"`).
    pub framework: &'static str,
    /// The config file that was loaded (the root of a multi-project setup).
    pub config_file: Option<PathBuf>,
    /// Projects (one unless the config uses `projects` / a workspace).
    pub projects: Vec<Project>,
    /// Problems that apply to every project (unreadable or unevaluable root
    /// config).
    pub uncertainties: Vec<FrameworkUncertainty>,
}

/// One test project: a config with its own root, inputs, resolution and
/// discovery.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Project {
    /// Display name (`displayName` / `test.name`), else the root directory.
    pub name: String,
    /// The config file the project's options come from (`None` for an inline
    /// project or defaults).
    pub config_file: Option<PathBuf>,
    /// `rootDir` (jest) / `root` (vitest).
    pub root_dir: PathBuf,
    /// Inputs of every test in the project.
    pub global: GlobalInputs,
    /// Resolution changes the runner applies to every module in the project.
    pub resolution: ResolutionOverrides,
    /// Which files are tests.
    pub discovery: TestDiscovery,
    /// How per-test mocks are found.
    pub mocks: MockPolicy,
    /// Where snapshots live.
    pub snapshots: SnapshotPolicy,
    /// Values the adapter could not read statically. The config file stays an
    /// input of every test either way.
    pub uncertainties: Vec<FrameworkUncertainty>,
}

impl Project {
    /// The working directories a test of this project may run in, which is what
    /// `process.cwd()` returns in test code and its helpers (P11:
    /// `require(process.cwd() + '/src/routes')`). The runner is started
    /// from the package that owns the config, so the config file's
    /// directory comes first; `rootDir` (`root`) is added when it differs
    /// (a `rootDir: 'src'` config still runs from the package directory, but
    /// resolving against both keeps the model sound). Projects without a
    /// config file run from their root. Consumers resolve a cwd-relative
    /// reference of a file in a test's closure against each of these and
    /// record the probes; outside a test context the reference
    /// stays `cwd_dependent`.
    pub fn test_cwds(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(dir) = self.config_file.as_deref().and_then(Path::parent) {
            out.push(dir.to_path_buf());
        }
        if !out.contains(&self.root_dir) {
            out.push(self.root_dir.clone());
        }
        out
    }
}

/// Inputs shared by every test of a project.
#[derive(Clone, Debug, Default, Serialize)]
pub struct GlobalInputs {
    /// Config files read: the config, local helpers it loads, a `package.json`
    /// holding the config. Changing any of them affects every test.
    pub config_files: Vec<PathBuf>,
    /// The subset of [`GlobalInputs::config_files`] written in code: trace them
    /// as JS entries so that everything they import is an input too.
    pub js_entries: Vec<PathBuf>,
    /// Modules loaded for every test: setup files, global setup and teardown,
    /// the test environment, transformers, a snapshot resolver.
    pub entries: Vec<InputRef>,
    /// Files every test depends on without importing them. Currently always
    /// empty: mocks registered in setup files are found by calling
    /// [`crate::Framework::test_inputs`] on each setup-file entry and
    /// adding the result to every test of the project.
    pub edges: Vec<ImplicitEdge>,
    /// Files the runner reads for every test without loading them as part of
    /// the test's module graph: an input by content only. What such a file
    /// imports is not an input (`next/jest` reads four fields of
    /// `next.config.*`; the config's own imports reach through the
    /// application).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<PathBuf>,
    /// Missing paths whose creation changes the project (a second config file
    /// name next to the config).
    pub probes: Vec<PathBuf>,
    /// Directory listings the project depends on (`__mocks__` directories in
    /// `roots`).
    pub memberships: Vec<MembershipSpec>,
    /// The package the project's tests belong to (Go module, crate, Python
    /// project), when the adapter knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageRef>,
    /// What a code config hands to the runner as plain values (bookkeeping for
    /// the graph, not part of the serialized inputs).
    #[serde(skip)]
    pub values: Option<ConfigValues>,
}

/// Parts of a code config that are values handed to the runner, not file reads
/// of the config module.
///
/// A config file is also traced as a module, and a general tracer has to guess
/// what a path expression is for: `path.resolve(__dirname, './src')` may be
/// read as a directory, so it includes every file below it. In a runner config
/// most such expressions are values of fields the adapter models: an alias
/// target, a root directory, a setup file. What tests load through those fields
/// is in the adapter's output (alias edges on the importing files, discovery,
/// setup-file edges), and the config module itself reads nothing. The graph
/// drops the tracer's guess for exactly these expressions, so that a test
/// depends on the files it loads through an alias and not on every file of the
/// aliased directory.
///
/// A path qualifies only when the evaluator saw every use of it: it is not the
/// value of a field the adapter does not model, and it did not reach code the
/// evaluator could not follow (an untracked helper call, a plugin's options, a
/// function nobody called).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigValues {
    /// The config file the byte ranges refer to.
    pub file: PathBuf,
    /// Absolute paths (normalized) that are values of modelled fields only.
    pub paths: Vec<PathBuf>,
    /// Byte ranges of function values the adapter models (a vite alias
    /// `customResolver` that is Node's `require.resolve`): references
    /// inside are the adapter's business.
    pub modelled: Vec<(u32, u32)>,
    /// Byte ranges the evaluator could not follow: references inside are kept.
    pub opaque: Vec<(u32, u32)>,
    /// Byte ranges whose references name modules the runner itself runs to
    /// report results (reporters): the values of the reporter fields, and
    /// the import declarations whose bindings are used nowhere else. A
    /// reporter observes the run. It is not part of a test's module graph,
    /// and what it loads or reads cannot change a test's outcome, so
    /// the graph marks these references as data: their targets stay inputs (an
    /// edit of the reporter still selects the project's tests), but
    /// uncertainty below them does not make the tests uncertain.
    pub runner: Vec<(u32, u32)>,
    /// Helper modules the config evaluator loaded only for such an import, with
    /// what they load in turn: not config inputs of the tests.
    pub runner_files: Vec<PathBuf>,
}

/// Per-test inputs, on top of the project's [`GlobalInputs`].
#[derive(Clone, Debug, Default, Serialize)]
pub struct TestInputs {
    /// The test file.
    pub test: PathBuf,
    /// Extra entries: per-file environments (`@jest-environment`), modules
    /// loaded through `jest.requireActual` / `vi.importActual`, modules
    /// automocked from their source.
    pub entries: Vec<InputRef>,
    /// Implicit edges: snapshot files, manual mocks.
    pub edges: Vec<ImplicitEdge>,
    /// Missing paths whose creation affects the test (manual mock candidates).
    pub probes: Vec<PathBuf>,
    /// Directory memberships (the snapshot file inside `__snapshots__`).
    pub memberships: Vec<MembershipSpec>,
    /// Mock calls found in the test, for explanations.
    pub mocks: Vec<MockSite>,
    /// Modules the file replaces with a mock that does not load them
    /// ([`crate::mocks`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<Replaced>,
    /// Modules the file loads for real whatever is mocked (`requireActual`, an
    /// automock, a factory that takes the original).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub loaded_real: Vec<PathBuf>,
    /// Why a test that loads this file may load a module it replaced.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mock_hazards: Vec<MockHazard>,
    /// Manual mocks a file that is not a test registers without a factory
    /// ([`crate::mocks`]): the runner loads one when its module is first
    /// requested.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lazy_mocks: Vec<LazyMock>,
    /// The modules of [`TestInputs::loaded_real`] that load when the file's
    /// code runs (`requireActual`, `requireMock`), not when something
    /// requests the module (the source of an automock, a factory that takes
    /// the original): a file that is not a test depends on them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub loaded_eager: Vec<PathBuf>,
    /// Problems (computed mock paths, custom snapshot resolvers).
    pub uncertainties: Vec<FrameworkUncertainty>,
    /// The package the test belongs to, when the adapter knows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageRef>,
    /// Directories put on the module search path for this test only (pytest's
    /// import root under `--import-mode=prepend|append`); resolve the
    /// test's imports with them.
    #[serde(skip_serializing_if = "ImportRoots::is_empty")]
    pub import_roots: ImportRoots,
}

/// Why a module is an input.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRole {
    /// jest `setupFiles`, vitest `setupFiles`.
    SetupFile,
    /// jest `setupFilesAfterEnv`.
    SetupFileAfterEnv,
    /// `globalSetup`.
    GlobalSetup,
    /// `globalTeardown`.
    GlobalTeardown,
    /// The test environment module.
    TestEnvironment,
    /// A transformer (jest `transform`).
    Transform,
    /// jest `snapshotResolver`.
    SnapshotResolver,
    /// jest `preset` from a package.
    Preset,
    /// jest `resolver` (a custom resolver module).
    Resolver,
    /// jest `testRunner`.
    TestRunner,
    /// The runner package itself (`jest`, `vitest`), as the project resolves
    /// it: which instance of the package runs the tests decides what it
    /// loads for them. vitest imports its built-in environments (`jsdom`,
    /// `happy-dom`, `@edge-runtime/vm`) as its own peer dependencies, so a
    /// project whose runner is re-resolved to another peer variant runs its
    /// tests in another DOM implementation with no file of the project
    /// changed. The edge makes the runner's resolution an input of every test
    /// of the project, also of tests that never import the runner
    /// (globals).
    Runner,
    /// `jest.requireActual`, `vi.importActual`, `jest.createMockFromModule`.
    RequireActual,
    /// A module mocked without a factory or manual mock: the runner loads it to
    /// build the automock.
    Automocked,
    /// A config file that is also a module (traced as an entry).
    Config,
    /// pytest: a `conftest.py` between `confcutdir` and the test's directory.
    Conftest,
    /// pytest: a plugin module (`-p NAME` in `addopts`, `pytest_plugins`).
    Plugin,
    /// go test: a file compiled into the test binary with the test (the package
    /// directory's `.go`, `_test.go`, cgo and assembly files).
    CompiledWithTest,
    /// A module the test imports implicitly or through its package: go test
    /// imports of the directory's `_test.go` files, the package
    /// `__init__.py` files pytest imports with a test.
    ImportedByTest,
    /// A data file the test reads by convention (go test `testdata/**`).
    TestData,
    /// cargo test: the root of a crate linked into the test binary (the
    /// package's library for integration tests, dev-dependencies for unit
    /// tests).
    LinkedCrate,
    /// cargo test: the package's build script (traced as a Rust entry; its
    /// `rerun-if-changed` inputs are config files).
    BuildScript,
}

/// The package a project or test belongs to, in its ecosystem's terms.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct PackageRef {
    /// `npm`, `pypi`, `go` or `crates`.
    pub ecosystem: &'static str,
    /// Package name: the `package.json` name, the `pyproject.toml` project
    /// name, the Go module path, the crate name.
    pub name: String,
    /// Version, when the manifest declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The package root (the manifest's directory).
    pub root: PathBuf,
}

/// Directories a runner puts on the module search path for one test (pytest's
/// `--import-mode`): `front` entries go before the project's roots, `back`
/// entries after.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ImportRoots {
    /// Prepended, in order.
    pub front: Vec<PathBuf>,
    /// Appended, in order.
    pub back: Vec<PathBuf>,
}

impl ImportRoots {
    /// Whether no directory is added.
    pub fn is_empty(&self) -> bool {
        self.front.is_empty() && self.back.is_empty()
    }
}

/// What an input points at.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// A file path (absolute). It may not exist yet; then it is a probe.
    Path(PathBuf),
    /// A module request resolved from a directory with the language resolver
    /// (`jest-extended/all` from the root dir, `./setup` from the config dir).
    Module {
        /// The request.
        request: String,
        /// The directory it is resolved from.
        from_dir: PathBuf,
    },
}

/// An extra entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct InputRef {
    /// Why.
    pub role: InputRole,
    /// What.
    pub target: Target,
}

/// The kind of an [`ImplicitEdge`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A snapshot file the test reads.
    Snapshot,
    /// A manual mock (`__mocks__/...`) that replaces a module.
    ManualMock,
}

/// A file dependency that no import expresses.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ImplicitEdge {
    /// Kind.
    pub kind: EdgeKind,
    /// The file (exists).
    pub to: PathBuf,
    /// Explanation (`jest.mock('fs')`, `default snapshot path`).
    pub reason: String,
}

/// A directory membership: the listing of `dir` filtered by a glob.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct MembershipSpec {
    /// The directory (absolute).
    pub dir: PathBuf,
    /// Glob relative to `dir` (`**` covers subdirectories).
    pub pattern: String,
    /// Explanation.
    pub reason: String,
}

/// A module that a test or setup file replaces with a mock that does not load
/// it: a top-level `jest.mock` / `vi.mock` with an inline factory, or without
/// one when a manual mock stands next to the module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Replaced {
    /// The file the request resolves to (first-party).
    pub module: PathBuf,
    /// The call (`jest.mock`, `vi.mock`).
    pub call: String,
    /// The request as written.
    pub request: String,
    /// 1-based line of the call.
    pub line: u32,
    /// The request names the file by path: a relative or absolute path, or a
    /// request one of the project's aliases rewrites to one. The runner
    /// then replaces that very file whoever imports it. A bare request (a
    /// package name) goes through the package's manifest, where the
    /// runner's conditions may pick another file than the tracer's:
    /// only imports by a bare request are known to get the mock.
    pub by_path: bool,
    /// The manual mock that stands in; `None` for a factory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<PathBuf>,
}

/// A manual mock that a file registers with `jest.mock('<module>')` / `vi.mock`
/// without a factory. Nothing is loaded by the call: the runner loads the mock
/// when `module` is first requested afterwards, so only a test that also
/// requests `module` depends on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LazyMock {
    /// The manual mock (`<dir>/__mocks__/<file>`).
    pub mock: PathBuf,
    /// The module it stands in for (first-party).
    pub module: PathBuf,
}

/// A use of the mock APIs after which a replaced module may be loaded for real,
/// and static analysis cannot say which.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MockHazard {
    /// What (`jest.requireActual(...) of a computed module`).
    pub what: String,
    /// 1-based line.
    pub line: u32,
}

/// A mock call in a test file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MockSite {
    /// The call (`jest.mock`, `vi.importActual`, ...).
    pub call: String,
    /// The module request.
    pub request: String,
    /// A factory was passed (no manual mock is used).
    pub factory: bool,
    /// 1-based line.
    pub line: u32,
}

/// How a module request is rewritten before resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AliasMatcher {
    /// A JS regex whose match replaces the whole request (jest
    /// `moduleNameMapper` keys); replacements use `$1`-style group
    /// references.
    Regex(JsRegex),
    /// A JS regex applied as `request.replace(regex, replacement)` (vite `{
    /// find: /re/ }`): only the matched part is replaced, with JS
    /// replacement patterns (`$1`, `$&`, ...).
    Replace(JsRegex),
    /// Vite string alias: the request equals the key or starts with `key/`; the
    /// key prefix is replaced.
    Prefix(String),
}

/// One ordered alias. The first matching alias wins; its replacements are tried
/// in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Alias {
    /// What matches.
    pub matcher: AliasMatcher,
    /// Replacement candidates (absolute paths after `<rootDir>` substitution,
    /// or module requests).
    pub replacements: Vec<String>,
    /// Which config field it came from.
    pub source: &'static str,
}

/// Resolution changes for every module of a project.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ResolutionOverrides {
    /// Ordered aliases (jest `moduleNameMapper`, vitest `test.alias` then
    /// `resolve.alias`).
    pub aliases: Vec<Alias>,
    /// jest `roots` (also where root `__mocks__` are found).
    pub roots: Vec<PathBuf>,
    /// jest `modulePaths`: extra absolute lookup directories (like
    /// `NODE_PATH`).
    pub module_paths: Vec<PathBuf>,
    /// jest `moduleDirectories`: directory names (or absolute directories)
    /// searched upwards, `node_modules` by default.
    pub module_directories: Vec<String>,
    /// jest `modulePathIgnorePatterns` (regexes over absolute paths).
    pub module_path_ignore_patterns: Vec<JsRegex>,
    /// Extensions tried, in order (jest `moduleFileExtensions`).
    pub extensions: Vec<String>,
    /// Loader plugins of the runner (vite): modules with these suffixes are
    /// compiled by a plugin that may read files the graph does not see. A
    /// file that imports such a module is uncertain; the plugin makes
    /// nothing else uncertain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub loaders: Vec<crate::viteplugins::Loader>,
    /// Requests for which a custom resolver (jest `resolver`) only changes the
    /// export conditions of the default resolution. Which file of an
    /// installed package that picks does not matter to the graph (a package
    /// is one unit); an importer whose request resolves into the repository
    /// is uncertain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub condition_requests: Vec<String>,
}

/// A test file pattern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPattern {
    /// A micromatch glob (braces, extglobs, `**`), see [`crate::pattern`].
    Glob(String),
    /// A JS regex over the absolute path.
    Regex(JsRegex),
}

/// Which files are tests.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TestDiscovery {
    /// Directories searched.
    pub roots: Vec<PathBuf>,
    /// A file is a test if it matches any of these...
    pub include: Vec<TestPattern>,
    /// ...and none of these.
    pub exclude: Vec<TestPattern>,
    /// Globs match paths relative to this directory (vitest); `None`: absolute
    /// paths (jest, micromatch over the absolute path).
    pub glob_base: Option<PathBuf>,
    /// Allowed extensions, without dot (empty: any).
    pub extensions: Vec<String>,
    /// Whether the runner takes a file for one of an installed package by its
    /// absolute path: jest's file map leaves out every path through a
    /// `node_modules` directory, also one the checkout itself lies below.
    /// Otherwise the directories above the repository root do not count
    /// (vitest, Playwright, pytest skip `node_modules` directories below
    /// the directory they search).
    #[serde(skip)]
    pub absolute: bool,
    /// Set when this discovery was chosen among two by what the tree holds
    /// ([`DiscoveryChoice`]).
    #[serde(skip)]
    pub choice: Option<Box<DiscoveryChoice>>,
    /// The discovery as its config writes it, without the place of the checkout
    /// ([`crate::location::DiscoverySpec`]), for a runner that matches the
    /// absolute path of a file (jest, Playwright): which files are tests
    /// may then depend on where the repository is checked out, and whoever
    /// keeps the project checks it ([`crate::location::verdict`]). `None`:
    /// the runner matches from its own root.
    #[serde(skip)]
    pub spec: Option<std::sync::Arc<crate::location::DiscoverySpec>>,
}

/// A project's discovery that was chosen by the files that are there: jest's
/// default patterns stand in for a configured discovery that matches no file
/// and may not have been read exactly. Which one applies is a fact about every
/// file below the project's roots; the answer "is this file a test" depends on
/// it only for a file that the default discovery matches and the configured one
/// does not ([`DiscoveryChoice::decides`]). A file the configured discovery
/// matches is a test whatever else is there (while it is there, that discovery
/// matches a file), a file neither matches is none, and a file the
/// default patterns match is itself the file that makes them apply when the
/// configured discovery matches nothing.
#[derive(Clone, Debug)]
pub struct DiscoveryChoice {
    /// The discovery that was not chosen.
    pub other: TestDiscovery,
    /// Whether the chosen one is the default (`other` is the configured one).
    pub fell_back: bool,
    /// Whether the configured discovery can match a file at all (it has roots
    /// and patterns). One that cannot (a config whose object is not known)
    /// never applies, whatever files appear: the choice then decides for no
    /// file.
    pub configured_can_match: bool,
    /// The part of the load's access log ([`crate::fsx::part`]) that holds what
    /// the choice was made from.
    pub part: String,
}

impl DiscoveryChoice {
    /// Whether the choice decides if a file is a test, given what the chosen
    /// discovery and the other one say of the file.
    pub fn decides(&self, chosen: bool, other: bool) -> bool {
        let (configured, default) = if self.fell_back {
            (other, chosen)
        } else {
            (chosen, other)
        };
        self.configured_can_match && default && !configured
    }
}

/// How per-test mocks are found.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MockPolicy {
    /// Directories whose `__mocks__/<request>` mock bare (package) requests.
    pub root_mock_dirs: Vec<PathBuf>,
    /// Manual mocks found under `roots`, by module name (jest's haste mock map:
    /// any `__mocks__/<name>` file in the roots mocks `require('<name>')`,
    /// even without a `jest.mock` call). Empty for vitest, which needs an
    /// explicit `vi.mock`.
    pub haste_mocks: Vec<(String, PathBuf)>,
    /// Whether package mocks apply without a mock call (jest: yes).
    pub implicit_package_mocks: bool,
    /// `automock: true`.
    pub automock: bool,
    /// Extensions tried for mock files.
    pub extensions: Vec<String>,
    /// Why a replaced module may still be loaded by the project's tests,
    /// whatever the test file says ([`crate::mocks`]): a transform that is
    /// not known to hoist mock calls, ES modules in jest, `automock`, tests
    /// that share a module registry. `None`: replacements hold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreliable: Option<String>,
}

/// Where snapshots live.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotPolicy {
    /// `<test dir>/__snapshots__/<test file name>.snap`.
    #[default]
    Default,
    /// A custom resolver (jest `snapshotResolver`, vitest
    /// `resolveSnapshotPath`): paths cannot be computed statically.
    Custom {
        /// The resolver module, if it is one.
        resolver: Option<Target>,
    },
}

/// A value the adapter could not read statically, or another problem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FrameworkUncertainty {
    /// Category (shared with the tracer).
    #[serde(serialize_with = "ser_kind")]
    pub kind: UncertaintyKind,
    /// The file (config or test).
    pub file: PathBuf,
    /// 1-based line (0 when unknown).
    pub line: u32,
    /// The config field, if any.
    pub field: Option<String>,
    /// Source text or explanation.
    pub detail: String,
}

impl FrameworkUncertainty {
    /// Whether the record says that the runner may resolve an import of the
    /// project's code to another module, or run another module in its
    /// place, than the graph has: plugins that are not modelled
    /// (`resolveId`, `load`, `transform`), a custom resolver, an alias with
    /// a custom resolver. Such a record is a *module load with an unknown
    /// target* of every test of the project, not a file read: the module
    /// that is loaded instead comes from somewhere, and the rule for such
    /// loads says from where (the test's package and the packages it
    /// declares; `turborepo_trace_graph::loads`).
    pub fn is_load(&self) -> bool {
        self.kind == UncertaintyKind::ConditionalResolution
            && matches!(
                self.field.as_deref(),
                Some("plugins" | "resolver" | "resolve.alias" | "test.alias")
            )
    }
}

fn ser_kind<S: Serializer>(k: &UncertaintyKind, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(k.as_str())
}

/// One alias application ([`ResolutionOverrides::apply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasHit {
    /// Index into [`ResolutionOverrides::aliases`].
    pub alias: usize,
    /// Rewritten requests, in order.
    pub requests: Vec<String>,
}

impl ResolutionOverrides {
    /// Rewrites `request` with the first matching alias. A regex no engine can
    /// run ([`JsRegex::is_supported`]) never matches; it is reported as an
    /// uncertainty when the project is loaded.
    pub fn apply(&self, request: &str) -> Option<AliasHit> {
        for (i, a) in self.aliases.iter().enumerate() {
            match &a.matcher {
                AliasMatcher::Regex(re) => {
                    let Some(caps) = re.captures(request) else {
                        continue;
                    };
                    let requests = a
                        .replacements
                        .iter()
                        .map(|r| substitute_groups(r, &caps))
                        .collect();
                    return Some(AliasHit { alias: i, requests });
                }
                AliasMatcher::Replace(re) => {
                    if !re.is_match(request) {
                        continue;
                    }
                    let requests = a
                        .replacements
                        .iter()
                        .filter_map(|r| re.replace(request, r))
                        .collect();
                    return Some(AliasHit { alias: i, requests });
                }
                AliasMatcher::Prefix(p) => {
                    let rest = if request == p {
                        Some("")
                    } else {
                        request
                            .strip_prefix(p.as_str())
                            .filter(|r| r.starts_with('/'))
                    };
                    if let Some(rest) = rest {
                        let requests = a
                            .replacements
                            .iter()
                            .map(|r| format!("{r}{rest}"))
                            .collect();
                        return Some(AliasHit { alias: i, requests });
                    }
                }
            }
        }
        None
    }
}

/// jest's mapper substitution: `$N` becomes group N (an unmatched group becomes
/// empty).
fn substitute_groups(repl: &str, caps: &[Option<String>]) -> String {
    let b = repl.as_bytes();
    let mut out = String::with_capacity(repl.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 {
                let n: usize = repl[i + 1..j].parse().unwrap_or(usize::MAX);
                if let Some(Some(g)) = caps.get(n) {
                    out.push_str(g);
                }
                i = j;
                continue;
            }
        }
        let ch = repl[i..].chars().next().unwrap_or('$');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// A compiled [`TestDiscovery`].
pub struct DiscoveryMatcher<'a> {
    d: &'a TestDiscovery,
    include: Vec<CompiledPattern>,
    exclude: Vec<CompiledPattern>,
    /// The repository root ([`DiscoveryMatcher::rooted`]).
    repo: Option<PathBuf>,
}

impl TestDiscovery {
    /// Compiles the patterns.
    pub fn matcher(&self) -> DiscoveryMatcher<'_> {
        DiscoveryMatcher {
            d: self,
            include: self.include.iter().map(CompiledPattern::new).collect(),
            exclude: self.exclude.iter().map(CompiledPattern::new).collect(),
            repo: None,
        }
    }

    /// Lists the tests under the roots (sorted, deduplicated). Skips
    /// `node_modules` and `.git`.
    pub fn enumerate(&self) -> Vec<PathBuf> {
        let m = self.matcher();
        let mut out = Vec::new();
        for r in &self.roots {
            crate::fsx::walk_files(r, &mut |p| {
                if m.is_test(p) {
                    out.push(p.to_path_buf());
                }
            });
        }
        out.sort();
        out.dedup();
        out
    }
}

impl DiscoveryMatcher<'_> {
    /// Names the repository root (absolute, real), for a matcher that is asked
    /// outside [`crate::fsx::with_fs`]: where the part of a path starts that is
    /// the repository's ([`DiscoveryMatcher::candidate`]).
    pub fn rooted(mut self, repo: &Path) -> Self {
        self.repo = Some(repo.to_path_buf());
        self
    }

    /// Where the repository's part of `path` starts.
    fn local_offset(&self, path: &Path) -> usize {
        match &self.repo {
            Some(r) => turborepo_trace_core::fs::shared_prefix(r, path),
            None => crate::fsx::local_offset(path),
        }
    }

    /// `path` (absolute) as text and the path from the glob base on, when the
    /// file can be a test of this project at all: it is below a root, not
    /// in an installed package, and has one of the extensions.
    fn candidate(&self, path: &Path) -> Option<(String, Option<String>)> {
        if !self.d.roots.iter().any(|r| path.starts_with(r)) {
            return None;
        }
        // The path as text, cut where the repository's part starts. The offset is one
        // of the path's bytes: a name that is not UTF-8 is replaced on each
        // side of it.
        use turborepo_trace_core::os::OsStrExt;
        let raw = path.as_os_str().as_bytes();
        let cut = self.local_offset(path).min(raw.len());
        let above = String::from_utf8_lossy(&raw[..cut]);
        let abs = format!("{above}{}", String::from_utf8_lossy(&raw[cut..]));
        // An installed package: for jest by the whole path (its file map leaves out a
        // checkout below a `node_modules` directory), for every other runner by the
        // part of the path that is the repository's.
        let from = if self.d.absolute { 0 } else { above.len() };
        if abs[from..].contains("/node_modules/") {
            return None;
        }
        if !self.d.extensions.is_empty() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !self.d.extensions.iter().any(|e| e == ext) {
                return None;
            }
        }
        let rel = self
            .d
            .glob_base
            .as_ref()
            .and_then(|b| path.strip_prefix(b).ok())
            .map(|r| r.to_string_lossy().into_owned());
        Some((abs, rel))
    }

    /// Whether `path` (absolute) is a test of this project.
    ///
    /// A glob with a base is matched against the path from the base on (vitest,
    /// pytest). Every other pattern is matched against the absolute path,
    /// as jest and Playwright match it: the directories the checkout lies
    /// below are part of what a pattern sees ([`crate::location`]).
    pub fn is_test(&self, path: &Path) -> bool {
        let Some((abs, rel)) = self.candidate(path) else {
            return false;
        };
        let subject = |p: &CompiledPattern| -> &str {
            match (p.is_glob(), &rel) {
                (true, Some(r)) => r.as_str(),
                _ => abs.as_str(),
            }
        };
        self.include.iter().any(|p| p.matches(subject(p)))
            && !self.exclude.iter().any(|p| p.matches(subject(p)))
    }

    /// Whether a pattern that leaves files out matches `path` (absolute), a
    /// file below the project's roots: for jest such a file is no part of
    /// the file map (no test, no mock, no module).
    pub fn left_out(&self, path: &Path) -> bool {
        if !self.d.roots.iter().any(|r| path.starts_with(r)) {
            return false;
        }
        let abs = path.to_string_lossy();
        let rel = self
            .d
            .glob_base
            .as_ref()
            .and_then(|b| path.strip_prefix(b).ok())
            .map(|r| r.to_string_lossy().into_owned());
        self.exclude.iter().any(|p| match (p.is_glob(), &rel) {
            (true, Some(r)) => p.matches(r),
            _ => p.matches(&abs),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapper_groups_and_prefix_aliases() {
        let r = ResolutionOverrides {
            aliases: vec![
                Alias {
                    matcher: AliasMatcher::Regex(JsRegex::new(r"^@app/(.*)/(.*)$", "")),
                    replacements: vec!["/r/src/$1/lib/$2".into(), "/r/fallback/$2$9".into()],
                    source: "moduleNameMapper",
                },
                Alias {
                    matcher: AliasMatcher::Prefix("@".into()),
                    replacements: vec!["/r/src".into()],
                    source: "resolve.alias",
                },
            ],
            ..ResolutionOverrides::default()
        };
        let h = r.apply("@app/ui/button").unwrap();
        assert_eq!(h.alias, 0);
        assert_eq!(h.requests, ["/r/src/ui/lib/button", "/r/fallback/button"]);
        assert_eq!(r.apply("@/x/y").unwrap().requests, ["/r/src/x/y"]);
        assert_eq!(r.apply("@").unwrap().requests, ["/r/src"]);
        assert!(r.apply("@scope/pkg").is_none());
        assert!(r.apply("lodash").is_none());
    }

    /// The offset of the repository's part is one of the path's bytes: a
    /// directory name that is not UTF-8, above the root or inside, is
    /// replaced on each side of it (the lossy text is longer than the path,
    /// and the offset fell into one of its characters).
    #[test]
    fn a_name_that_is_not_utf8_is_matched_like_any_other() {
        use std::ffi::OsStr;

        use turborepo_trace_core::os::OsStrExt;
        let repo = PathBuf::from(OsStr::from_bytes(
            b"/ci/\xff\xff\xff\xff\xff\xff\xff\xff/repo",
        ));
        let d = TestDiscovery {
            roots: vec![repo.clone()],
            include: vec![
                TestPattern::Glob("**/*.test.js".into()),
                TestPattern::Regex(JsRegex::new("/checks/.*[.]js$", "")),
            ],
            exclude: vec![TestPattern::Regex(JsRegex::new("/node_modules/", ""))],
            ..TestDiscovery::default()
        };
        let m = d.matcher().rooted(&repo);
        assert!(m.is_test(&repo.join("src/a.test.js")));
        assert!(m.is_test(&repo.join("checks/a.js")));
        assert!(!m.is_test(&repo.join("src/a.js")));
        assert!(!m.is_test(&repo.join("node_modules/x/a.test.js")));
        let inside = repo.join(OsStr::from_bytes(b"s\xffrc/a.test.js"));
        assert!(m.is_test(&inside));
    }

    /// jest matches absolute paths: a pattern sees the directories the checkout
    /// lies below, on the side that makes a file a test and on the side
    /// that leaves it out.
    #[test]
    fn patterns_are_matched_against_the_absolute_path() {
        let at = |repo: &str| {
            let repo = Path::new(repo);
            let d = TestDiscovery {
                roots: vec![repo.to_path_buf()],
                include: vec![
                    TestPattern::Glob("**/repo/a/checks/**/*.js".into()),
                    TestPattern::Regex(JsRegex::new("repo/b/checks/.*[.]js$", "")),
                ],
                exclude: vec![
                    TestPattern::Regex(JsRegex::new("/node_modules/", "")),
                    TestPattern::Regex(JsRegex::new("cmp", "")),
                ],
                absolute: true,
                ..TestDiscovery::default()
            };
            let m = d.matcher().rooted(repo);
            ["a/checks/one.js", "b/checks/one.js", "c/checks/one.js"]
                .map(|rel| m.is_test(&repo.join(rel)))
        };
        assert_eq!(at("/work/repo"), [true, true, false]);
        // Elsewhere the patterns name nothing.
        assert_eq!(at("/work/other"), [false, false, false]);
        // `cmp` above the root leaves every file out, as it does in jest.
        assert_eq!(at("/vercel/cmp/repo"), [false, false, false]);
        // So does a checkout below `node_modules`, for jest.
        assert_eq!(at("/ci/node_modules/repo"), [false, false, false]);
    }

    /// A runner that searches below its own root (vitest, Playwright) finds the
    /// tests of a checkout below a `node_modules` directory.
    #[test]
    fn a_checkout_below_node_modules_has_tests_for_a_runner_that_matches_from_its_root() {
        let repo = Path::new("/ci/node_modules/x");
        let d = TestDiscovery {
            roots: vec![repo.to_path_buf()],
            include: vec![TestPattern::Glob("**/*.test.js".into())],
            glob_base: Some(repo.to_path_buf()),
            ..TestDiscovery::default()
        };
        let m = d.matcher().rooted(repo);
        assert!(m.is_test(&repo.join("src/a.test.js")));
        assert!(!m.is_test(&repo.join("node_modules/p/a.test.js")));
    }

    /// A vite regex alias replaces the matched part of the request
    /// (`String.replace`), and a negative lookahead excludes requests
    /// exactly.
    #[test]
    fn vite_regex_aliases_replace_the_match() {
        let alias = |re: &str, to: &str| Alias {
            matcher: AliasMatcher::Replace(JsRegex::new(re, "")),
            replacements: vec![to.into()],
            source: "resolve.alias",
        };
        let r = ResolutionOverrides {
            aliases: vec![
                alias(r"^#/(?!(?:app/routes\.mjs|lib/valid\.mjs)$)(.*)$", "/r/$1"),
                alias(r"^next/font/(?:local|google)$", "/r/test/font-stub.ts"),
                alias(r"^~", "/r/src"),
            ],
            ..ResolutionOverrides::default()
        };
        assert_eq!(r.apply("#/lib/util").unwrap().requests, ["/r/lib/util"]);
        // Excluded by the lookahead: not aliased at all.
        assert!(r.apply("#/app/routes.mjs").is_none());
        assert_eq!(
            r.apply("next/font/google").unwrap().requests,
            ["/r/test/font-stub.ts"]
        );
        assert!(r.apply("next/font/other").is_none());
        // An unanchored match keeps the rest of the request.
        assert_eq!(r.apply("~/a/b").unwrap().requests, ["/r/src/a/b"]);
    }
}
