//! Toolchains: the abstraction that makes Turborepo generic over language
//! ecosystems.
//!
//! A [`Toolchain`] answers ecosystem-specific questions about packages —
//! starting with "which packages exist?" — so that the package graph and the
//! rest of the system never branch on a specific ecosystem. JavaScript is the
//! first implementation ([`JavaScriptToolchain`]); additional toolchains
//! (e.g. Cargo) register alongside it in the [`ToolchainRegistry`].
//!
//! The trait grows one concern at a time (discovery today; command
//! resolution, derived task inputs/outputs, external-dependency hashing,
//! watch triggers, and prune participation as they are needed), and every
//! concern must ship with real implementations for every registered
//! toolchain.
//!
//! # Design rules
//!
//! These rules keep the door open to an out-of-process plugin architecture
//! (subprocess or WASM adapters implementing this same trait) without
//! committing to one today:
//!
//! 1. Trait methods are coarse-grained and data-in/data-out: arguments and
//!    return values are serializable-shaped (paths, strings, plain structs). No
//!    internal graph types, no lifetime-carrying views, no callbacks.
//! 2. [`ToolchainId`] is an open identifier, never a closed enum. A future
//!    toolchain (or plugin) mints a new id without touching existing code.
//! 3. All toolchain lookups go through the [`ToolchainRegistry`]. Scattered
//!    per-toolchain branch points (`if id == "cargo"`) are a design defect.
//!
//! # Known debt
//!
//! Some JavaScript machinery that predates this abstraction is still called
//! directly, outside the trait, because the build phases that need it have
//! no trait surface yet. The list below is a checklist to burn down: as the
//! trait gains a surface for each concern, the corresponding direct access
//! goes away. When the list is empty, JavaScript is fully behind the
//! abstraction.
//!
//! - [`JavaScriptToolchain::package_manager`]: package-manager resolution feeds
//!   dependency splitting and lockfile handling in the package graph builder.
//!   Lockfile handling gains a trait surface with external dependency hashing;
//!   dependency splitting remains JS-native for now.
//! - The prune command's JavaScript machinery (lockfile subgraphing,
//!   workspace-file rewriting, patches) is its native path rather than a
//!   [`Toolchain::prune_plan`] implementation.

use std::{borrow::Cow, ffi::OsString, fmt, future::Future, pin::Pin, sync::Arc};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::{
    discovery::{self, PackageDiscovery},
    package_json::PackageJson,
    package_manager::PackageManager,
};

/// Identifies a toolchain: the language ecosystem a package belongs to.
///
/// Open by design (see the module's design rules): any string can be a
/// toolchain id, so new toolchains — including, potentially, ones loaded as
/// plugins — do not require changes to this type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToolchainId(Cow<'static, str>);

impl ToolchainId {
    /// The JavaScript toolchain: packages discovered from `package.json`
    /// manifests, regardless of package manager or runtime.
    pub const JAVASCRIPT: ToolchainId = ToolchainId(Cow::Borrowed("javascript"));

