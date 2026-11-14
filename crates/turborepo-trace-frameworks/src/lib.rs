//! Test-framework adapters for `turbo trace`.
//!
//! A test runner adds dependencies that no import expresses: its config file
//! and everything the config imports, setup files, a test environment,
//! transformers, module mappers that change resolution, manual mocks, and
//! snapshot files. An adapter ([`Framework`]) finds and statically evaluates
//! the runner's config and reports these in a neutral shape
//! ([`FrameworkInputs`] per config, [`TestInputs`] per test file), so the graph
//! store and `affected` can treat them like any other edge, probe or
//! membership.
//!
//! Adapters are compiled in, like languages ([`frameworks`]): [`jest::Jest`]
//! and [`vitest::Vitest`].
//!
//! # Shape
//!
//! | Piece | Where | Meaning |
//! |---|---|---|
//! | config files + helpers | [`GlobalInputs::config_files`], [`GlobalInputs::js_entries`] | inputs of every test; code configs are traced as JS entries |
//! | setup / env / transform modules | [`GlobalInputs::entries`] | extra entries of every test |
//! | `__mocks__` dirs in roots | [`GlobalInputs::memberships`], [`MockPolicy`] | an added manual mock can replace a package |
//! | module mappers, aliases, roots | [`ResolutionOverrides`] | ordered aliases for the resolver |
//! | test discovery | [`TestDiscovery`] | `--tests jest` enumeration |
//! | snapshot, mocks, per-file env | [`TestInputs`] | per-test edges, probes, memberships, entries |
//! | unknown config parts | [`FrameworkUncertainty`] | reported; the config stays an input |
//!
//! # Other languages
//!
//! - **pytest** (`pytest::Pytest`, cargo feature `python`): `pytest.ini`,
//!   `pyproject.toml`, `tox.ini`, `setup.cfg`; every `conftest.py` from
//!   `confcutdir` down to the test ([`InputRole::Conftest`], probes for missing
//!   ones), plugins ([`InputRole::Plugin`]), package `__init__.py` files
//!   ([`InputRole::ImportedByTest`]) and the test's own import roots
//!   ([`TestInputs::import_roots`]).
//! - **go test** ([`gotest::GoTest`]): the package directory
//!   ([`InputRole::CompiledWithTest`]), `testdata/**` ([`InputRole::TestData`])
//!   and the test files' imports ([`InputRole::ImportedByTest`]).
//! - **cargo test** ([`cargotest::CargoTest`]): `Cargo.toml`, the build script
//!   ([`InputRole::BuildScript`]) and its `rerun-if-changed` inputs, linked
//!   crates ([`InputRole::LinkedCrate`]).
//!
//! [`GlobalInputs::package`] and [`TestInputs::package`] name the package (Go
//! module, crate, Python project) in its ecosystem's terms, so the graph store
//! can attach framework inputs to the right shard.
//!
//! # File access
//!
//! Every adapter reads the filesystem through [`fsx`]. Inside [`fsx::with_fs`]
//! that is the tracer's shared dir cache, so the probes of config discovery
//! (`jest.config.*` next to and above tests, `conftest.py` candidates, helper
//! extension probing) and config reads are recorded like resolver probes.

// Every public item is documented.
#![warn(missing_docs)]

#[cfg(feature = "rust")]
pub mod cargotest;
pub mod common;
pub mod fsx;
#[cfg(feature = "go")]
pub mod gotest;
pub mod iniconf;
pub mod inputs;
pub mod jest;
pub mod jsconfig;
pub mod location;
pub mod mocks;
pub mod nextjs;
pub mod pattern;
pub mod playwright;
#[cfg(feature = "python")]
pub mod pytest;
pub mod scan;
pub mod viteplugins;
pub mod vitest;

use std::path::{Path, PathBuf};

pub use common::{ModuleResolver, SimpleResolver};
pub use inputs::{
    Alias, AliasHit, AliasMatcher, ConfigValues, DiscoveryChoice, DiscoveryMatcher, EdgeKind,
    FrameworkInputs, FrameworkUncertainty, GlobalInputs, ImplicitEdge, ImportRoots, InputRef,
    InputRole, LazyMock, MembershipSpec, MockHazard, MockPolicy, MockSite, PackageRef, Project,
    Replaced, ResolutionOverrides, SnapshotPolicy, Target, TestDiscovery, TestInputs, TestPattern,
};
pub use pattern::JsRegex;

