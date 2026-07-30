//! Repository contributors: construction-time adapters for language ecosystems.
//!
//! A [`RepositoryContributor`] discovers ecosystem packages and contributes
//! immutable observations. JavaScript is the first implementation
//! ([`JavaScriptContributor`]); additional producers (e.g. Cargo) register
//! alongside it during package-graph construction.
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
//! 3. Contributors are construction-scoped. Runtime consumers use immutable
//!    knowledge retained by the completed package graph.
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
//! - [`JavaScriptContributor::package_manager`]: package-manager resolution
//!   feeds dependency splitting and lockfile handling in the package graph
//!   builder. Lockfile handling gains a trait surface with external dependency
//!   hashing; dependency splitting remains JS-native for now.
//! - The prune command's JavaScript machinery (lockfile subgraphing,
//!   workspace-file rewriting, patches) remains on its native path rather than
//!   the immutable prune-knowledge path.

use std::{borrow::Cow, ffi::OsString, fmt, future::Future, pin::Pin, sync::Arc};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_errors::Spanned;

use crate::{
    change_knowledge::ChangeObservation,
    discovery::{self, PackageDiscovery},
    external_resolution::ExternalResolutionDomain,
    package_json::PackageJson,
    package_manager::PackageManager,
    prune_knowledge::PruneDomain,
    relationships::Relationship,
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

    /// The Rust toolchain: crates discovered from a Cargo workspace (see
    /// [`crate::cargo`]). Named for the language — the public axis users
    /// think in — while the implementation is Cargo-specific.
    /// Experimental, gated behind `futureFlags.experimentalCargoWorkspaces`.
    pub const RUST: ToolchainId = ToolchainId(Cow::Borrowed("rust"));

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

/// Compatibility output from native package discovery.
///
/// Identity and source facts feed [`crate::knowledge::RepositoryKnowledge`].
/// `descriptor` remains temporary compatibility input for relationship
/// classification and JavaScript construction paths. JavaScript packages retain
/// their parsed manifest; native producers can contribute normalized
/// relationships and tasks separately without synthesizing JavaScript
/// dependency maps. Cargo still supplies an empty descriptor only because graph
/// assembly currently requires a compatibility payload for every scope.
#[derive(Debug, Clone)]
pub struct DiscoveredPackage {
    /// Real user-facing identity, extracted by the native producer. `None`
    /// preserves JavaScript's historical unnamed-package suppression.
    name: Option<String>,
    /// Source provenance for the authored package name when it agrees with the
    /// authoritative identity.
    name_source: Option<Spanned<()>>,
    /// Whether this is a real package or an execution-only aggregate scope.
    scope_kind: DiscoveredScopeKind,
    /// Temporary relationship/task compatibility data; never identity or path
    /// authority.
    descriptor: PackageJson,
    /// Absolute path to the package's native manifest (`package.json`,
    /// `Cargo.toml`, ...).
    manifest_path: AbsoluteSystemPathBuf,
    /// Native relationship facts, already classified by the producer. `None`
    /// preserves compatibility by asking core to classify `descriptor`,
    /// regardless of the producer's toolchain id.
    native_relationships: Option<Vec<Relationship>>,
    /// Native task facts contributed at discovery time. `None` means the
    /// graph builder should derive tasks from `descriptor.scripts` when
    /// present (JavaScript). Cargo fills this with verb-table facts.
    native_tasks: Option<Vec<crate::native_tasks::NativeTask>>,
    task_contract: Option<crate::task_contracts::ScopeTaskContract>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveredScopeKind {
    Package,
    Aggregate,
}

pub(crate) struct DiscoveredPackageParts {
    pub name: Option<String>,
    pub name_source: Option<Spanned<()>>,
    pub scope_kind: DiscoveredScopeKind,
    pub descriptor: PackageJson,
    pub manifest_path: AbsoluteSystemPathBuf,
    pub native_relationships: Option<Vec<Relationship>>,
    pub native_tasks: Option<Vec<crate::native_tasks::NativeTask>>,
    pub task_contract: Option<crate::task_contracts::ScopeTaskContract>,
}

/// Parser-neutral observation of one contributed native workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    kind: String,
    path: AbsoluteSystemPathBuf,
}

impl WorkspaceRoot {
    pub fn new(kind: impl Into<String>, path: AbsoluteSystemPathBuf) -> Self {
        Self {
            kind: kind.into(),
            path,
        }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn path(&self) -> &AbsoluteSystemPath {
        &self.path
    }
}

/// One contributor's package/scope and native workspace-root observations.
#[derive(Debug, Default)]
pub struct DiscoveredPackages {
    packages: Vec<DiscoveredPackage>,
    workspace_roots: Vec<WorkspaceRoot>,
    external_resolutions: Vec<ExternalResolutionDomain>,
    change_observations: Vec<ChangeObservation>,
    prune_domains: Vec<Arc<dyn PruneDomain>>,
}

pub type DiscoveredPackagesParts = (
    Vec<DiscoveredPackage>,
    Vec<WorkspaceRoot>,
    Vec<ExternalResolutionDomain>,
    Vec<ChangeObservation>,
    Vec<Arc<dyn PruneDomain>>,
);

impl DiscoveredPackages {
    pub fn new(packages: Vec<DiscoveredPackage>, workspace_roots: Vec<WorkspaceRoot>) -> Self {
        Self {
            packages,
            workspace_roots,
            external_resolutions: Vec::new(),
            change_observations: Vec::new(),
            prune_domains: Vec::new(),
        }
    }

    pub fn with_external_resolution(mut self, resolution: ExternalResolutionDomain) -> Self {
        self.external_resolutions.push(resolution);
        self
    }

    pub fn with_change_observation(mut self, observation: ChangeObservation) -> Self {
        self.change_observations.push(observation);
        self
    }

    pub fn with_prune_domain(mut self, domain: Arc<dyn PruneDomain>) -> Self {
        self.prune_domains.push(domain);
        self
    }

    pub fn packages(&self) -> &[DiscoveredPackage] {
        &self.packages
    }

    pub fn workspace_roots(&self) -> &[WorkspaceRoot] {
        &self.workspace_roots
    }

    pub fn into_parts(self) -> DiscoveredPackagesParts {
        (
            self.packages,
            self.workspace_roots,
            self.external_resolutions,
            self.change_observations,
            self.prune_domains,
        )
    }
}

impl DiscoveredPackage {
    /// Construct a real package observation. The compatibility descriptor's
    /// name is normalized to the authoritative identity, making divergence
    /// structurally impossible. `None` preserves unnamed JavaScript package
    /// suppression.
    pub fn package(
        name: Option<String>,
        mut descriptor: PackageJson,
        manifest_path: AbsoluteSystemPathBuf,
    ) -> Self {
        let name_source = descriptor
            .name
            .as_ref()
            .filter(|source| name.as_deref() == Some(source.as_str()))
            .map(|source| source.to(()));
        descriptor.name = name.clone().map(Spanned::new);
        Self {
            name,
            name_source,
            scope_kind: DiscoveredScopeKind::Package,
            descriptor,
            manifest_path,
            native_relationships: None,
            native_tasks: None,
            task_contract: None,
        }
    }

    /// Construct a named aggregate execution scope and normalize its temporary
    /// compatibility descriptor to the same identity.
    pub fn aggregate(
        name: String,
        mut descriptor: PackageJson,
        manifest_path: AbsoluteSystemPathBuf,
    ) -> Self {
        descriptor.name = Some(Spanned::new(name.clone()));
        Self {
            name: Some(name),
            name_source: None,
            scope_kind: DiscoveredScopeKind::Aggregate,
            descriptor,
            manifest_path,
            native_relationships: None,
            native_tasks: None,
            task_contract: None,
        }
    }

    /// Supply already-classified native relationship facts. `Some(Vec::new())`
    /// explicitly declares that this package has no relationships; leaving
    /// this unset makes core classify `descriptor` for compatibility.
    pub fn with_native_relationships(mut self, relationships: Vec<Relationship>) -> Self {
        self.native_relationships = Some(relationships);
        self
    }

    /// Supply native task facts observed at discovery time.
    pub fn with_native_tasks(mut self, tasks: Vec<crate::native_tasks::NativeTask>) -> Self {
        self.native_tasks = Some(tasks);
        self
    }

    pub fn with_task_contract(
        mut self,
        contract: crate::task_contracts::ScopeTaskContract,
    ) -> Self {
        self.task_contract = Some(contract);
        self
    }

    pub(crate) fn into_parts(self) -> DiscoveredPackageParts {
        let Self {
            name,
            name_source,
            scope_kind,
            descriptor,
            manifest_path,
            native_relationships,
            native_tasks,
            task_contract,
        } = self;
        DiscoveredPackageParts {
            name,
            name_source,
            scope_kind,
            descriptor,
            manifest_path,
            native_relationships,
            native_tasks,
            task_contract,
        }
    }
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

/// The future returned by [`RepositoryContributor::discover_packages`]. Boxed
/// so the contributor trait stays object-safe.
pub type DiscoverPackagesFuture<'a> =
    Pin<Box<dyn Future<Output = Result<DiscoveredPackages, Error>> + Send + 'a>>;

/// A command resolved from native-task knowledge, as plain data. The executor
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

/// Build a [`TaskCommand`] from a task's `command` override: element 0 is
/// the program, the rest its arguments — nothing is prepended, by any
/// toolchain. Pass-through args append verbatim (turbo cannot know an
/// arbitrary command's separator convention). The working directory is the
/// package's own, uniformly across toolchains; the caller supplies its
/// toolchain-specific serial group.
pub fn override_task_command(
    context: &crate::package_graph::PackageTaskContext<'_>,
    override_command: &[String],
    pass_through_args: Option<&[String]>,
    serial_group: Option<String>,
) -> Option<TaskCommand> {
    let (program, args) = override_command.split_first()?;
    let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
    if let Some(pass_through_args) = pass_through_args {
        args.extend(pass_through_args.iter().map(OsString::from));
    }
    Some(TaskCommand {
        program: OsString::from(program),
        args,
        cwd: context.repository_root().resolve(context.directory()),
        serial_group,
    })
}

/// A language ecosystem that contributes packages to the repository.
///
/// See the module docs for the design rules trait methods must follow.
pub trait RepositoryContributor: Send + Sync {
    /// This contributor's ecosystem identifier.
    fn id(&self) -> ToolchainId;

    /// Discover this contributor's packages/scopes and native workspace roots
    /// in one observation envelope.
    fn discover_packages(&self) -> DiscoverPackagesFuture<'_>;
}

/// A Turborepo-served compile cache endpoint, as plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileCacheEndpoint {
    /// HTTP endpoint of the local compile-cache proxy.
    pub url: String,
    /// Bearer token the proxy requires.
    pub token: String,
    /// Absolute path to the compiler-wrapper executable — the running
    /// `turbo` binary itself, which embeds sccache and dispatches wrapper
    /// invocations to it (see [`COMPILE_CACHE_WRAPPER_ENV`]).
    pub wrapper: String,
    /// Port for the compile cache's background server, stable per
    /// repository. Isolates turbo's server from any user- or image-managed
    /// sccache server on the global default port, whose storage
    /// configuration would otherwise capture turbo's wrapper traffic.
    pub server_port: u16,
}

/// Environment variable marking task processes whose `RUSTC_WRAPPER` is the
/// turbo binary itself. The turbo entrypoint dispatches invocations carrying
/// this marker to the embedded sccache instead of the normal CLI.
pub const COMPILE_CACHE_WRAPPER_ENV: &str = "TURBO_SCCACHE_WRAPPER";

/// Watch classification projected from immutable change knowledge.
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

/// Default task behavior supplied by task-contract knowledge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskDefaults {
    /// Whether task logs and outputs are cacheable.
    pub cache: Option<bool>,
}

/// Platform-aware environment projection for one toolchain's I/O derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskIOEnvironment {
    values: std::collections::HashMap<String, String>,
    case_insensitive: bool,
}