    /// The Cargo toolchain: Rust crates discovered from a Cargo workspace
    /// (see [`crate::cargo`]). Experimental, gated behind
    /// `futureFlags.experimentalCargoWorkspaces`.
    pub const CARGO: ToolchainId = ToolchainId(Cow::Borrowed("cargo"));

    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ToolchainId {
    fn default() -> Self {
        Self::JAVASCRIPT
    }
}

impl fmt::Display for ToolchainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A package discovered by a toolchain.
///
/// `descriptor` is the toolchain-neutral package descriptor. [`PackageJson`]
/// serves as that descriptor for every toolchain: JavaScript packages parse
/// theirs from disk, while other toolchains synthesize one from their native
/// manifest (only the fields they populate — at minimum `name` and internal
/// dependencies — are meaningful).
#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    /// The toolchain-neutral package descriptor.
    pub descriptor: PackageJson,
    /// Absolute path to the package's native manifest (`package.json`,
    /// `Cargo.toml`, ...).
    pub manifest_path: AbsoluteSystemPathBuf,
    /// External-dependency identities for this package, when the toolchain
    /// resolves them at discovery time. They feed the package's
    /// external-dependency hash: a task's hash changes exactly when an
    /// identity in its package's set changes. Cargo computes these from
    /// Cargo.lock (per-crate transitive closures) plus a compiler-version
    /// stamp.
    ///
    /// `None` defers to the toolchain's own pipeline. Known debt (see
    /// module docs): JavaScript's closures are computed by the graph
    /// builder's lockfile phase — deliberately concurrent with run setup —
    /// rather than through this field; folding that in requires a
    /// deferred-aware trait surface.
    pub external_dependencies: Option<std::collections::HashSet<turborepo_lockfiles::Package>>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Discovery(#[from] discovery::Error),
    #[error(transparent)]
    Descriptor(#[from] crate::package_json::Error),
    /// A toolchain-specific failure. Boxed rather than enumerated so the
    /// generic error surface does not accumulate a variant per toolchain.
    #[error(transparent)]
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

/// The future returned by [`Toolchain::discover_packages`]. Boxed so the
/// trait stays object-safe; toolchains live behind `dyn Toolchain` in the
/// [`ToolchainRegistry`].
pub type DiscoverPackagesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<DiscoveredPackage>, Error>> + Send + 'a>>;

/// A command resolved by a toolchain for a task, as plain data. The executor
/// turns it into a process, applying the task's environment, stdin policy,
/// and any decorations that are not toolchain concerns (e.g.
/// microfrontends proxy variables).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCommand {
    /// The program to execute. `OsString` rather than `String` only to
    /// tolerate non-UTF-8 binary paths; the value is still plain data.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Absolute directory to run in.
    pub cwd: AbsoluteSystemPathBuf,
    /// Mutually-exclusive execution group: the executor runs at most one
    /// command per group at a time, process-wide. For tools that hold
    /// global locks (e.g. Cargo's build directory), where concurrent
    /// processes cannot make progress anyway.
    pub serial_group: Option<String>,
}

/// A language ecosystem that contributes packages to the repository.
///
/// See the module docs for the design rules trait methods must follow.
pub trait Toolchain: Send + Sync {
    /// This toolchain's identifier.
    fn id(&self) -> ToolchainId;

    /// Discover this toolchain's packages.
    fn discover_packages(&self) -> DiscoverPackagesFuture<'_>;

    /// Resolve the command that implements `task` for `package`, or `None`
    /// when the toolchain defines no command for it — the task is then a
    /// no-op, like a missing package.json script.
    ///
    /// `pass_through_args` are user-supplied arguments (`turbo run task --
    /// <args>`); the toolchain owns how they are attached, since separator
    /// conventions differ per tool.
    fn task_command(
        &self,
        repo_root: &AbsoluteSystemPath,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        pass_through_args: Option<&[String]>,
    ) -> Result<Option<TaskCommand>, Error> {
        let _ = (repo_root, package, task, pass_through_args);
        Ok(None)
    }

    /// A one-line description of what `task` runs for `package`, for
    /// dry-run output and run summaries. Derived from the same tables as
    /// [`Toolchain::task_command`] so display cannot drift from execution.
    fn task_display_command(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
    ) -> Option<String> {
        let _ = (package, task);
        None
    }

    /// Whether this toolchain defines an executable command for `task` in
    /// `package`. Tasks without one are phantom/transit tasks (they exist
    /// solely for dependency ordering via `dependsOn: ["^task"]`): they do
    /// not execute, so hashing concerns like global input files must not
    /// apply to them.
    fn defines_task(&self, package: &crate::package_graph::PackageInfo, task: &str) -> bool {
        let _ = (package, task);
        false
    }

    /// Hash wiring this toolchain derives for `task` in `package`, beyond
    /// what turbo.json declares: extra input globs and env vars that
    /// participate in the task hash, output globs to cache, and whether the
    /// package's own default (git-index based) file hashing applies.
    ///
    /// `path_to_root` is the unix path from the package directory to the
    /// repo root (empty for a package at the root); returned globs are
    /// relative to the package directory. `dependencies` are the package's
    /// transitive internal dependencies. `wants_automatic_inputs` reflects
    /// the task's `inputs` configuration: `true` unless explicit inputs
    /// omitted `$TURBO_DEFAULT$` — for toolchains that derive inputs,
    /// `$TURBO_DEFAULT$` means "everything the toolchain derives", so users
    /// can append inputs without forfeiting automatic invalidation.
    ///
    /// `None` means the toolchain derives nothing and turbo.json is the
    /// whole story.
    fn derived_task_io(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        path_to_root: &str,
        dependencies: &[&crate::package_graph::PackageInfo],
        wants_automatic_inputs: bool,
    ) -> Option<DerivedTaskIO> {
        let _ = (
            package,
            task,
            path_to_root,
            dependencies,
            wants_automatic_inputs,
        );
        None
    }

    /// Whether [`Toolchain::derived_task_io`] can return `Some` for this
    /// package/task. Callers use this to skip assembling the (expensive)
    /// dependency-closure argument when the answer is knowably `None` —
    /// notably engine construction, which resolves a definition per task.
    fn derives_task_io(&self, package: &crate::package_graph::PackageInfo, task: &str) -> bool {
        let _ = (package, task);
        false
    }

    /// How filesystem events relate to this toolchain in watch mode:
    /// workspace-definition files whose change requires rediscovery, and
    /// build-byproduct directories whose events must be ignored.
    fn watch_spec(&self) -> WatchSpec {
        WatchSpec::default()
    }

    /// What `turbo prune` must carry for this toolchain so the pruned
    /// repository is self-contained, given the names of this toolchain's
    /// packages already selected for the pruned output. `None` means the
    /// toolchain contributes nothing beyond the packages themselves.
    fn prune_plan(&self, kept_packages: &[String]) -> Result<Option<PrunePlan>, Error> {
        let _ = kept_packages;
        Ok(None)
    }

    /// Called after the pruned output is fully written, with its root
    /// directory. Toolchains may polish their own files in place (e.g.
    /// Cargo canonicalizes the pruned lockfile through `cargo metadata`).
    /// Failures must be non-fatal: log and continue.
    fn prune_finalize(&self, pruned_root: &AbsoluteSystemPath) {
        let _ = pruned_root;
    }
}

/// A toolchain's contribution to a pruned repository. See
/// [`Toolchain::prune_plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// Packages that must additionally be kept and copied, beyond the ones
    /// requested (e.g. crates reachable only through dev-dependency edges,
    /// whose manifests are referenced by kept crates).
    pub extra_packages: Vec<String>,
    /// Files to write into the pruned repository: (repo-relative unix path,
    /// contents). They define dependency resolution, so they go to the full
    /// layer and, in docker mode, the json layer.
    pub root_files: Vec<(String, String)>,
    /// Repo-relative unix paths of toolchain configuration files to copy
    /// verbatim when present (missing ones are skipped).
    pub copy_paths: Vec<String>,
}

/// How filesystem events relate to a toolchain in watch mode. See
/// [`Toolchain::watch_spec`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchSpec {
    /// Manifest file names that define the toolchain's workspace membership
    /// or edges wherever they appear in the repository (outside
    /// [`WatchSpec::ignore_prefixes`]): a change means the package set may
    /// have changed, requiring full rediscovery.
    pub definition_file_names: Vec<String>,
    /// Repo-root-relative unix paths that define the workspace, with the
    /// same rediscovery consequence.
    pub definition_paths: Vec<String>,
    /// Repo-root-relative unix directory prefixes containing the
    /// toolchain's own build byproducts. Events under them are dropped:
    /// they are written by the very tasks a change would re-trigger, and
    /// must not feed back into the watcher even when not gitignored.
    pub ignore_prefixes: Vec<String>,
}