/// A test framework adapter (compiled in, like a language).
///
/// The trait is language-neutral: nothing in it is specific to JS. Adapters do
/// their own static evaluation (the JS ones use the lang-js config evaluator)
/// and report [`FrameworkInputs`].
pub trait Framework: Send + Sync {
    /// Stable id (`"jest"`).
    fn id(&self) -> &'static str;

    /// File names that hold a config (for repo walks and change detection).
    fn config_file_names(&self) -> &'static [&'static str];

    /// Whether `path` is a config file of this framework (a `package.json` only
    /// if it holds the framework's key).
    fn is_config_file(&self, path: &Path) -> bool;

    /// Every config file under `root` (skipping `node_modules`, `.git`).
    fn discover(&self, root: &Path) -> Vec<PathBuf>;

    /// The config that governs `test` (nearest ancestor with a config), if any.
    fn config_for(&self, test: &Path) -> Option<PathBuf>;

    /// Loads and statically evaluates a config file.
    fn load(&self, config: &Path) -> FrameworkInputs;

    /// Per-test inputs for `test` (with source `src`) in `project`. `resolver`
    /// resolves mocked module requests (aliases are applied first).
    fn test_inputs(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        resolver: &dyn ModuleResolver,
    ) -> TestInputs;

    /// [`Framework::test_inputs`] when the caller already has what the file
    /// does with the mock APIs of the runners (the fact of its extraction,
    /// `turborepo_trace_lang_js::mocks::MockScan`): the adapter then does not
    /// scan the source for it again. `None`: as [`Framework::test_inputs`].
    fn test_inputs_scanned(
        &self,
        project: &Project,
        test: &Path,
        src: &[u8],
        resolver: &dyn ModuleResolver,
        _mocks: Option<&mocks::MockScan>,
    ) -> TestInputs {
        self.test_inputs(project, test, src, resolver)
    }

    /// The directories the runner may be in while it runs `test` of `project`:
    /// what a relative path and the working directory mean in the test's
    /// code and its helpers. Default: the directory of the project's config
    /// file and the project root ([`Project::test_cwds`]): jest, vitest and
    /// pytest are started there, `cargo test` runs a package's tests in the
    /// package directory. A runner that changes directory
    /// per test overrides it (`go test`: the package directory of the test
    /// file).
    fn test_cwds(&self, project: &Project, test: &Path) -> Vec<PathBuf> {
        let _ = test;
        project.test_cwds()
    }
}

/// The compiled-in adapters.
pub fn frameworks() -> &'static [&'static dyn Framework] {
    &[
        &jest::Jest,
        &vitest::Vitest,
        #[cfg(feature = "rust")]
        &cargotest::CargoTest,
        #[cfg(feature = "go")]
        &gotest::GoTest,
        #[cfg(feature = "python")]
        &pytest::Pytest,
        &playwright::Playwright,
    ]
}

/// Whether the pytest adapter is compiled in (cargo feature `python`).
pub const PYTHON: bool = cfg!(feature = "python");

/// pytest's config file for a test at `probe` (`None` without the `python`
/// feature).
pub fn pytest_config_for(probe: &Path) -> Option<PathBuf> {
    #[cfg(feature = "python")]
    {
        pytest::Pytest.config_for(probe)
    }
    #[cfg(not(feature = "python"))]
    {
        let _ = probe;
        None
    }
}

/// What pytest reads of the config file `config` (`None` without the `python`
/// feature: unknown).
pub fn pytest_inputs(config: &Path) -> Option<FrameworkInputs> {
    #[cfg(feature = "python")]
    {
        Some(pytest::Pytest.load(config))
    }
    #[cfg(not(feature = "python"))]
    {
        let _ = config;
        None
    }
}

/// The adapter with id `id`.
pub fn framework(id: &str) -> Option<&'static dyn Framework> {
    frameworks().iter().copied().find(|f| f.id() == id)
}

impl FrameworkInputs {
    /// The project whose discovery includes `test`, else the first project
    /// whose root contains it.
    pub fn project_for(&self, test: &Path) -> Option<&Project> {
        self.projects
            .iter()
            .find(|p| p.discovery.matcher().is_test(test))
            .or_else(|| self.projects.iter().find(|p| test.starts_with(&p.root_dir)))
    }

    /// Whether any project or the config itself has uncertainties.
    pub fn is_partial(&self) -> bool {
        !self.uncertainties.is_empty() || self.projects.iter().any(|p| !p.uncertainties.is_empty())
    }
}