impl TaskIOEnvironment {
    pub fn new(values: std::collections::HashMap<String, String>) -> Self {
        Self {
            values,
            case_insensitive: cfg!(windows),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        if self.case_insensitive {
            self.values
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        } else {
            self.values.get(name).map(String::as_str)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    #[cfg(test)]
    fn case_insensitive(values: std::collections::HashMap<String, String>) -> Self {
        Self {
            values,
            case_insensitive: true,
        }
    }
}

impl Default for TaskIOEnvironment {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

/// Run-scoped inputs that can affect toolchain-derived task I/O.
#[derive(Debug, Clone, Copy)]
pub struct TaskIOContext<'a> {
    pub task_args: Option<&'a [String]>,
    pub environment: &'a TaskIOEnvironment,
}

/// Whether a toolchain can resolve a task's automatic outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedOutputs {
    /// Output paths relative to the package directory. An empty list means the
    /// task has no toolchain-derived outputs.
    Resolved(Vec<String>),
    /// The task produces outputs, but their paths cannot be resolved safely.
    Unavailable,
}

impl Default for DerivedOutputs {
    fn default() -> Self {
        Self::Resolved(Vec::new())
    }
}

/// Whether all toolchain-derived inputs can participate in task hashing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DerivedInputSafety {
    #[default]
    Tracked,
    /// The task can read inputs that Turborepo cannot hash automatically.
    Untracked,
}

/// Hash wiring derived by task-contract knowledge for one task.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DerivedTaskIO {
    /// Extra input globs, relative to the package directory.
    pub input_globs: Vec<String>,
    /// When `Some`, overrides whether the package's own default (git-index
    /// based) file hashing applies to this task.
    pub package_default_inputs: Option<bool>,
    /// Env vars that participate in the task hash.
    pub env: Vec<String>,
    pub input_safety: DerivedInputSafety,
    pub outputs: DerivedOutputs,
}