impl WatchSpec {
    /// Merge another spec into this one.
    pub fn extend(&mut self, other: WatchSpec) {
        self.definition_file_names
            .extend(other.definition_file_names);
        self.definition_paths.extend(other.definition_paths);
        self.ignore_prefixes.extend(other.ignore_prefixes);
    }
}

/// Hash wiring derived by a toolchain for one task. See
/// [`Toolchain::derived_task_io`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DerivedTaskIO {
    /// Extra input globs, relative to the package directory.
    pub input_globs: Vec<String>,
    /// When `Some`, overrides whether the package's own default (git-index
    /// based) file hashing applies to this task.
    pub package_default_inputs: Option<bool>,
    /// Env vars that participate in the task hash.
    pub env: Vec<String>,
    /// Output globs to cache, relative to the package directory.
    pub output_globs: Vec<String>,
}

/// The set of toolchains contributing packages to the repository.
///
/// All toolchain lookups go through the registry; it is the single place
/// that knows which toolchains exist. Today entries are registered
/// statically during package graph construction. A future plugin system
/// would construct entries from a manifest instead — an additive change.
#[derive(Default)]
pub struct ToolchainRegistry {
    toolchains: Vec<Arc<dyn Toolchain>>,
}

impl ToolchainRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a toolchain. Registration order is discovery order.
    pub fn register(&mut self, toolchain: Arc<dyn Toolchain>) {
        debug_assert!(
            self.get(&toolchain.id()).is_none(),
            "toolchain {} registered twice",
            toolchain.id()
        );
        self.toolchains.push(toolchain);
    }

    pub fn get(&self, id: &ToolchainId) -> Option<&dyn Toolchain> {
        self.toolchains
            .iter()
            .find(|toolchain| toolchain.id() == *id)
            .map(AsRef::as_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Toolchain> {
        self.toolchains.iter().map(AsRef::as_ref)
    }

    /// The union of every registered toolchain's [`WatchSpec`].
    pub fn watch_spec(&self) -> WatchSpec {
        let mut merged = WatchSpec::default();
        for toolchain in self.iter() {
            merged.extend(toolchain.watch_spec());
        }
        merged
    }
}

impl fmt::Debug for ToolchainRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.toolchains.iter().map(|toolchain| toolchain.id()))
            .finish()
    }
}

