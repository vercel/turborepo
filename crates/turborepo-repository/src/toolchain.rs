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