/// The JavaScript contributor: packages discovered from `package.json`
/// manifests.
///
/// Wraps a [`PackageDiscovery`] strategy (local filesystem walk,
/// daemon-backed, or a composition) — the strategy decides *how* manifests
/// are found, the contributor owns *what a JavaScript package is*: it loads
/// and parses each manifest into the package descriptor.
pub struct JavaScriptContributor<P> {
    discovery: P,
    repo_root: AbsoluteSystemPathBuf,
    known_package_manager: Option<PackageManager>,
}

impl<P: PackageDiscovery + Send + Sync> JavaScriptContributor<P> {
    pub fn new(
        discovery: P,
        repo_root: AbsoluteSystemPathBuf,
        known_package_manager: Option<PackageManager>,
    ) -> Self {
        Self {
            discovery,
            repo_root,
            known_package_manager,
        }
    }

    /// The repository's JavaScript package manager.
    ///
    /// Known debt (see module docs): dependency splitting and lockfile
    /// handling in the package graph builder are not yet trait concerns, so
    /// they reach into the JavaScript toolchain directly for this.
    pub async fn package_manager(&self) -> Result<PackageManager, discovery::Error> {
        match self.known_package_manager.as_ref() {
            Some(package_manager) => Ok(package_manager.clone()),
            None => Ok(self.discovery.discover_packages().await?.package_manager),
        }
    }