/// The JavaScript toolchain: packages discovered from `package.json`
/// manifests.
///
/// Wraps a [`PackageDiscovery`] strategy (local filesystem walk,
/// daemon-backed, or a composition) — the strategy decides *how* manifests
/// are found, the toolchain owns *what a JavaScript package is*: it loads
/// and parses each manifest into the package descriptor.
pub struct JavaScriptToolchain<P> {
    discovery: P,
    /// The package manager as resolved during graph construction, recorded
    /// by the builder so synchronous concerns (command resolution) can use
    /// it without re-running discovery.
    resolved_package_manager: std::sync::OnceLock<PackageManager>,
    /// The package manager binary, resolved lazily on first command.
    package_manager_binary: std::sync::OnceLock<Result<std::path::PathBuf, which::Error>>,
}

#[derive(Debug, thiserror::Error)]
enum JavaScriptCommandError {
    // Message kept identical to the pre-toolchain provider error for
    // output compatibility.
    #[error("Unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
}

impl<P: PackageDiscovery + Send + Sync> JavaScriptToolchain<P> {
    pub fn new(discovery: P) -> Self {
        Self {
            discovery,
            resolved_package_manager: std::sync::OnceLock::new(),
            package_manager_binary: std::sync::OnceLock::new(),
        }
    }

    /// The repository's JavaScript package manager.
    ///
    /// Known debt (see module docs): dependency splitting and lockfile
    /// handling in the package graph builder are not yet trait concerns, so
    /// they reach into the JavaScript toolchain directly for this.
    pub async fn package_manager(&self) -> Result<PackageManager, discovery::Error> {
        Ok(self.discovery.discover_packages().await?.package_manager)
    }

    /// Record the package manager resolved during graph construction. Called
    /// by the package graph builder; later calls are no-ops.
    pub fn set_resolved_package_manager(&self, package_manager: PackageManager) {
        let _ = self.resolved_package_manager.set(package_manager);
    }
}

#[cfg(windows)]
// Avoid npm.cmd so Windows Ctrl+C reaches npm/node without cmd.exe emitting
// "Terminate batch job (Y/N)?" during graceful shutdown.
fn npm_direct_command(
    package_manager_binary: &std::path::Path,
) -> Option<(std::path::PathBuf, OsString)> {
    if package_manager_binary.file_name()?.to_str()? != "npm.cmd" {
        return None;
    }

    let node_dir = package_manager_binary.parent()?;
    let node = node_dir.join("node.exe");
    let npm_cli = node_dir
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");

    (node.is_file() && npm_cli.is_file()).then(|| (node, npm_cli.into_os_string()))
}

#[cfg(windows)]
fn package_manager_command(
    package_manager: &PackageManager,
    package_manager_binary: &std::path::Path,
) -> (OsString, Vec<OsString>) {
    if package_manager == &PackageManager::Npm
        && let Some((node, npm_cli)) = npm_direct_command(package_manager_binary)
    {
        return (node.into_os_string(), vec![npm_cli]);
    }

    (package_manager_binary.as_os_str().to_owned(), Vec::new())
}

#[cfg(not(windows))]
fn package_manager_command(
    _package_manager: &PackageManager,
    package_manager_binary: &std::path::Path,
) -> (OsString, Vec<OsString>) {
    (package_manager_binary.as_os_str().to_owned(), Vec::new())
}

impl<P: PackageDiscovery + Send + Sync> Toolchain for JavaScriptToolchain<P> {
    fn id(&self) -> ToolchainId {
        ToolchainId::JAVASCRIPT
    }

    fn task_command(
        &self,
        repo_root: &AbsoluteSystemPath,
        package: &crate::package_graph::PackageInfo,
        task: &str,
        pass_through_args: Option<&[String]>,
    ) -> Result<Option<TaskCommand>, Error> {
        // No script (or an empty one) means the task is a no-op.
        if package
            .package_json
            .scripts
            .get(task)
            .is_none_or(|script| script.is_empty())
        {
            return Ok(None);
        }
        let Some(package_manager) = self.resolved_package_manager.get() else {
            // The graph was not built through this toolchain instance;
            // without a package manager there is no way to run a script.
            return Ok(None);
        };

        let package_manager_binary = self
            .package_manager_binary
            .get_or_init(|| which::which(package_manager.command()))
            .as_deref()
            .map_err(|err| Error::Failed(Box::new(JavaScriptCommandError::Which(*err))))?;
        let (program, mut args) = package_manager_command(package_manager, package_manager_binary);
        args.extend([OsString::from("run"), OsString::from(task)]);
        if let Some(pass_through_args) = pass_through_args {
            args.extend(
                package_manager
                    .arg_separator(pass_through_args)
                    .map(OsString::from),
            );
            args.extend(pass_through_args.iter().map(OsString::from));
        }

        Ok(Some(TaskCommand {
            program,
            args,
            cwd: repo_root.resolve(package.package_path()),
            serial_group: None,
        }))
    }

    fn task_display_command(
        &self,
        package: &crate::package_graph::PackageInfo,
        task: &str,
    ) -> Option<String> {
        // Summaries show the script text itself, matching historical
        // behavior.
        package
            .package_json
            .scripts
            .get(task)
            .map(|script| script.as_inner().clone())
    }

    fn defines_task(&self, package: &crate::package_graph::PackageInfo, task: &str) -> bool {
        package
            .package_json
            .scripts
            .get(task)
            .is_some_and(|script| !script.is_empty())
    }

    fn derived_task_io(
        &self,
        _package: &crate::package_graph::PackageInfo,
        _task: &str,
        _path_to_root: &str,
        _dependencies: &[&crate::package_graph::PackageInfo],
        _wants_automatic_inputs: bool,
    ) -> Option<DerivedTaskIO> {
        // Deliberately nothing: for JavaScript, turbo.json is the whole
        // story — inputs default to the package's files, outputs are
        // whatever the user declares, and no tool-level files or env vars
        // are implied. This is the real answer, not an unimplemented stub.
        None
    }

    fn watch_spec(&self) -> WatchSpec {
        // Deliberately nothing: JavaScript workspace redefinition (a new or
        // removed package.json, a lockfile change) is caught by the change
        // mapper's conservative fallback — unattributable files map to
        // "all packages", which triggers rediscovery — and JS build outputs
        // land inside package directories where gitignore filtering already
        // applies. This is the real answer, not an unimplemented stub.
        WatchSpec::default()
    }