    pub(crate) async fn workspace_root(&self) -> Result<WorkspaceRoot, Error> {
        let package_manager = self.package_manager().await?;
        Ok(WorkspaceRoot::new(
            package_manager.command(),
            self.repo_root.clone(),
        ))
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
pub(crate) fn package_manager_command(
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
pub(crate) fn package_manager_command(
    _package_manager: &PackageManager,
    package_manager_binary: &std::path::Path,
) -> (OsString, Vec<OsString>) {
    (package_manager_binary.as_os_str().to_owned(), Vec::new())
}

impl<P: PackageDiscovery + Send + Sync> RepositoryContributor for JavaScriptContributor<P> {
    fn id(&self) -> ToolchainId {
        ToolchainId::JAVASCRIPT
    }

    fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
        Box::pin(async move {
            use tracing::Instrument;
            let response = self
                .discovery
                .discover_packages()
                .instrument(tracing::info_span!("workspace_discovery"))
                .await?;
            let package_manager = match self.known_package_manager.as_ref() {
                Some(known) if known.command() != response.package_manager.command() => {
                    return Err(discovery::Error::InvalidResponse(format!(
                        "package manager family `{}` does not match authoritative family `{}`",
                        response.package_manager.command(),
                        known.command()
                    ))
                    .into());
                }
                Some(known) => known,
                None => &response.package_manager,
            };
            let workspace_root =
                WorkspaceRoot::new(package_manager.command(), self.repo_root.clone());
            // Parse manifests in parallel; manifest parsing dominates discovery
            // time on large repositories.
            let _span = tracing::info_span!("manifest_parse").entered();
            let packages = turborepo_rayon_compat::block_in_place(|| {
                use rayon::prelude::*;
                response
                    .workspaces
                    .into_par_iter()
                    .map(|workspace| {
                        let descriptor = PackageJson::load(&workspace.package_json)?;
                        let name = descriptor.name.as_ref().map(|name| name.as_inner().clone());
                        Ok(DiscoveredPackage::package(
                            name,
                            descriptor,
                            workspace.package_json,
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()
            })?;
            Ok(DiscoveredPackages::new(packages, vec![workspace_root]))
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
        let custom = ToolchainId::new("gleam");
        assert_ne!(custom, js);
        assert_eq!(custom.to_string(), "gleam");
        let dynamic = ToolchainId::new(String::from("python-uv"));
        assert_eq!(dynamic.as_str(), "python-uv");
    }

    #[test]
    fn pure_native_root_override_uses_repository_context() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let directory = turbopath::AnchoredSystemPath::new("").unwrap();
        let context = crate::package_graph::PackageTaskContext::new_for_test(
            crate::package_graph::PackageName::Root,
            &root,
            directory,
            None,
            crate::package_graph::PackageTaskContextKind::Root,
            None,
        );
        let argv = ["./scripts/release".to_string(), "check".to_string()];
        let pass_through = ["--locked".to_string()];

        let command = override_task_command(&context, &argv, Some(&pass_through), None)
            .expect("non-empty root override resolves");

        assert_eq!(command.program, OsString::from("./scripts/release"));
        assert_eq!(
            command.args,
            vec![OsString::from("check"), OsString::from("--locked")]
        );
        assert_eq!(command.cwd, root);
        assert_eq!(command.serial_group, None);
    }

    #[test]
    fn discovered_package_constructor_enforces_one_identity() {
        let path = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\repo\package.json"
        } else {
            "/repo/package.json"
        })
        .unwrap();
        let mismatched = PackageJson {
            name: Some(
                Spanned::new("payload-name".to_string())
                    .with_range(9..23)
                    .with_text(r#"{"name": "payload-name"}"#)
                    .with_path("package.json".into()),
            ),
            ..Default::default()
        };

        let package = DiscoveredPackage::package(
            Some("authoritative-name".to_string()),
            mismatched.clone(),
            path.clone(),
        )
        .into_parts();
        assert_eq!(package.name.as_deref(), Some("authoritative-name"));
        assert_eq!(package.name_source, None);
        assert_eq!(
            package.descriptor.name.as_ref().map(|name| name.as_str()),
            Some("authoritative-name")
        );

        let unnamed = DiscoveredPackage::package(None, mismatched, path.clone()).into_parts();
        assert_eq!(unnamed.name, None);
        assert_eq!(unnamed.name_source, None);
        assert_eq!(unnamed.descriptor.name, None);

        let authored_name = Spanned::new("authoritative-name".to_string())
            .with_range(9..29)
            .with_text(r#"{"name": "authoritative-name"}"#)
            .with_path("package.json".into());
        let matching = DiscoveredPackage::package(
            Some("authoritative-name".to_string()),
            PackageJson {
                name: Some(authored_name.clone()),
                ..Default::default()
            },
            path.clone(),
        )
        .into_parts();
        assert_eq!(matching.name_source, Some(authored_name.to(())));

        let aggregate =
            DiscoveredPackage::aggregate("workspace".to_string(), PackageJson::default(), path)
                .into_parts();
        assert_eq!(aggregate.name.as_deref(), Some("workspace"));
        assert_eq!(aggregate.name_source, None);
        assert_eq!(aggregate.scope_kind, DiscoveredScopeKind::Aggregate);
        assert_eq!(
            aggregate.descriptor.name.as_ref().map(|name| name.as_str()),
            Some("workspace")
        );
    }

    #[tokio::test]
    async fn javascript_discovery_reports_only_the_base_canonical_manager_root() {
        #[derive(Clone)]
        struct StubDiscovery(discovery::DiscoveryResponse);

        impl PackageDiscovery for StubDiscovery {
            async fn discover_packages(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                Ok(self.0.clone())
            }

            async fn discover_packages_blocking(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                self.discover_packages().await
            }
        }

        let tempdir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let package_root = repo_root.join_components(&["packages", "app"]);
        std::fs::create_dir_all(package_root.as_std_path()).unwrap();
        let package_json = package_root.join_component("package.json");
        package_json
            .create_with_contents(r#"{"name":"app"}"#)
            .unwrap();
        let toolchain = JavaScriptContributor::new(
            StubDiscovery(discovery::DiscoveryResponse {
                workspaces: vec![discovery::WorkspaceData {
                    package_json,
                    turbo_json: None,
                }],
                package_manager: PackageManager::Pnpm6,
            }),
            repo_root.clone(),
            None,
        );

        let discovered = toolchain.discover_packages().await.unwrap();
        assert_eq!(discovered.packages().len(), 1);
        assert_eq!(discovered.workspace_roots().len(), 1);
        assert_eq!(discovered.workspace_roots()[0].kind(), "pnpm");
        assert_eq!(discovered.workspace_roots()[0].path(), repo_root.as_ref());
    }

    #[test]
    fn test_javascript_task_command() {
        let repo_root_buf =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let repo_root = repo_root_buf.as_ref() as &AbsoluteSystemPath;
        let package = crate::package_graph::PackageInfo {
            package_json: PackageJson::from_value(serde_json::json!({
                "name": "stale-web",
                "scripts": { "build": "next build", "empty": "" }
            }))
            .unwrap(),
        };
        let package_directory = turbopath::AnchoredSystemPath::new("apps/web").unwrap();
        let context = crate::package_graph::PackageTaskContext::new_for_test(
            "web".into(),
            repo_root,
            package_directory,
            Some(&package),
            crate::package_graph::PackageTaskContextKind::Package,
            Some(&ToolchainId::JAVASCRIPT),
        );

        let command = crate::native_tasks::resolve_task_command(
            &context,
            context.native_tasks().get("build").expect("build task"),
            Some(&PackageManager::Npm),
            which::which("npm").ok().as_deref(),
            None,
            None,
            None,
        )
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
        assert!(!context.native_tasks().defines("lint"));
        assert!(!context.native_tasks().defines("empty"));

        // Display shows the script text itself.
        assert_eq!(
            context
                .native_tasks()
                .get("build")
                .and_then(|task| task.display()),
            Some("next build")
        );
        assert_eq!(
            context
                .native_tasks()
                .get("lint")
                .and_then(|task| task.display()),
            None
        );

        // A `command` override replaces the whole package-manager
        // indirection: argv[0] is the program, nothing is prepended, cwd is
        // the package directory. It also defines tasks no script does, and
        // pass-through args append verbatim.
        let override_argv = vec!["vitest".to_string(), "run".to_string()];
        let cmd = override_task_command(
            &context,
            &override_argv,
            Some(&["--bail".to_string()]),
            None,
        )
        .expect("override defines the task");
        assert_eq!(cmd.program, OsString::from("vitest"));
        assert_eq!(
            cmd.args,
            vec![OsString::from("run"), OsString::from("--bail")]
        );
        assert_eq!(cmd.cwd, repo_root.resolve(package_directory));
        assert_eq!(cmd.serial_group, None);
    }

    #[test]
    fn task_io_environment_supports_windows_casing() {
        let environment = TaskIOEnvironment::case_insensitive(std::collections::HashMap::from([(
            "Cargo_Build_Target".to_string(),
            "x86_64-pc-windows-msvc".to_string(),
        )]));
        assert_eq!(
            environment.get("CARGO_BUILD_TARGET"),
            Some("x86_64-pc-windows-msvc")
        );
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
}