    fn prune_plan(&self, _kept_packages: &[String]) -> Result<Option<PrunePlan>, Error> {
        // Known debt (see module docs): the prune command's JavaScript
        // machinery — lockfile subgraphing, root package.json and
        // pnpm-workspace rewriting, patch carrying — is its native code
        // path, predating this abstraction. Folding it into this surface
        // means restructuring a battle-tested command; until then, the JS
        // contribution is deliberately empty here.
        Ok(None)
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            use tracing::Instrument;
            let workspaces = self
                .discovery
                .discover_packages()
                .instrument(tracing::info_span!("workspace_discovery"))
                .await?
                .workspaces;
            // Parse manifests in parallel; manifest parsing dominates
            // discovery time on large repositories.
            let _span = tracing::info_span!("manifest_parse").entered();
            turborepo_rayon_compat::block_in_place(|| {
                use rayon::prelude::*;
                workspaces
                    .into_par_iter()
                    .map(|workspace| {
                        let descriptor = PackageJson::load(&workspace.package_json)?;
                        Ok(DiscoveredPackage {
                            descriptor,
                            manifest_path: workspace.package_json,
                            // JavaScript closures come from the builder's
                            // (deliberately concurrent) lockfile phase; see
                            // the field docs.
                            external_dependencies: None,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toolchain_id_is_open() {
        let js = ToolchainId::default();
        assert_eq!(js, ToolchainId::JAVASCRIPT);
        assert_eq!(js.as_str(), "javascript");

        // Any string is a valid id; no closed set to extend.
        let custom = ToolchainId::new("cargo");
        assert_ne!(custom, js);
        assert_eq!(custom.to_string(), "cargo");
        let dynamic = ToolchainId::new(String::from("python-uv"));
        assert_eq!(dynamic.as_str(), "python-uv");
    }

    #[test]
    fn test_javascript_task_command() {
        struct StubDiscovery;
        impl PackageDiscovery for StubDiscovery {
            async fn discover_packages(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                Ok(discovery::DiscoveryResponse {
                    package_manager: PackageManager::Npm,
                    workspaces: vec![],
                })
            }
            async fn discover_packages_blocking(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                self.discover_packages().await
            }
        }

        let repo_root_buf =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let repo_root = repo_root_buf.as_ref() as &AbsoluteSystemPath;

        let toolchain = JavaScriptToolchain::new(StubDiscovery);
        toolchain.set_resolved_package_manager(PackageManager::Npm);

        let package = crate::package_graph::PackageInfo {
            package_json: PackageJson::from_value(serde_json::json!({
                "name": "web",
                "scripts": { "build": "next build", "empty": "" }
            }))
            .unwrap(),
            package_json_path: turbopath::AnchoredSystemPathBuf::from_raw(
                ["apps", "web", "package.json"].join(std::path::MAIN_SEPARATOR_STR),
            )
            .unwrap(),
            ..Default::default()
        };

        let command = toolchain
            .task_command(repo_root, &package, "build", None)
            .unwrap()
            .expect("script exists, command resolves");
        // The program is the resolved npm binary (or node.exe on Windows);
        // the invocation shape is what matters.
        assert!(
            command
                .args
                .ends_with(&[OsString::from("run"), OsString::from("build")]),
            "expected `run build` invocation, got {:?}",
            command.args
        );
        assert_eq!(
            command.cwd,
            repo_root.join_components(&["apps", "web"]),
            "command runs in the package directory"
        );
        assert_eq!(command.serial_group, None);

        // Missing and empty scripts are no-ops.
        assert!(
            toolchain
                .task_command(repo_root, &package, "lint", None)
                .unwrap()
                .is_none()
        );
        assert!(
            toolchain
                .task_command(repo_root, &package, "empty", None)
                .unwrap()
                .is_none()
        );

        // Display shows the script text itself.
        assert_eq!(
            toolchain.task_display_command(&package, "build").as_deref(),
            Some("next build")
        );
        assert_eq!(toolchain.task_display_command(&package, "lint"), None);
    }

    #[cfg(windows)]
    #[test]
    fn npm_cmd_unwraps_to_node_and_npm_cli() {
        let tempdir = tempfile::tempdir().unwrap();
        let npm_cmd = tempdir.path().join("npm.cmd");
        let node = tempdir.path().join("node.exe");
        let npm_cli = tempdir
            .path()
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npm-cli.js");

        std::fs::write(&npm_cmd, "").unwrap();
        std::fs::write(&node, "").unwrap();
        std::fs::create_dir_all(npm_cli.parent().unwrap()).unwrap();
        std::fs::write(&npm_cli, "").unwrap();

        let (program, args) = package_manager_command(&PackageManager::Npm, &npm_cmd);

        assert_eq!(program, node.into_os_string());
        assert_eq!(args, vec![npm_cli.into_os_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn npm_cmd_falls_back_when_npm_cli_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let npm_cmd = tempdir.path().join("npm.cmd");
        std::fs::write(&npm_cmd, "").unwrap();

        let (program, args) = package_manager_command(&PackageManager::Npm, &npm_cmd);

        assert_eq!(program, npm_cmd.into_os_string());
        assert!(args.is_empty());
    }

    #[test]
    fn test_registry_lookup() {
        struct Fake(ToolchainId);
        impl Toolchain for Fake {
            fn id(&self) -> ToolchainId {
                self.0.clone()
            }
            fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
                Box::pin(async { Ok(Vec::new()) })
            }
        }

        let mut registry = ToolchainRegistry::new();
        registry.register(Arc::new(Fake(ToolchainId::JAVASCRIPT)));
        registry.register(Arc::new(Fake(ToolchainId::new("cargo"))));

        assert!(registry.get(&ToolchainId::JAVASCRIPT).is_some());
        assert!(registry.get(&ToolchainId::new("cargo")).is_some());
        assert!(registry.get(&ToolchainId::new("zig")).is_none());
        assert_eq!(registry.iter().count(), 2);
    }
}
