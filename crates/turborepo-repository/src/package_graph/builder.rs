use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use miette::{Diagnostic, Report};
use petgraph::graph::{Graph, NodeIndex};
use tracing::warn;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_lockfiles::Lockfile;

use super::{
    ExternalResolutionKnowledge, PackageGraph, PackageName, PackageNode,
    dep_splitter::{DependencySplitter, WorkspacePathIndex},
    javascript,
};
use crate::{
    discovery::{
        self, CachingPackageDiscovery, LocalPackageDiscoveryBuilder, PackageDiscovery,
        PackageDiscoveryBuilder,
    },
    external_resolution::{ExternalResolutionDomain, ExternalResolutionGeneration},
    knowledge::{
        PackageScopeObservation, RelationshipGroup, RelationshipKnowledge, RepositoryKnowledge,
        ScopeKind, WorkspaceRootObservation,
    },
    package_json::{DependencyKind, PackageJson},
    package_manager::{PackageManager, pnpm::PnpmCatalogs},
    relationships::{Relationship, RelationshipTarget},
    toolchain::{
        DiscoveredPackage, DiscoveredPackageParts, DiscoveredScopeKind, JavaScriptContributor,
        RepositoryContributor, ToolchainId,
    },
};

pub struct PackageGraphBuilder<'a, T> {
    repo_root: &'a AbsoluteSystemPath,
    /// The root `package.json`, when the repository has one. Absent for a
    /// pure Cargo workspace (`futureFlags.experimentalCargoWorkspaces` with
    /// no root `package.json`): there is no JavaScript project, so no
    /// package manager, lockfile, or root manifest to resolve.
    root_package_json: Option<PackageJson>,
    is_single_package: bool,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_discovery: T,
    package_manager: Option<PackageManager>,
    /// Toolchains registered in addition to JavaScript (e.g. Cargo when
    /// `futureFlags.experimentalCargoWorkspaces` is enabled). Their packages
    /// are discovered alongside JavaScript packages; name collisions across
    /// toolchains are a hard error.
    extra_contributors: Vec<Arc<dyn RepositoryContributor>>,
}

#[derive(Debug, Diagnostic, thiserror::Error)]
pub enum Error {
    #[error("Could not resolve workspace.")]
    #[diagnostic(transparent)]
    PackageManager(#[from] crate::package_manager::Error),
    #[error(
        "Failed to add workspace \"{name}\" from \"{path}\", it already exists at \
         \"{existing_path}\""
    )]
    DuplicateWorkspace {
        name: String,
        path: String,
        existing_path: String,
    },
    #[error("Path error: {0}")]
    Path(#[from] turbopath::PathError),
    #[diagnostic(transparent)]
    #[error(transparent)]
    PackageJson(#[from] crate::package_json::Error),
    #[error("package.json must have a name field:\n{0}")]
    PackageJsonMissingName(AbsoluteSystemPathBuf),
    #[error("package definition {path} is outside repository root {repository_root}")]
    DefinitionOutsideRepository {
        path: AbsoluteSystemPathBuf,
        repository_root: AbsoluteSystemPathBuf,
    },
    #[error("missing construction descriptor for discovered scope {name}")]
    MissingDescriptor { name: String },
    #[error("construction descriptor {name} has no authoritative discovered scope")]
    UnexpectedDescriptor { name: String },
    #[error("repository package knowledge was not constructed")]
    MissingRepositoryKnowledge,
    #[error("repository relationship knowledge was not constructed")]
    MissingRelationshipKnowledge,
    #[error("external resolution generation failed: {0}")]
    ExternalResolution(String),
    #[error("native task knowledge generation failed: {0}")]
    NativeTasks(String),
    #[error("Task contract knowledge error: {0}")]
    TaskContracts(String),
    #[error("Prune knowledge error: {0}")]
    PruneKnowledge(String),
    #[error("change knowledge is invalid: {0}")]
    ChangeKnowledge(String),
    #[error("package definition {path} is claimed by both {existing_identity} and {identity}")]
    DuplicateDefinitionPath {
        path: AnchoredSystemPathBuf,
        identity: String,
        existing_identity: String,
    },
    #[error("package or aggregate scope at {path} uses reserved root identity //")]
    ReservedRootIdentity { path: AnchoredSystemPathBuf },
    #[error("relationship source {identity} has no authoritative repository scope")]
    UnknownRelationshipSource { identity: String },
    #[error("internal relationship target {identity} has no authoritative repository scope")]
    UnknownRelationshipTarget { identity: String },
    #[error("repository contributor {id} was registered more than once")]
    DuplicateContributor { id: ToolchainId },
    #[error(
        "toolchain {toolchain} contributed multiple workspace roots: accepted {accepted_kind} \
         root {accepted_root}, conflicting {conflicting_kind} root {conflicting_root}"
    )]
    MultipleWorkspaceRoots {
        toolchain: ToolchainId,
        accepted_kind: String,
        accepted_root: AnchoredSystemPathBuf,
        conflicting_kind: String,
        conflicting_root: AnchoredSystemPathBuf,
    },
    #[error("{kind} workspace root {path} is outside repository root {repository_root}")]
    WorkspaceRootOutsideRepository {
        kind: String,
        path: AbsoluteSystemPathBuf,
        repository_root: AbsoluteSystemPathBuf,
    },
    #[error("ecosystem {toolchain} contributed packages without a workspace root")]
    MissingWorkspaceRoot { toolchain: ToolchainId },
    #[error(transparent)]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error(transparent)]
    Discovery(#[from] crate::discovery::Error),
    #[error(transparent)]
    Contribution(Box<dyn std::error::Error + Send + Sync>),
}

// JavaScript contribution errors map onto the pre-existing variants rather than
// new ones: consumers match on `Error::PackageJson` (diagnostic rendering,
// io-NotFound telemetry in the run builder), and those contracts must not
// depend on whether the error surfaced through a contributor.
impl From<crate::toolchain::Error> for Error {
    fn from(err: crate::toolchain::Error) -> Self {
        match err {
            crate::toolchain::Error::Discovery(err) => Error::Discovery(err),
            crate::toolchain::Error::Descriptor(err) => Error::PackageJson(err),
            crate::toolchain::Error::Failed(err) => Error::Contribution(err),
        }
    }
}

impl From<crate::knowledge::Error> for Error {
    fn from(error: crate::knowledge::Error) -> Self {
        match error {
            crate::knowledge::Error::DuplicateScope {
                name,
                path,
                existing_path,
            } => Error::DuplicateWorkspace {
                name,
                path: path.to_string(),
                existing_path: existing_path.to_string(),
            },
            crate::knowledge::Error::DefinitionOutsideRepository {
                path,
                repository_root,
            } => Error::DefinitionOutsideRepository {
                path,
                repository_root,
            },
            crate::knowledge::Error::DuplicateDefinitionPath {
                path,
                identity,
                existing_identity,
            } => Error::DuplicateDefinitionPath {
                path,
                identity,
                existing_identity,
            },
            crate::knowledge::Error::MultipleWorkspaceRoots {
                toolchain,
                accepted_kind,
                accepted_root,
                conflicting_kind,
                conflicting_root,
            } => Error::MultipleWorkspaceRoots {
                toolchain,
                accepted_kind,
                accepted_root,
                conflicting_kind,
                conflicting_root,
            },
            crate::knowledge::Error::WorkspaceRootOutsideRepository {
                kind,
                path,
                repository_root,
            } => Error::WorkspaceRootOutsideRepository {
                kind,
                path,
                repository_root,
            },
            crate::knowledge::Error::MissingWorkspaceRoot { toolchain } => {
                Error::MissingWorkspaceRoot { toolchain }
            }
            crate::knowledge::Error::ReservedRootIdentity { path } => {
                Error::ReservedRootIdentity { path }
            }
            crate::knowledge::Error::UnknownRelationshipSource { identity } => {
                Error::UnknownRelationshipSource { identity }
            }
            crate::knowledge::Error::UnknownRelationshipTarget { identity } => {
                Error::UnknownRelationshipTarget { identity }
            }
            crate::knowledge::Error::Path(error) => Error::Path(error),
        }
    }
}

/// Attempts to extract the file path that caused the error from the error chain
/// Falls back to the lockfile path if no specific file can be determined
fn extract_file_path_from_error(
    error: &Error,
    package_manager: &crate::package_manager::PackageManager,
    repo_root: &AbsoluteSystemPath,
) -> AbsoluteSystemPathBuf {
    match error {
        Error::PackageJsonMissingName(path) => path.clone(),
        // TODO: We're handling every other error here. We could handle situations where the
        // lockfile isn't the issue better.
        _ => package_manager.lockfile_path(repo_root),
    }
}

impl<'a> PackageGraphBuilder<'a, LocalPackageDiscoveryBuilder> {
    pub fn new(repo_root: &'a AbsoluteSystemPath, root_package_json: PackageJson) -> Self {
        Self::new_optional(repo_root, Some(root_package_json))
    }

    /// Build over a repository that may have no root `package.json`. When
    /// `root_package_json` is `None`, the JavaScript contributor supplies
    /// nothing (no package manager, no lockfile); the graph is populated
    /// entirely by the extra contributors registered via
    /// [`PackageGraphBuilder::with_contributor`] (Cargo). When it is `Some`,
    /// this behaves exactly like [`PackageGraphBuilder::new`].
    pub fn new_optional(
        repo_root: &'a AbsoluteSystemPath,
        root_package_json: Option<PackageJson>,
    ) -> Self {
        Self {
            package_discovery: LocalPackageDiscoveryBuilder::new(
                repo_root.to_owned(),
                None,
                root_package_json.clone(),
            ),
            repo_root,
            root_package_json,
            is_single_package: false,
            package_jsons: None,
            lockfile: None,
            package_manager: None,
            extra_contributors: Vec::new(),
        }
    }

    pub fn with_allow_no_package_manager(mut self, allow_no_package_manager: bool) -> Self {
        self.package_discovery
            .with_allow_no_package_manager(allow_no_package_manager);
        self
    }

    pub fn with_package_manager(mut self, package_manager: PackageManager) -> Self {
        self.package_manager = Some(package_manager.clone());
        self.package_discovery
            .with_package_manager(Some(package_manager));
        self
    }
}

impl<'a, P> PackageGraphBuilder<'a, P> {
    pub fn with_single_package_mode(mut self, is_single: bool) -> Self {
        self.is_single_package = is_single;
        self
    }

    pub fn with_package_jsons(
        mut self,
        package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    ) -> Self {
        self.package_jsons = package_jsons;
        self
    }

    pub fn with_lockfile(mut self, lockfile: Option<Box<dyn Lockfile>>) -> Self {
        self.lockfile = lockfile;
        self
    }

    /// Register a contributor in addition to JavaScript. Its packages are
    /// discovered alongside JavaScript packages; a package name collision
    /// across ecosystems are a hard error, like any duplicate package name.
    pub fn with_contributor(mut self, contributor: Arc<dyn RepositoryContributor>) -> Self {
        self.extra_contributors.push(contributor);
        self
    }

    /// Enable Cargo repository contribution for this graph generation.
    pub fn with_cargo(self) -> Self {
        let repo_root = self.repo_root.to_owned();
        self.with_contributor(crate::cargo::CargoContributor::new(repo_root))
    }

    /// Enable uv (Python) repository contribution for this graph generation.
    pub fn with_uv(self) -> Self {
        let repo_root = self.repo_root.to_owned();
        self.with_contributor(crate::uv::UvContributor::new(repo_root))
    }

    /// Set the package discovery strategy to use. Note that whatever strategy
    /// selected here will be wrapped in a `CachingPackageDiscovery` to
    /// prevent unnecessary work during building.
    pub fn with_package_discovery<P2: PackageDiscoveryBuilder>(
        self,
        discovery: P2,
    ) -> PackageGraphBuilder<'a, P2> {
        PackageGraphBuilder {
            repo_root: self.repo_root,
            root_package_json: self.root_package_json,
            is_single_package: self.is_single_package,
            package_jsons: self.package_jsons,
            lockfile: self.lockfile,
            package_discovery: discovery,
            package_manager: self.package_manager,
            extra_contributors: self.extra_contributors,
        }
    }
}

impl<T> PackageGraphBuilder<'_, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send + Sync + 'static,
    T::Error: Into<crate::package_manager::Error>,
{
    /// Build the `PackageGraph`.
    #[tracing::instrument(skip(self))]
    pub async fn build(mut self) -> Result<PackageGraph, Error> {
        let is_single_package = self.is_single_package;

        // If no pre-supplied lockfile, start reading it on a blocking thread
        // concurrently with package discovery + JSON parsing. A pure Cargo
        // workspace has no root package.json and therefore no JavaScript
        // package manager or lockfile to read.
        let known_pm = self
            .package_manager
            .take()
            .or_else(|| {
                self.root_package_json
                    .as_ref()
                    .and_then(|root_package_json| {
                        PackageManager::get_package_manager(self.repo_root, root_package_json).ok()
                    })
            })
            .map(|pm| pm.with_resolved_nub_lockfile(self.repo_root));
        self.package_manager.clone_from(&known_pm);
        let lockfile_future = if !is_single_package && self.lockfile.is_none() {
            if let (Some(pm), Some(root_package_json)) =
                (known_pm.clone(), self.root_package_json.clone())
            {
                let repo_root = self.repo_root.to_owned();
                Some(tokio::task::spawn_blocking(
                    move || -> Option<Box<dyn Lockfile>> {
                        pm.read_lockfile(&repo_root, &root_package_json).ok()
                    },
                ))
            } else {
                None
            }
        } else {
            None
        };

        let state = BuildState::new(self)?;

        match is_single_package {
            true => Ok(state.build_single_package_graph().await?),
            false => {
                let state = state.parse_package_jsons().await?;
                let state = state.resolve_lockfile(lockfile_future).await?;
                Ok(state.build_inner().await?)
            }
        }
    }
}

struct BuildState<'a, S, T> {
    repo_root: &'a AbsoluteSystemPath,
    single: bool,
    assembler: PackageGraphAssembler,
    knowledge: Option<Arc<RepositoryKnowledge>>,
    relationship_knowledge: Option<Arc<RelationshipKnowledge>>,
    native_relationships: HashMap<String, Vec<Relationship>>,
    native_external_resolutions: Vec<ExternalResolutionDomain>,
    native_task_observations: Vec<crate::native_tasks::NativeTaskObservation>,
    native_change_observations: Vec<crate::change_knowledge::ChangeObservation>,
    native_prune_domains: Vec<Arc<dyn crate::prune_knowledge::PruneDomain>>,
    /// The root `package.json`, absent for a pure Cargo workspace. See
    /// [`PackageGraphBuilder::root_package_json`].
    root_package_json: Option<PackageJson>,
    lockfile: Option<Box<dyn Lockfile>>,
    package_manager: Option<PackageManager>,
    package_jsons: Option<HashMap<AbsoluteSystemPathBuf, PackageJson>>,
    state: std::marker::PhantomData<S>,
    /// The JavaScript contributor, kept typed. Package-manager resolution for
    /// dependency splitting and lockfile handling reaches through this —
    /// documented debt, see `crate::toolchain` module docs. Absent for a
    /// pure Cargo workspace, where there is no JavaScript project to resolve
    /// a package manager or lockfile from.
    javascript: Option<Arc<JavaScriptContributor<T>>>,
    /// Additional package contributors. JavaScript is kept typed above so
    /// pre-parsed manifest input cannot be routed through an open ID.
    extra_contributors: Vec<Arc<dyn RepositoryContributor>>,
}

struct PackageGraphAssembler {
    package_jsons: HashMap<PackageName, PackageJson>,
    workspace_graph: Graph<PackageNode, DependencyKind>,
    root_node_index: NodeIndex,
    root_workspace_index: NodeIndex,
    node_lookup: HashMap<PackageNode, NodeIndex>,
}

struct PackageGraphAssembly {
    workspace_graph: Graph<PackageNode, DependencyKind>,
    root_node_index: NodeIndex,
    root_workspace_index: NodeIndex,
    node_lookup: HashMap<PackageNode, NodeIndex>,
}

struct ObservedPackage {
    scope: PackageScopeObservation,
    descriptor: Option<(String, PackageJson)>,
    native_relationships: Option<(String, Vec<Relationship>)>,
    native_tasks: Option<crate::native_tasks::NativeTaskObservation>,
}

impl PackageGraphAssembler {
    fn new(root_package_json: Option<PackageJson>) -> Self {
        let mut package_jsons = HashMap::new();
        if let Some(root_package_json) = root_package_json {
            package_jsons.insert(PackageName::Root, root_package_json);
        }

        let mut workspace_graph = Graph::new();
        let root_node_index = workspace_graph.add_node(PackageNode::Root);
        let root_workspace = PackageNode::Workspace(PackageName::Root);
        let root_workspace_index = workspace_graph.add_node(root_workspace.clone());
        workspace_graph.add_edge(
            root_workspace_index,
            root_node_index,
            DependencyKind::Production,
        );

        let mut node_lookup = HashMap::new();
        node_lookup.insert(PackageNode::Root, root_node_index);
        node_lookup.insert(root_workspace, root_workspace_index);

        Self {
            package_jsons,
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
        }
    }

    fn reserve(&mut self, additional: usize) {
        self.package_jsons.reserve(additional);
        self.node_lookup.reserve(additional);
    }

    fn add_scope(&mut self, name: PackageName, descriptor: Option<PackageJson>) {
        if let Some(descriptor) = descriptor {
            self.package_jsons.insert(name.clone(), descriptor);
        }
        let node = PackageNode::Workspace(name);
        let idx = self.workspace_graph.add_node(node.clone());
        self.node_lookup.insert(node, idx);
    }

    fn add_knowledge(
        &mut self,
        knowledge: &RepositoryKnowledge,
        descriptors: Vec<(String, PackageJson)>,
    ) -> Result<(), Error> {
        let mut descriptors: HashMap<_, _> = descriptors.into_iter().collect();
        self.reserve(knowledge.packages().count() + knowledge.aggregate_scopes().count());

        for package in knowledge.packages() {
            let name = package.identity();
            self.add_scope(
                PackageName::Other(name.to_string()),
                Some(
                    descriptors
                        .remove(name)
                        .ok_or_else(|| Error::MissingDescriptor {
                            name: name.to_string(),
                        })?,
                ),
            );
        }
        for aggregate in knowledge.aggregate_scopes() {
            let name = aggregate.identity();
            self.add_scope(
                PackageName::Other(name.to_string()),
                Some(
                    descriptors
                        .remove(name)
                        .ok_or_else(|| Error::MissingDescriptor {
                            name: name.to_string(),
                        })?,
                ),
            );
        }
        if let Some(name) = descriptors.keys().next() {
            return Err(Error::UnexpectedDescriptor { name: name.clone() });
        }
        Ok(())
    }

    fn project_relationships(
        &mut self,
        relationships: &RelationshipKnowledge,
    ) -> Result<(), Error> {
        for group in relationships.groups() {
            let identity = group.source();
            let name = package_name_from_identity(identity);
            if !self.package_jsons.contains_key(&name) {
                return Err(Error::MissingDescriptor {
                    name: identity.to_string(),
                });
            }
            let mut seen = HashSet::new();
            let mut internal = HashMap::<&str, DependencyKind>::new();
            for relationship in group.relationships() {
                if !relationship.orders_tasks() {
                    continue;
                }
                if !seen.insert(relationship.declaration_name()) {
                    continue;
                }
                match (relationship.kind(), relationship.target()) {
                    (DependencyKind::Peer { .. }, _) => {}
                    (
                        DependencyKind::Production
                        | DependencyKind::Optional
                        | DependencyKind::Development,
                        RelationshipTarget::Internal(target),
                    ) => {
                        let kind = match relationship.kind() {
                            DependencyKind::Optional => DependencyKind::Production,
                            kind => kind,
                        };
                        internal.entry(target).or_insert(kind);
                    }
                    (
                        DependencyKind::Production
                        | DependencyKind::Optional
                        | DependencyKind::Development,
                        RelationshipTarget::UnresolvedExternal { .. },
                    ) => {}
                }
            }
            let node_idx = self
                .node_lookup
                .get(&PackageNode::Workspace(name.clone()))
                .copied()
                .ok_or_else(|| Error::MissingDescriptor {
                    name: identity.to_string(),
                })?;
            if internal.is_empty() {
                self.workspace_graph.add_edge(
                    node_idx,
                    self.root_node_index,
                    DependencyKind::Production,
                );
            }
            for (dependency, kind) in internal {
                let dependency_idx = self
                    .node_lookup
                    .get(&PackageNode::Workspace(package_name_from_identity(
                        dependency,
                    )))
                    .copied()
                    .ok_or_else(|| Error::UnknownRelationshipTarget {
                        identity: dependency.to_string(),
                    })?;
                self.workspace_graph
                    .add_edge(node_idx, dependency_idx, kind);
            }
        }
        Ok(())
    }

    fn finish(self) -> PackageGraphAssembly {
        PackageGraphAssembly {
            workspace_graph: self.workspace_graph,
            root_node_index: self.root_node_index,
            root_workspace_index: self.root_workspace_index,
            node_lookup: self.node_lookup,
        }
    }
}

// Allows us to perform workspace discovery and parse package jsons
enum ResolvedPackageManager {}

// Allows us to build the workspace graph and list over external dependencies
enum ResolvedWorkspaces {}

// Allows us to collect all transitive deps
enum ResolvedLockfile {}

impl<'a, T> BuildState<'a, ResolvedPackageManager, T>
where
    T: PackageDiscoveryBuilder,
    T::Output: Send + Sync + 'static,
    T::Error: Into<crate::package_manager::Error>,
{
    fn new(
        builder: PackageGraphBuilder<'a, T>,
    ) -> Result<BuildState<'a, ResolvedPackageManager, CachingPackageDiscovery<T::Output>>, Error>
    {
        let PackageGraphBuilder {
            repo_root,
            root_package_json,
            is_single_package: single,

            package_jsons,
            lockfile,
            package_discovery,
            package_manager,
            extra_contributors,
        } = builder;
        // Pure Cargo workspace: with no root package.json there is no
        // JavaScript project, so the typed JavaScript contributor is neither
        // constructed nor queried for a package manager. The graph is built
        // entirely from the extra contributors (Cargo).
        let no_javascript = root_package_json.is_none();
        let assembler = PackageGraphAssembler::new(root_package_json.clone());

        // The discovery strategy is shared (via the JavaScript contributor)
        // between package discovery and package-manager resolution; the
        // caching wrapper guarantees the underlying strategy runs once. For a
        // pure Cargo workspace there is no JavaScript project, so discovery and
        // the typed contributor are not constructed.
        let mut additional_contributors: Vec<Arc<dyn RepositoryContributor>> = Vec::new();
        let javascript = if no_javascript {
            None
        } else {
            let javascript = Arc::new(JavaScriptContributor::new(
                CachingPackageDiscovery::new(package_discovery.build().map_err(Into::into)?),
                repo_root.to_owned(),
                package_manager,
            ));
            Some(javascript)
        };
        for contributor in extra_contributors {
            let id = contributor.id();
            if (javascript.is_some() && id == ToolchainId::JAVASCRIPT)
                || additional_contributors
                    .iter()
                    .any(|existing| existing.id() == id)
            {
                return Err(Error::DuplicateContributor { id });
            }
            additional_contributors.push(contributor);
        }

        Ok(BuildState {
            repo_root,
            single,

            assembler,
            knowledge: None,
            relationship_knowledge: None,
            native_relationships: HashMap::new(),
            native_external_resolutions: Vec::new(),
            native_task_observations: Vec::new(),
            native_change_observations: Vec::new(),
            native_prune_domains: Vec::new(),
            lockfile,
            package_manager: None,
            package_jsons,
            root_package_json,
            state: std::marker::PhantomData,
            javascript,
            extra_contributors: additional_contributors,
        })
    }
}

impl<'a, T: PackageDiscovery + Send + Sync> BuildState<'a, ResolvedPackageManager, T> {
    fn observe_package(
        &self,
        toolchain: ToolchainId,
        package: DiscoveredPackage,
    ) -> Result<ObservedPackage, Error> {
        let DiscoveredPackageParts {
            name,
            name_source,
            scope_kind,
            descriptor: json,
            manifest_path,
            native_relationships,
            native_tasks,
            task_contract,
        } = package.into_parts();
        // Producer-resolved external identities are contributed to the
        // resolution generation separately.
        let task_contract =
            task_contract.unwrap_or_else(crate::task_contracts::ScopeTaskContract::empty);
        if let Some(contract_toolchain) = task_contract.toolchain()
            && contract_toolchain != &toolchain
        {
            return Err(Error::TaskContracts(format!(
                "scope task contract belongs to {contract_toolchain}, not {toolchain}"
            )));
        }
        let native_task_observation = match (name.as_ref(), native_tasks) {
            (Some(identity), Some(tasks)) => Some(crate::native_tasks::NativeTaskObservation {
                scope: identity.clone(),
                tasks,
                task_contract,
            }),
            (Some(identity), None) => {
                let mut observation =
                    crate::native_tasks::observation_from_package_json(identity.clone(), &json);
                observation.task_contract = task_contract;
                Some(observation)
            }
            (None, _) => None,
        };
        let observation = PackageScopeObservation {
            identity: name.clone(),
            name_source,
            definition_path: manifest_path.clone(),
            toolchain,
            scope_kind: match scope_kind {
                DiscoveredScopeKind::Package => ScopeKind::Package,
                DiscoveredScopeKind::Aggregate => ScopeKind::Aggregate,
            },
        };
        let native_relationships = name.clone().zip(native_relationships);
        let descriptor = name.map(|name| (name, json));
        if descriptor.is_none() {
            tracing::debug!(
                "ignoring package definition at {} since it has no name",
                manifest_path
            );
        }
        Ok(ObservedPackage {
            scope: observation,
            descriptor,
            native_relationships,
            native_tasks: native_task_observation,
        })
    }

    // need our own type
    #[tracing::instrument(skip(self))]
    async fn parse_package_jsons(mut self) -> Result<BuildState<'a, ResolvedWorkspaces, T>, Error> {
        // A pre-supplied set of parsed package.json files (used by the
        // package-change watcher and tests) stands in for JavaScript
        // discovery only; other toolchains always discover for themselves.
        let mut discovered: Vec<(ToolchainId, DiscoveredPackage)> = Vec::new();
        let mut workspace_roots = Vec::new();
        let mut contributor_outputs = Vec::with_capacity(self.extra_contributors.len() + 1);
        if let Some(javascript) = self.javascript.as_ref() {
            let output = match self.package_jsons.take() {
                Some(package_jsons) => {
                    javascript
                        .discover_preparsed_packages(package_jsons)
                        .await?
                }
                None => javascript.discover_packages().await?,
            };
            contributor_outputs.push((ToolchainId::JAVASCRIPT, output));
        }
        for contributor in &self.extra_contributors {
            let id = contributor.id();
            let output = contributor.discover_packages().await?;
            contributor_outputs.push((id, output));
        }
        for (id, output) in contributor_outputs {
            let (packages, roots, external_resolutions, changes, prune_domains) =
                output.into_parts();
            self.native_external_resolutions
                .extend(external_resolutions);
            self.native_change_observations.extend(changes);
            self.native_prune_domains.extend(prune_domains);
            workspace_roots.extend(
                roots
                    .into_iter()
                    .map(|root| WorkspaceRootObservation::new(root, id.clone())),
            );
            discovered.extend(packages.into_iter().map(|package| (id.clone(), package)));
        }

        let _span = tracing::info_span!("add_packages").entered();
        let mut observations = Vec::with_capacity(discovered.len());
        let mut descriptors = Vec::with_capacity(discovered.len());
        for (toolchain, package) in discovered {
            let observed = self.observe_package(toolchain, package)?;
            observations.push(observed.scope);
            if let Some(descriptor) = observed.descriptor {
                descriptors.push(descriptor);
            }
            if let Some((source, relationships)) = observed.native_relationships {
                self.native_relationships.insert(source, relationships);
            }
            if let Some(tasks) = observed.native_tasks {
                self.native_task_observations.push(tasks);
            }
        }
        let root_name = self.root_package_json.as_ref().map(|package_json| {
            package_json
                .name
                .as_ref()
                .map(|name| name.as_inner().clone())
        });
        let knowledge = Arc::new(RepositoryKnowledge::build(
            self.repo_root,
            root_name,
            &observations,
            &workspace_roots,
        )?);
        for root in knowledge.workspace_roots() {
            tracing::debug!(
                kind = root.kind(),
                path = %root.path(),
                toolchain = %root.toolchain(),
                "observed native workspace root"
            );
        }
        self.assembler.add_knowledge(&knowledge, descriptors)?;
        self.knowledge = Some(knowledge);

        let Self {
            repo_root,
            single,
            assembler,
            knowledge,
            relationship_knowledge,
            native_relationships,
            native_external_resolutions,
            native_task_observations,
            native_change_observations,
            native_prune_domains,
            root_package_json,
            lockfile,
            package_manager,
            javascript,
            extra_contributors,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            assembler,
            knowledge,
            relationship_knowledge,
            native_relationships,
            native_external_resolutions,
            native_task_observations,
            native_change_observations,
            native_prune_domains,
            root_package_json,
            lockfile,
            package_manager,
            javascript,
            extra_contributors,
            package_jsons: None,
            state: std::marker::PhantomData,
        })
    }

    async fn build_single_package_graph(self) -> Result<PackageGraph, Error> {
        let Self {
            single,
            assembler,
            knowledge,
            relationship_knowledge: _,
            native_relationships: _,
            native_external_resolutions: _,
            native_task_observations,
            root_package_json,
            lockfile,
            javascript,
            repo_root,
            ..
        } = self;
        let workspace_roots = match &javascript {
            Some(javascript) => vec![WorkspaceRootObservation::new(
                javascript.workspace_root().await?,
                ToolchainId::JAVASCRIPT,
            )],
            None => Vec::new(),
        };
        let knowledge = match knowledge {
            Some(knowledge) => knowledge,
            None => {
                let root_name = root_package_json.as_ref().map(|package_json| {
                    package_json
                        .name
                        .as_ref()
                        .map(|name| name.as_inner().clone())
                });
                Arc::new(RepositoryKnowledge::build(
                    repo_root,
                    root_name,
                    &[],
                    &workspace_roots,
                )?)
            }
        };
        let relationship_groups = root_package_json
            .as_ref()
            .map(|package_json| {
                RelationshipGroup::new(
                    "//",
                    package_json
                        .dependencies_with_kind()
                        .map(|(name, specifier, kind)| {
                            Relationship::new(
                                name,
                                kind,
                                RelationshipTarget::UnresolvedExternal {
                                    name: name.clone(),
                                    specifier: specifier.clone(),
                                },
                            )
                        })
                        .collect(),
                )
            })
            .into_iter()
            .collect();
        let relationship_knowledge = Arc::new(RelationshipKnowledge::build(
            &knowledge,
            relationship_groups,
        )?);
        let PackageGraphAssembly {
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
        } = assembler.finish();

        let package_manager = match &javascript {
            Some(javascript) => {
                let package_manager = javascript
                    .package_manager()
                    .await?
                    .with_resolved_nub_lockfile(repo_root);
                // Command resolution is synchronous; record the resolved
                // package manager on the toolchain so it does not re-run
                // discovery.
                Some(package_manager)
            }
            None => None,
        };

        debug_assert!(single, "expected single package graph");
        let mut native_task_observations = native_task_observations;
        if let Some(root_package_json) = &root_package_json {
            native_task_observations.push(crate::native_tasks::observation_from_package_json(
                "//",
                root_package_json,
            ));
        }
        let task_contract_observations = native_task_observations
            .iter()
            .map(|observation| (observation.scope.clone(), observation.task_contract.clone()))
            .collect::<Vec<_>>();
        let native_task_knowledge = Arc::new(
            crate::native_tasks::NativeTaskKnowledge::build(&knowledge, native_task_observations)
                .map_err(|error| Error::NativeTasks(error.to_string()))?,
        );

        let task_contract_knowledge = Arc::new({
            let root_engines = root_package_json
                .as_ref()
                .and_then(|package_json| package_json.engines())
                .map(|engines| {
                    engines
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            crate::task_contracts::TaskContractKnowledge::build_with_engines(
                task_contract_observations,
                root_engines,
            )
            .map_err(|error| Error::TaskContracts(error.to_string()))?
        });
        let change_knowledge = Arc::new(
            crate::change_knowledge::ChangeKnowledge::build(
                &knowledge,
                package_manager.as_ref(),
                Vec::new(),
            )
            .map_err(|error| Error::ChangeKnowledge(error.to_string()))?,
        );
        let prune_knowledge = Arc::new(crate::prune_knowledge::PruneKnowledge::default());

        Ok(PackageGraph {
            graph: workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            lockfile: lockfile.map(Arc::from),
            package_manager,
            knowledge,
            relationship_knowledge,
            external_declarations: std::sync::OnceLock::new(),
            relationship_projections: std::sync::OnceLock::new(),
            external_resolution: std::sync::Mutex::new(ExternalResolutionKnowledge::absent()),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
            root_internal_dependencies: std::sync::OnceLock::new(),
            native_task_knowledge,
            task_contract_knowledge,
            change_knowledge,
            prune_knowledge,
        })
    }
}

impl<'a, T: PackageDiscovery + Send + Sync> BuildState<'a, ResolvedWorkspaces, T> {
    #[tracing::instrument(skip(self))]
    fn connect_internal_dependencies(
        &mut self,
        package_manager: Option<&PackageManager>,
    ) -> Result<(), Error> {
        let knowledge = self
            .knowledge
            .as_deref()
            .ok_or(Error::MissingRepositoryKnowledge)?;
        let path_index = WorkspacePathIndex::from_knowledge(knowledge);
        // Compute once — for pnpm/Berry this reads a config file from disk.
        // Without hoisting, classifying each JavaScript descriptor would
        // redundantly read the same file. Cargo supplies classified internal
        // relationships, so its empty descriptors never reach this fallback.
        let link_workspace_packages =
            package_manager.is_some_and(|pm| pm.link_workspace_packages(self.repo_root));
        let catalogs = package_manager.and_then(|pm| pm.read_catalogs(self.repo_root));
        let identities: Vec<_> = knowledge
            .root_javascript_scope()
            .map(|_| "//")
            .into_iter()
            .chain(knowledge.scopes().map(|scope| scope.identity()))
            .collect();
        let mut native_relationships = std::mem::take(&mut self.native_relationships);
        let inputs: Vec<_> = identities
            .into_iter()
            .map(|identity| (identity, native_relationships.remove(identity)))
            .collect();
        debug_assert!(native_relationships.is_empty());
        // Release the map's retained capacity before parallel classification allocates
        // results.
        drop(native_relationships);
        // Classification is read-only and remains parallel across packages.
        // RelationshipKnowledge sorts the indexed results before retaining
        // the immutable generation.
        let groups = {
            use rayon::prelude::*;
            inputs
                .into_par_iter()
                .map(|(identity, native)| -> Result<_, Error> {
                    let relationships = match native {
                        Some(relationships) => relationships,
                        None => {
                            let name = package_name_from_identity(identity);
                            let entry =
                                self.assembler.package_jsons.get(&name).ok_or_else(|| {
                                    Error::MissingDescriptor {
                                        name: identity.to_string(),
                                    }
                                })?;
                            let definition_path = scope_definition_path(knowledge, &name)
                                .ok_or_else(|| Error::MissingDescriptor {
                                    name: identity.to_string(),
                                })?;
                            Relationships::classify(
                                self.repo_root,
                                definition_path,
                                &self.assembler.package_jsons,
                                link_workspace_packages,
                                entry.dependencies_with_kind(),
                                &path_index,
                                catalogs.as_ref(),
                            )
                        }
                    };
                    Ok(RelationshipGroup::new(identity, relationships))
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let relationship_knowledge = Arc::new(RelationshipKnowledge::build(knowledge, groups)?);
        self.assembler
            .project_relationships(&relationship_knowledge)?;
        self.relationship_knowledge = Some(relationship_knowledge);

        Ok(())
    }

    #[tracing::instrument(skip(self, package_manager))]
    async fn populate_lockfile(
        &mut self,
        package_manager: &PackageManager,
    ) -> Result<Box<dyn Lockfile>, Error> {
        match self.lockfile.take() {
            Some(lockfile) => Ok(lockfile),
            None => {
                let root_package_json = self
                    .root_package_json
                    .as_ref()
                    .expect("JavaScript package manager requires a root package.json");
                let lockfile = package_manager.read_lockfile(self.repo_root, root_package_json)?;
                Ok(lockfile)
            }
        }
    }

    #[tracing::instrument(skip(self, lockfile_future))]
    async fn resolve_lockfile(
        mut self,
        lockfile_future: Option<tokio::task::JoinHandle<Option<Box<dyn Lockfile>>>>,
    ) -> Result<BuildState<'a, ResolvedLockfile, T>, Error> {
        // Since we've already performed package discovery, this should just be
        // a cache hit. A pure Cargo workspace has no JavaScript toolchain and
        // therefore no package manager or lockfile.
        let package_manager = match &self.javascript {
            Some(javascript) => Some(
                javascript
                    .package_manager()
                    .await?
                    .with_resolved_nub_lockfile(self.repo_root),
            ),
            None => None,
        };
        turborepo_rayon_compat::block_in_place(|| {
            self.connect_internal_dependencies(package_manager.as_ref())
        })?;

        if let Some(handle) = lockfile_future
            && let Ok(Some(lockfile)) = handle.await
        {
            self.lockfile = Some(lockfile);
        }

        let lockfile = match package_manager.as_ref() {
            // No JavaScript package manager (pure Cargo): no JS lockfile to
            // parse. Cargo's own lockfile is handled by the Cargo toolchain.
            None => None,
            Some(package_manager) => match self.populate_lockfile(package_manager).await {
                Ok(lockfile) => Some(lockfile),
                Err(e) => {
                    let problematic_file_path =
                        extract_file_path_from_error(&e, package_manager, self.repo_root);

                    warn!(
                        "An issue occurred while attempting to parse {}. Turborepo will still \
                         function, but some features may not be available:\n {:?}",
                        problematic_file_path,
                        Report::new(e)
                    );
                    None
                }
            },
        };

        let Self {
            repo_root,
            single,
            assembler,
            knowledge,
            relationship_knowledge,
            native_external_resolutions,
            native_task_observations,
            native_change_observations,
            native_prune_domains,
            root_package_json,
            javascript,
            extra_contributors,
            ..
        } = self;
        Ok(BuildState {
            repo_root,
            single,
            assembler,
            knowledge,
            relationship_knowledge,
            // Native contributions were consumed before lockfile setup.
            native_relationships: HashMap::new(),
            native_external_resolutions,
            native_task_observations,
            native_change_observations,
            native_prune_domains,
            root_package_json,
            lockfile,
            package_manager,
            package_jsons: None,
            state: std::marker::PhantomData,
            javascript,
            extra_contributors,
        })
    }
}

fn scope_definition_path<'a>(
    knowledge: &'a RepositoryKnowledge,
    name: &PackageName,
) -> Option<&'a AnchoredSystemPath> {
    match name {
        PackageName::Root => knowledge
            .root_javascript_scope()
            .map(|scope| scope.definition_path()),
        PackageName::Other(name) => knowledge.scope(name).map(|scope| scope.definition_path()),
    }
}

fn build_failure(error: Error) -> discovery::Error {
    discovery::Error::Failed(Box::new(error))
}

impl<T: PackageDiscovery + Send + Sync> BuildState<'_, ResolvedLockfile, T> {
    fn all_external_dependencies(
        &self,
    ) -> Result<HashMap<String, BTreeMap<String, String>>, Error> {
        let knowledge = self
            .knowledge
            .as_deref()
            .ok_or(Error::MissingRepositoryKnowledge)?;
        let relationships = self
            .relationship_knowledge
            .as_deref()
            .ok_or(Error::MissingRelationshipKnowledge)?;
        Ok(javascript::external_dependencies(knowledge, relationships))
    }

    #[tracing::instrument(skip(self))]
    async fn build_inner(mut self) -> Result<PackageGraph, discovery::Error> {
        // External resolution is produced during repository construction and
        // retained as an immutable generation. Readiness belongs to build().
        let knowledge = self
            .knowledge
            .clone()
            .ok_or_else(|| build_failure(Error::MissingRepositoryKnowledge))?;
        let package_manager = self.package_manager.clone();
        let definition_source = package_manager
            .as_ref()
            .map(|package_manager| AnchoredSystemPathBuf::from_raw(package_manager.lockfile_name()))
            .transpose()
            .map_err(Error::from)
            .map_err(build_failure)?;
        let arc_lockfile: Option<Arc<dyn Lockfile>> = self.lockfile.take().map(Arc::from);
        let mut external_resolution = ExternalResolutionKnowledge::absent();
        let mut native_external_resolutions = std::mem::take(&mut self.native_external_resolutions);

        if let Some(definition_source) = definition_source {
            if let Some(lockfile) = arc_lockfile.clone() {
                match self.all_external_dependencies() {
                    Ok(external_dependencies) => {
                        let mut snapshot = turborepo_rayon_compat::block_in_place(|| {
                            javascript::resolve_dependencies(
                                &knowledge,
                                std::mem::take(&mut native_external_resolutions),
                                lockfile.as_ref(),
                                external_dependencies,
                                false,
                                definition_source,
                            )
                        })
                        .map_err(|message| build_failure(Error::ExternalResolution(message)))?;
                        if let Some(warning) = snapshot.warning.take() {
                            warn!("Unable to calculate transitive closures: {}", warning);
                        }
                        external_resolution =
                            ExternalResolutionKnowledge::complete(snapshot.generation);
                    }
                    Err(error) => {
                        warn!("Unable to calculate transitive closures: {}", error);
                        let snapshot = javascript::unavailable_resolution(
                            &knowledge,
                            std::mem::take(&mut native_external_resolutions),
                            definition_source,
                            "declarations-unavailable",
                            error.to_string(),
                            None,
                        )
                        .map_err(|message| build_failure(Error::ExternalResolution(message)))?;
                        external_resolution =
                            ExternalResolutionKnowledge::complete(snapshot.generation);
                    }
                }
            } else {
                let snapshot = javascript::unavailable_resolution(
                    &knowledge,
                    std::mem::take(&mut native_external_resolutions),
                    definition_source,
                    "lockfile-unavailable",
                    "JavaScript lockfile could not be read or parsed".to_string(),
                    None,
                )
                .map_err(|message| build_failure(Error::ExternalResolution(message)))?;
                external_resolution = ExternalResolutionKnowledge::complete(snapshot.generation);
            }
        } else if !native_external_resolutions.is_empty() {
            let generation =
                ExternalResolutionGeneration::build(&knowledge, native_external_resolutions)
                    .map_err(|error| build_failure(Error::ExternalResolution(error.to_string())))?;
            external_resolution = ExternalResolutionKnowledge::complete(Arc::new(generation));
        }
        let Self {
            assembler,
            knowledge,
            relationship_knowledge,
            native_task_observations,
            native_change_observations,
            native_prune_domains,
            root_package_json,
            ..
        } = self;
        let knowledge = knowledge.ok_or(discovery::Error::Failed(Box::new(
            Error::MissingRepositoryKnowledge,
        )))?;
        let relationship_knowledge = relationship_knowledge.ok_or(discovery::Error::Failed(
            Box::new(Error::MissingRelationshipKnowledge),
        ))?;
        let PackageGraphAssembly {
            workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
        } = assembler.finish();

        // Include root JavaScript scripts when a root package.json exists.
        let mut native_task_observations = native_task_observations;
        if let Some(root_package_json) = &root_package_json {
            native_task_observations.push(crate::native_tasks::observation_from_package_json(
                "//",
                root_package_json,
            ));
        }
        let task_contract_observations = native_task_observations
            .iter()
            .map(|observation| (observation.scope.clone(), observation.task_contract.clone()))
            .collect::<Vec<_>>();
        let native_task_knowledge = Arc::new(
            crate::native_tasks::NativeTaskKnowledge::build(&knowledge, native_task_observations)
                .map_err(|error| {
                discovery::Error::Failed(Box::new(Error::NativeTasks(error.to_string())))
            })?,
        );
        let task_contract_knowledge = Arc::new({
            let root_engines = root_package_json
                .as_ref()
                .and_then(|package_json| package_json.engines())
                .map(|engines| {
                    engines
                        .into_iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            crate::task_contracts::TaskContractKnowledge::build_with_engines(
                task_contract_observations,
                root_engines,
            )
            .map_err(|error| {
                discovery::Error::Failed(Box::new(Error::TaskContracts(error.to_string())))
            })?
        });
        let change_knowledge = Arc::new(
            crate::change_knowledge::ChangeKnowledge::build(
                &knowledge,
                package_manager.as_ref(),
                native_change_observations,
            )
            .map_err(|error| {
                discovery::Error::Failed(Box::new(Error::ChangeKnowledge(error.to_string())))
            })?,
        );
        let referenced_prune_domains =
            task_contract_knowledge
                .scopes()
                .filter_map(|(_, contract)| match contract.prune_package_mode() {
                    Some(crate::task_contracts::PrunePackageMode::NativeDomain(domain)) => {
                        Some(domain.clone())
                    }
                    _ => None,
                });
        let prune_knowledge = Arc::new(
            crate::prune_knowledge::PruneKnowledge::new(
                native_prune_domains,
                referenced_prune_domains,
            )
            .map_err(|error| {
                discovery::Error::Failed(Box::new(Error::PruneKnowledge(error.to_string())))
            })?,
        );

        Ok(PackageGraph {
            graph: workspace_graph,
            root_node_index,
            root_workspace_index,
            node_lookup,
            root_package_json,
            package_manager,
            lockfile: arc_lockfile,
            knowledge,
            relationship_knowledge,
            external_declarations: std::sync::OnceLock::new(),
            relationship_projections: std::sync::OnceLock::new(),
            external_resolution: std::sync::Mutex::new(external_resolution),
            external_dep_to_internal_dependents: std::sync::OnceLock::new(),
            root_internal_dependencies: std::sync::OnceLock::new(),
            native_task_knowledge,
            task_contract_knowledge,
            change_knowledge,
            prune_knowledge,
        })
    }
}

struct Relationships;

impl Relationships {
    fn classify<'a, I: IntoIterator<Item = (&'a String, &'a String, DependencyKind)>>(
        repo_root: &AbsoluteSystemPath,
        workspace_json_path: &AnchoredSystemPath,
        workspaces: &HashMap<PackageName, PackageJson>,
        link_workspace_packages: bool,
        dependencies: I,
        path_index: &WorkspacePathIndex<'_>,
        catalogs: Option<&PnpmCatalogs>,
    ) -> Vec<Relationship> {
        let resolved_workspace_json_path = repo_root.resolve(workspace_json_path);
        let workspace_dir = resolved_workspace_json_path
            .parent()
            .expect("package.json path should have parent");
        let dependencies = dependencies.into_iter();
        let mut relationships = Vec::with_capacity(dependencies.size_hint().0);
        let splitter = DependencySplitter::new(
            repo_root,
            workspace_dir,
            workspaces,
            link_workspace_packages,
            path_index,
            catalogs,
        );
        for (name, version, kind) in dependencies {
            let target = splitter.is_internal(name, version).map_or_else(
                || RelationshipTarget::UnresolvedExternal {
                    name: name.clone(),
                    specifier: version.clone(),
                },
                |workspace| RelationshipTarget::Internal(workspace.as_str().to_string()),
            );
            relationships.push(Relationship::new(name, kind, target));
        }
        relationships
    }
}

fn package_name_from_identity(identity: &str) -> PackageName {
    if identity == "//" {
        PackageName::Root
    } else {
        PackageName::Other(identity.to_string())
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use turborepo_errors::Spanned;

    use super::*;
    use crate::toolchain::{DiscoverPackagesFuture, DiscoveredPackages, WorkspaceRoot};

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            Ok(crate::discovery::DiscoveryResponse {
                package_manager: crate::package_manager::PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    #[derive(Clone)]
    struct ManagerDiscovery(PackageManager);

    impl PackageDiscovery for ManagerDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            Ok(crate::discovery::DiscoveryResponse {
                package_manager: self.0.clone(),
                workspaces: Vec::new(),
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    struct RootObservingContributor {
        id: ToolchainId,
        roots: Vec<WorkspaceRoot>,
    }

    impl RepositoryContributor for RootObservingContributor {
        fn id(&self) -> ToolchainId {
            self.id.clone()
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            Box::pin(async move { Ok(DiscoveredPackages::new(Vec::new(), self.roots.clone())) })
        }
    }

    struct PackageWithoutRootContributor {
        root: AbsoluteSystemPathBuf,
    }

    impl RepositoryContributor for PackageWithoutRootContributor {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("missing-root")
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            Box::pin(async move {
                Ok(DiscoveredPackages::new(
                    vec![DiscoveredPackage::package(
                        Some("orphan".to_string()),
                        PackageJson::default(),
                        self.root.join_components(&["orphan", "manifest"]),
                    )],
                    Vec::new(),
                ))
            })
        }
    }

    struct PackageContributor {
        id: ToolchainId,
        root: AbsoluteSystemPathBuf,
        packages: Vec<DiscoveredPackage>,
    }

    impl RepositoryContributor for PackageContributor {
        fn id(&self) -> ToolchainId {
            self.id.clone()
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            Box::pin(async move {
                Ok(DiscoveredPackages::new(
                    self.packages.clone(),
                    vec![WorkspaceRoot::new("custom", self.root.clone())],
                ))
            })
        }
    }

    fn custom_package(
        root: &AbsoluteSystemPath,
        name: &str,
        descriptor: PackageJson,
    ) -> DiscoveredPackage {
        let directory = name.replace(['/', '@'], "_");
        DiscoveredPackage::package(
            Some(name.to_string()),
            descriptor,
            root.join_components(&["custom-packages", &directory, "custom-manifest"]),
        )
    }

    #[tokio::test]
    async fn custom_toolchain_descriptor_fallback_and_native_empty_are_distinct() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let dependency_descriptor = || PackageJson {
            dependencies: Some(
                [("custom-lib".to_string(), "workspace:*".to_string())]
                    .into_iter()
                    .collect(),
            ),
            ..Default::default()
        };
        let legacy = custom_package(&root, "legacy-app", dependency_descriptor());
        let native = custom_package(&root, "native-app", dependency_descriptor())
            .with_native_relationships(Vec::new());
        let library = custom_package(&root, "custom-lib", PackageJson::default());
        let toolchain = PackageContributor {
            id: ToolchainId::new("custom-relationships"),
            root: root.clone(),
            packages: vec![legacy, native, library],
        };

        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_contributor(Arc::new(toolchain))
            .build()
            .await
            .unwrap();

        let library = PackageNode::Workspace(PackageName::from("custom-lib"));
        assert!(
            graph
                .immediate_dependencies(&PackageNode::Workspace(PackageName::from("legacy-app")))
                .unwrap()
                .contains(&library),
            "None must preserve descriptor classification for custom toolchains"
        );
        let native_dependencies = graph
            .immediate_dependencies(&PackageNode::Workspace(PackageName::from("native-app")))
            .unwrap();
        assert_eq!(native_dependencies.len(), 1);
        assert!(native_dependencies.contains(&PackageNode::Root));
        assert!(
            graph
                .relationship_knowledge
                .groups()
                .iter()
                .find(|group| group.source() == "native-app")
                .unwrap()
                .relationships()
                .is_empty()
        );
        let custom_context = graph
            .package_task_context(&PackageName::from("legacy-app"))
            .unwrap();
        let custom_contract = custom_context.task_contract();
        assert_eq!(custom_contract.toolchain(), None);
        assert_eq!(
            custom_contract.command_map_argv(&[("javascript".into(), vec!["node".into()])]),
            None
        );
    }

    #[tokio::test]
    async fn reserved_root_identity_is_rejected_in_pure_native_and_mixed_repositories() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        for mixed in [false, true] {
            let toolchain = PackageContributor {
                id: ToolchainId::new("reserved-root"),
                root: root.clone(),
                packages: vec![
                    custom_package(&root, "//", PackageJson::default())
                        .with_native_relationships(Vec::new()),
                ],
            };
            let result = PackageGraphBuilder::new_optional(&root, mixed.then(PackageJson::default))
                .with_package_discovery(MockDiscovery)
                .with_contributor(Arc::new(toolchain))
                .build()
                .await;

            assert!(matches!(result, Err(Error::ReservedRootIdentity { .. })));
        }
    }

    #[tokio::test]
    async fn javascript_packages_have_canonical_identities_paths_and_root_scope() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        for root_name in [Some("named-root"), None] {
            let package_jsons = HashMap::from([
                (
                    root.join_components(&["apps", "app", "package.json"]),
                    PackageJson {
                        name: Some(
                            Spanned::new("app".to_string())
                                .with_range(9..14)
                                .with_text(r#"{"name": "app"}"#)
                                .with_path("apps/app/package.json".into()),
                        ),
                        ..Default::default()
                    },
                ),
                (
                    root.join_components(&["packages", "group", "nested", "package.json"]),
                    PackageJson {
                        name: Some(Spanned::new("@scope/nested".into())),
                        ..Default::default()
                    },
                ),
                (
                    root.join_components(&["packages", "unnamed", "package.json"]),
                    PackageJson::default(),
                ),
            ]);
            let root_package_json = PackageJson {
                name: root_name.map(|name| Spanned::new(name.to_string())),
                ..Default::default()
            };

            let graph = PackageGraphBuilder::new(&root, root_package_json)
                .with_package_discovery(MockDiscovery)
                .with_package_jsons(Some(package_jsons))
                .build()
                .await
                .unwrap();

            let knowledge = graph.repository_knowledge();
            assert_eq!(knowledge.repository_root(), root.as_ref());
            let root_scope = knowledge
                .root_javascript_scope()
                .expect("a root package.json creates a JavaScript execution scope");
            assert_eq!(root_scope.user_facing_name(), root_name);
            assert_eq!(
                graph
                    .package_dir(&PackageName::Root)
                    .expect("root graph scope has a directory")
                    .to_unix()
                    .as_str(),
                ""
            );
            assert_eq!(
                root_scope.definition_path().to_unix().as_str(),
                "package.json"
            );
            assert_eq!(knowledge.packages().count(), 2);
            assert!(knowledge.scope("//").is_none());
            assert!(knowledge.scope("unnamed").is_none());
            assert_eq!(knowledge.aggregate_scopes().count(), 0);
            assert_ne!(
                PackageNode::Root,
                PackageNode::Workspace(PackageName::Root),
                "the graph sentinel and root execution scope node are distinct"
            );
            assert_eq!(graph.root_javascript_scope_name(), Some(root_name));
            let sentinel = graph
                .node_view(&PackageNode::Root)
                .expect("the graph sentinel is always present");
            assert_eq!(
                sentinel.kind(),
                super::super::PackageGraphNodeKind::GraphSentinel
            );
            assert_eq!(sentinel.directory(), None);
            assert_eq!(sentinel.definition_path(), None);
            assert_eq!(sentinel.toolchain(), None);

            let root_view = graph
                .node_view(&PackageNode::Workspace(PackageName::Root))
                .expect("a package.json contributes the root JavaScript scope");
            assert_eq!(
                root_view.kind(),
                super::super::PackageGraphNodeKind::RootJavaScript
            );
            assert_eq!(
                root_view.directory(),
                Some(knowledge.repository_directory())
            );
            assert_eq!(
                root_view
                    .definition_path()
                    .map(|path| path.to_unix().to_string()),
                Some("package.json".to_string())
            );
            assert_eq!(root_view.toolchain(), Some(&ToolchainId::JAVASCRIPT));

            let app_view = graph
                .node_view(&PackageNode::Workspace(PackageName::from("app")))
                .expect("the real package has an authoritative view");
            assert_eq!(app_view.kind(), super::super::PackageGraphNodeKind::Package);
            assert_eq!(
                app_view.directory().map(|path| path.to_unix().to_string()),
                Some("apps/app".to_string())
            );
            assert_eq!(
                app_view
                    .definition_path()
                    .map(|path| path.to_unix().to_string()),
                Some("apps/app/package.json".to_string())
            );
            assert_eq!(app_view.toolchain(), Some(&ToolchainId::JAVASCRIPT));
            let app_contract = graph
                .package_task_context(&PackageName::from("app"))
                .unwrap()
                .task_contract()
                .clone();
            assert_eq!(app_contract.toolchain(), Some(&ToolchainId::JAVASCRIPT));
            assert_eq!(
                app_contract.command_map_argv(&[("javascript".into(), vec!["node".into()])]),
                Some(vec!["node".into()])
            );
            let app_name_source = app_view
                .name_source()
                .expect("the authored JavaScript name retains diagnostic provenance");
            assert_eq!(app_name_source.range, Some(9..14));
            assert_eq!(app_name_source.text.as_deref(), Some(r#"{"name": "app"}"#));
            assert_eq!(
                app_name_source.path.as_deref(),
                Some("apps/app/package.json")
            );
            assert_eq!(graph.node_views().count(), 4);

            let mut packages = graph
                .package_task_contexts()
                .map(|context| {
                    let definition = graph
                        .package_definition_path(context.package())
                        .map(|path| path.to_unix().to_string());
                    (
                        context.package().as_str().to_string(),
                        context.directory().to_unix().to_string(),
                        definition,
                    )
                })
                .collect::<Vec<_>>();
            packages.sort();

            assert_eq!(
                packages,
                vec![
                    (
                        "//".to_string(),
                        "".to_string(),
                        Some("package.json".to_string()),
                    ),
                    (
                        "@scope/nested".to_string(),
                        "packages/group/nested".to_string(),
                        Some("packages/group/nested/package.json".to_string()),
                    ),
                    (
                        "app".to_string(),
                        "apps/app".to_string(),
                        Some("apps/app/package.json".to_string()),
                    ),
                ]
            );
        }
    }

    #[tokio::test]
    async fn single_package_mode_exposes_only_the_canonical_root_scope() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("user-facing-root-name".into())),
                dependencies: Some(
                    [("prod".to_string(), "1".to_string())]
                        .into_iter()
                        .collect(),
                ),
                optional_dependencies: Some(
                    [("optional".to_string(), "1".to_string())]
                        .into_iter()
                        .collect(),
                ),
                dev_dependencies: Some(
                    [("dev".to_string(), "1".to_string())].into_iter().collect(),
                ),
                peer_dependencies: Some(
                    [("peer".to_string(), "1".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        )
        .with_single_package_mode(true)
        .with_package_discovery(MockDiscovery)
        .build()
        .await
        .unwrap();

        let retained_generation = graph.knowledge.clone();
        assert!(Arc::ptr_eq(&retained_generation, &graph.knowledge));
        assert_eq!(
            retained_generation
                .root_javascript_scope()
                .expect("single-package mode retains the root execution scope")
                .user_facing_name(),
            Some("user-facing-root-name")
        );
        assert_eq!(retained_generation.packages().count(), 0);
        assert_eq!(
            graph.external_resolution_global_file_fallback(),
            Some(vec![root.join_component("package.json")]),
            "single-package mode must preserve package.json as a global hash input"
        );

        let contexts = graph.package_task_contexts().collect::<Vec<_>>();
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].package(), &PackageName::Root);
        assert_eq!(
            contexts[0]
                .external_declarations()
                .iter()
                .map(|declaration| (declaration.declaration_name(), declaration.kind()))
                .collect::<Vec<_>>(),
            vec![
                ("prod", DependencyKind::Production),
                ("optional", DependencyKind::Optional),
                ("dev", DependencyKind::Development),
                ("peer", DependencyKind::Peer { optional: false }),
            ]
        );
        assert_eq!(
            graph
                .package_dir(&PackageName::Root)
                .unwrap()
                .to_unix()
                .to_string(),
            ""
        );
        assert_eq!(
            graph.package_definition_path(&PackageName::Root),
            Some(AnchoredSystemPath::new("package.json").unwrap())
        );
    }

    // Regression test: relationship projection must produce correct graph
    // edges and external declaration views regardless of iteration order or
    // parallelism. This captures edges and declaration projections so any
    // refactor of the collection phase (e.g. rayon parallelization) is safe.
    #[tokio::test]
    async fn test_connect_internal_dependencies_produces_correct_edges() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let mut package_jsons = HashMap::new();
        // "web" depends on "ui" (workspace:*) and "react" (external)
        package_jsons.insert(
            root.join_components(&["apps", "web", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("web".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [
                        ("ui".to_string(), "workspace:*".to_string()),
                        ("react".to_string(), "^18.0.0".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
        );
        // "api" depends on "utils" (workspace:*) and "express" (external)
        package_jsons.insert(
            root.join_components(&["apps", "api", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("api".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [
                        ("utils".to_string(), "workspace:*".to_string()),
                        ("express".to_string(), "^4.0.0".to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
        );
        // "ui" has no workspace deps, only "csstype" (external)
        package_jsons.insert(
            root.join_components(&["packages", "ui", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("ui".into())),
                version: Some("1.0.0".to_string()),
                dependencies: Some(
                    [("csstype".to_string(), "^3.0.0".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        );
        // "utils" has no deps at all
        package_jsons.insert(
            root.join_components(&["packages", "utils", "package.json"]),
            PackageJson {
                name: Some(Spanned::new("utils".into())),
                version: Some("1.0.0".to_string()),
                ..Default::default()
            },
        );

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(package_jsons))
        .build()
        .await
        .unwrap();

        let web_relationships = graph
            .relationship_knowledge
            .groups()
            .iter()
            .find(|group| group.source() == "web")
            .unwrap()
            .relationships();
        assert!(web_relationships.iter().any(|relationship| {
            relationship.kind() == DependencyKind::Production
                && relationship.target() == &RelationshipTarget::Internal("ui".to_string())
        }));
        assert!(web_relationships.iter().any(|relationship| {
            relationship.target()
                == &RelationshipTarget::UnresolvedExternal {
                    name: "react".to_string(),
                    specifier: "^18.0.0".to_string(),
                }
        }));

        // Verify internal dependency edges via the package graph API
        let web_name = PackageName::from("web");
        let api_name = PackageName::from("api");
        let ui_name = PackageName::from("ui");
        let utils_name = PackageName::from("utils");

        // web -> ui (internal)
        let web_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(web_name.clone()))
            .unwrap();
        assert!(
            web_deps.contains(&PackageNode::Workspace(ui_name.clone())),
            "web should depend on ui, got: {:?}",
            web_deps
        );
        let mut graph_ordering: Vec<_> = web_deps
            .iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(name) => Some(name.clone()),
                PackageNode::Root => None,
            })
            .collect();
        graph_ordering.sort();
        let projected_ordering: Vec<_> = graph
            .ordering_relationships()
            .direct_dependencies(&web_name)
            .expect("web is authoritative")
            .cloned()
            .collect();
        assert_eq!(projected_ordering, graph_ordering);

        let mut graph_dependencies: Vec<_> = graph
            .dependencies(&PackageNode::Workspace(web_name.clone()))
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(name) if name != &web_name => Some(name.clone()),
                PackageNode::Root | PackageNode::Workspace(_) => None,
            })
            .collect();
        graph_dependencies.sort();
        assert_eq!(
            graph
                .filtering_relationships()
                .transitive_dependencies(&web_name),
            Ok(graph_dependencies.clone())
        );
        assert_eq!(
            graph.hash_relationships().dependency_inputs(&web_name),
            Ok(graph_dependencies)
        );

        let mut graph_dependents: Vec<_> = graph
            .ancestors(&PackageNode::Workspace(ui_name.clone()))
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(name) if name != &ui_name => Some(name.clone()),
                PackageNode::Root | PackageNode::Workspace(_) => None,
            })
            .collect();
        graph_dependents.sort();
        assert_eq!(
            graph
                .filtering_relationships()
                .transitive_dependents(&ui_name),
            Ok(graph_dependents.clone())
        );
        graph_dependents.push(ui_name.clone());
        graph_dependents.sort();
        assert_eq!(
            graph
                .affected_relationships()
                .affected_by(std::slice::from_ref(&ui_name)),
            Ok(graph_dependents)
        );

        // api -> utils (internal)
        let api_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(api_name.clone()))
            .unwrap();
        assert!(
            api_deps.contains(&PackageNode::Workspace(utils_name.clone())),
            "api should depend on utils, got: {:?}",
            api_deps
        );

        // ui has no internal deps -> should connect to root
        let ui_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(ui_name.clone()))
            .unwrap();
        assert!(
            ui_deps.contains(&PackageNode::Root),
            "ui should depend on root (no internal deps), got: {:?}",
            ui_deps
        );

        // utils has no internal deps -> should connect to root
        let utils_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(utils_name.clone()))
            .unwrap();
        assert!(
            utils_deps.contains(&PackageNode::Root),
            "utils should depend on root (no internal deps), got: {:?}",
            utils_deps
        );

        // Verify external deps are recorded correctly
        let web_ext: std::collections::BTreeMap<_, _> = graph
            .external_declarations(&web_name)
            .iter()
            .map(|declaration| {
                (
                    declaration.package_name().to_string(),
                    declaration.specifier().to_string(),
                )
            })
            .collect();
        assert_eq!(web_ext.get("react").map(|v| v.as_str()), Some("^18.0.0"));
        assert!(
            !web_ext.contains_key("ui"),
            "ui should be internal, not external"
        );

        let api_ext: std::collections::BTreeMap<_, _> = graph
            .external_declarations(&api_name)
            .iter()
            .map(|declaration| {
                (
                    declaration.package_name().to_string(),
                    declaration.specifier().to_string(),
                )
            })
            .collect();
        assert_eq!(api_ext.get("express").map(|v| v.as_str()), Some("^4.0.0"));
        assert!(
            !api_ext.contains_key("utils"),
            "utils should be internal, not external"
        );

        let ui_ext: std::collections::BTreeMap<_, _> = graph
            .external_declarations(&ui_name)
            .iter()
            .map(|declaration| {
                (
                    declaration.package_name().to_string(),
                    declaration.specifier().to_string(),
                )
            })
            .collect();
        assert_eq!(ui_ext.get("csstype").map(|v| v.as_str()), Some("^3.0.0"));

        let utils_ext: std::collections::BTreeMap<_, _> = graph
            .external_declarations(&utils_name)
            .iter()
            .map(|declaration| {
                (
                    declaration.package_name().to_string(),
                    declaration.specifier().to_string(),
                )
            })
            .collect();
        assert!(
            utils_ext.is_empty(),
            "utils should have no external deps, got: {:?}",
            utils_ext
        );
    }

    #[tokio::test]
    async fn test_dev_dependency_edge_kind() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["apps", "web", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("web".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("lib".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    dev_dependencies: Some(
                        [("tooling".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "lib", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("lib".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "tooling", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("tooling".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let web = PackageNode::Workspace(PackageName::from("web"));
        let lib = PackageNode::Workspace(PackageName::from("lib"));
        let tooling = PackageNode::Workspace(PackageName::from("tooling"));

        assert_eq!(
            graph.dependency_kind(&web, &lib),
            Some(DependencyKind::Production)
        );
        assert_eq!(
            graph.dependency_kind(&web, &tooling),
            Some(DependencyKind::Development)
        );

        let web_closure = graph.production_transitive_closure([&web]);
        assert!(web_closure.contains(&web));
        assert!(web_closure.contains(&lib));
        assert!(!web_closure.contains(&tooling));
    }

    #[tokio::test]
    async fn test_duplicate_dependency_prefers_production_kind() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["apps", "web", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("web".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("shared".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    dev_dependencies: Some(
                        [("shared".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    peer_dependencies: Some(
                        [("shared".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "shared", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("shared".into())),
                    version: Some("1.0.0".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let web = PackageNode::Workspace(PackageName::from("web"));
        let shared = PackageNode::Workspace(PackageName::from("shared"));

        assert_eq!(
            graph.dependency_kind(&web, &shared),
            Some(DependencyKind::Production)
        );
        let retained_kinds: Vec<_> = graph
            .relationship_knowledge
            .groups()
            .iter()
            .find(|group| group.source() == "web")
            .unwrap()
            .relationships()
            .iter()
            .filter(|relationship| relationship.declaration_name() == "shared")
            .map(Relationship::kind)
            .collect();
        assert_eq!(
            retained_kinds,
            [
                DependencyKind::Production,
                DependencyKind::Development,
                DependencyKind::Peer { optional: false },
            ]
        );
    }

    #[tokio::test]
    async fn test_peer_workspace_dep_does_not_override_concrete_external_dep() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("b".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("b".into())),
                    version: Some("1.0.0".to_string()),
                    dev_dependencies: Some(
                        [("buffer".to_string(), "npm:buffer@6.0.3".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    peer_dependencies: Some(
                        [("buffer".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "buffer", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("buffer".into())),
                    version: Some("6.0.3".to_string()),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let b = PackageName::from("b");
        let buffer = PackageName::from("buffer");
        let b_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(b.clone()))
            .unwrap();
        assert!(
            !b_deps.contains(&PackageNode::Workspace(buffer)),
            "peer workspace specifier should not create an internal edge when a concrete external \
             dependency exists, got: {:?}",
            b_deps
        );

        let b_external = {
            let decls: std::collections::BTreeMap<_, _> = graph
                .external_declarations(&b)
                .iter()
                .map(|d| (d.package_name().to_string(), d.specifier().to_string()))
                .collect();
            decls
        };
        assert_eq!(
            b_external.get("buffer").map(|v| v.as_str()),
            Some("npm:buffer@6.0.3")
        );
    }

    #[tokio::test]
    async fn test_pure_peer_workspace_dep_does_not_create_edge() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("b".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("b".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [("a".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let b = PackageName::from("b");

        let a_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(a.clone()))
            .unwrap();
        assert!(
            a_deps.contains(&PackageNode::Workspace(b.clone())),
            "a should depend on b, got: {:?}",
            a_deps
        );

        let b_deps = graph
            .immediate_dependencies(&PackageNode::Workspace(b.clone()))
            .unwrap();
        assert!(
            !b_deps.contains(&PackageNode::Workspace(a)),
            "pure peer workspace specifier should not create an internal edge, got: {:?}",
            b_deps
        );

        assert!(
            graph.find_cycles().is_empty(),
            "package graph should be acyclic once the pure peer edge is dropped"
        );
    }

    #[tokio::test]
    async fn test_external_peer_dep_is_retained_as_peer_declaration() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [("react".to_string(), "*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let declaration = graph
            .external_declarations(&a)
            .iter()
            .find(|declaration| declaration.package_name() == "react")
            .expect("external peer should remain available to declaration consumers");
        assert_eq!(declaration.kind(), DependencyKind::Peer { optional: false });
        let resolution_inputs =
            javascript::external_dependencies(&graph.knowledge, &graph.relationship_knowledge);
        assert!(
            resolution_inputs
                .values()
                .all(|dependencies| !dependencies.contains_key("react")),
            "external peer dependency must not reach JavaScript resolution inputs, got: \
             {resolution_inputs:?}"
        );
    }

    #[tokio::test]
    async fn test_external_peers_preserve_optional_metadata() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson::from_value(serde_json::json!({
                    "name": "a",
                    "version": "1.0.0",
                    "peerDependencies": {
                        "react": "*",
                        "lodash": "*"
                    },
                    "peerDependenciesMeta": {
                        "react": { "optional": true }
                    }
                }))
                .unwrap(),
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let a = PackageName::from("a");
        let peers: std::collections::BTreeMap<_, _> = graph
            .external_declarations(&a)
            .iter()
            .map(|declaration| (declaration.package_name(), declaration.kind()))
            .collect();
        assert_eq!(
            peers.get("react"),
            Some(&DependencyKind::Peer { optional: true })
        );
        assert_eq!(
            peers.get("lodash"),
            Some(&DependencyKind::Peer { optional: false })
        );
        let resolution_inputs =
            javascript::external_dependencies(&graph.knowledge, &graph.relationship_knowledge);
        assert!(
            resolution_inputs.values().all(|dependencies| {
                !dependencies.contains_key("react") && !dependencies.contains_key("lodash")
            }),
            "peer dependencies must not reach JavaScript resolution inputs, got: \
             {resolution_inputs:?}"
        );
    }

    #[tokio::test]
    async fn test_peer_dependencies_do_not_create_internal_edges() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();

        let graph = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_single_package_mode(false)
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut package_jsons = HashMap::new();
            package_jsons.insert(
                root.join_components(&["packages", "app", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("app".into())),
                    version: Some("1.0.0".to_string()),
                    dependencies: Some(
                        [("lib".to_string(), "workspace:*".to_string())]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons.insert(
                root.join_components(&["packages", "lib", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("lib".into())),
                    version: Some("1.0.0".to_string()),
                    peer_dependencies: Some(
                        [
                            ("app".to_string(), "workspace:*".to_string()),
                            ("react".to_string(), "*".to_string()),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                    ..Default::default()
                },
            );
            package_jsons
        }))
        .build()
        .await
        .unwrap();

        let app = PackageNode::Workspace(PackageName::from("app"));
        let lib_name = PackageName::from("lib");
        let lib = PackageNode::Workspace(lib_name.clone());

        let lib_closure = graph.transitive_closure([&lib]);
        assert!(
            !lib_closure.contains(&app),
            "package graph closure for lib should exclude pure-peer workspace app, got: \
             {lib_closure:?}"
        );
        assert!(
            graph.transitive_closure([&app]).contains(&lib),
            "prune closure for app should include its regular dependency lib"
        );

        let react = graph
            .external_declarations(&lib_name)
            .iter()
            .find(|declaration| declaration.package_name() == "react")
            .expect("external peer should remain available to declaration consumers");
        assert_eq!(react.kind(), DependencyKind::Peer { optional: false });
        let resolution_inputs =
            javascript::external_dependencies(&graph.knowledge, &graph.relationship_knowledge);
        assert!(
            resolution_inputs
                .values()
                .all(|dependencies| !dependencies.contains_key("react")),
            "external peer must not reach JavaScript resolution inputs, got: {resolution_inputs:?}"
        );
    }

    #[tokio::test]
    async fn test_duplicate_package_names() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let builder = PackageGraphBuilder::new(
            &root,
            PackageJson {
                name: Some(Spanned::new("root".into())),
                ..Default::default()
            },
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_components(&["packages", "a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("foo".into())),
                    ..Default::default()
                },
            );
            map.insert(
                root.join_components(&["packages", "b", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("foo".into())),
                    ..Default::default()
                },
            );
            map
        }));
        let error = builder.build().await.unwrap_err();
        let Error::DuplicateWorkspace {
            name,
            path,
            existing_path,
        } = error
        else {
            panic!("expected duplicate workspace error, got {error:?}");
        };
        let mut paths = [path.replace('\\', "/"), existing_path.replace('\\', "/")];
        paths.sort();

        assert_eq!(name, "foo");
        assert_eq!(
            paths,
            ["packages/a/package.json", "packages/b/package.json"]
        );
    }

    #[tokio::test]
    async fn package_definition_outside_repository_is_a_typed_error() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let outside = AbsoluteSystemPathBuf::new(if cfg!(windows) {
            r"C:\outside\package.json"
        } else {
            "/outside/package.json"
        })
        .unwrap();
        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::from([(
                outside.clone(),
                PackageJson {
                    name: Some(Spanned::new("escaped".into())),
                    ..Default::default()
                },
            )])))
            .build()
            .await;

        assert!(matches!(
            result,
            Err(Error::DefinitionOutsideRepository {
                path,
                repository_root,
            }) if path == outside && repository_root == root
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn package_definition_escaping_through_symlink_is_a_typed_error() {
        let tempdir = tempfile::tempdir().unwrap();
        let repository = tempdir.path().join("repository");
        let outside = tempdir.path().join("outside");
        std::fs::create_dir_all(repository.join("packages")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("package.json"), "{}").unwrap();
        std::os::unix::fs::symlink(&outside, repository.join("packages/escaped")).unwrap();

        let root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(&repository)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();
        let definition = root.join_components(&["packages", "escaped", "package.json"]);
        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::from([(
                definition.clone(),
                PackageJson {
                    name: Some(Spanned::new("escaped".into())),
                    ..Default::default()
                },
            )])))
            .build()
            .await;

        assert!(matches!(
            result,
            Err(Error::DefinitionOutsideRepository {
                path,
                repository_root,
            }) if path == definition && repository_root == root
        ));
    }

    #[tokio::test]
    async fn single_package_reports_only_javascript_root_without_running_extra_contributors() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_manager(PackageManager::Npm)
            .with_single_package_mode(true)
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("unused-extra"),
                roots: vec![WorkspaceRoot::new(
                    "unused-extra",
                    root.join_component("other"),
                )],
            }))
            .build()
            .await
            .unwrap();
        let roots = graph
            .repository_knowledge()
            .workspace_roots()
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].kind(), "npm");
    }

    #[tokio::test]
    async fn contributor_ids_are_unique() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let id = ToolchainId::new("duplicate");
        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: id.clone(),
                roots: Vec::new(),
            }))
            .with_contributor(Arc::new(RootObservingContributor {
                id: id.clone(),
                roots: Vec::new(),
            }))
            .build()
            .await;

        assert!(
            matches!(result, Err(Error::DuplicateContributor { id: duplicate }) if duplicate == id)
        );

        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::JAVASCRIPT,
                roots: Vec::new(),
            }))
            .build()
            .await;
        assert!(matches!(
            result,
            Err(Error::DuplicateContributor { id }) if id == ToolchainId::JAVASCRIPT
        ));
    }

    #[tokio::test]
    async fn contributor_cannot_contribute_multiple_workspace_root_kinds() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let first = root.join_component("first");
        let second = root.join_component("second");

        let duplicate = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-one"),
                roots: vec![
                    WorkspaceRoot::new("npm", first.clone()),
                    WorkspaceRoot::new("pnpm", second.clone()),
                ],
            }))
            .build()
            .await;
        assert!(matches!(
            duplicate,
            Err(Error::MultipleWorkspaceRoots {
                ref toolchain,
                ref accepted_kind,
                ref conflicting_kind,
                ..
            }) if toolchain == &ToolchainId::new("future-one")
                && accepted_kind == "npm"
                && conflicting_kind == "pnpm"
        ));

        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-two"),
                roots: vec![WorkspaceRoot::new("future-a", first)],
            }))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-three"),
                roots: vec![WorkspaceRoot::new("future-b", second)],
            }))
            .build()
            .await
            .unwrap();
        assert_eq!(graph.repository_knowledge().workspace_roots().count(), 3);
    }

    #[tokio::test]
    async fn same_physical_root_retains_each_core_bound_producer() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let first = ToolchainId::new("future-first");
        let second = ToolchainId::new("future-second");
        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: first.clone(),
                roots: vec![WorkspaceRoot::new("shared", root.clone())],
            }))
            .with_contributor(Arc::new(RootObservingContributor {
                id: second.clone(),
                roots: vec![WorkspaceRoot::new("shared", root.clone())],
            }))
            .build()
            .await
            .unwrap();
        let owners = graph
            .repository_knowledge()
            .workspace_roots()
            .filter(|root| root.kind() == "shared")
            .map(|root| root.toolchain().clone())
            .collect::<HashSet<_>>();

        assert_eq!(owners, HashSet::from([first, second]));
    }

    #[tokio::test]
    async fn contributed_packages_require_a_workspace_root() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(PackageWithoutRootContributor {
                root: root.clone(),
            }))
            .build()
            .await;
        assert!(matches!(
            result,
            Err(Error::MissingWorkspaceRoot { ref toolchain })
                if toolchain == &ToolchainId::new("missing-root")
        ));

        // A root returned by another registry entry cannot claim ownership for
        // the package producer, even when its kind and path would otherwise be
        // accepted.
        let cross_producer = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("spoof-attempt"),
                roots: vec![WorkspaceRoot::new("claimed", root.clone())],
            }))
            .with_contributor(Arc::new(PackageWithoutRootContributor {
                root: root.clone(),
            }))
            .build()
            .await;
        assert!(matches!(
            cross_producer,
            Err(Error::MissingWorkspaceRoot { ref toolchain })
                if toolchain == &ToolchainId::new("missing-root")
        ));

        PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("empty-no-op"),
                roots: Vec::new(),
            }))
            .build()
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn discovery_manager_must_match_the_authoritative_command_family() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let mismatch = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_manager(PackageManager::Npm)
            .with_package_discovery(ManagerDiscovery(PackageManager::Pnpm9))
            .build()
            .await;
        assert!(matches!(
            mismatch,
            Err(Error::Discovery(crate::discovery::Error::InvalidResponse(
                _
            )))
        ));

        for (authoritative, discovered, family) in [
            (PackageManager::Pnpm9, PackageManager::Pnpm6, "pnpm"),
            (PackageManager::Berry, PackageManager::Yarn, "yarn"),
        ] {
            let graph = PackageGraphBuilder::new(&root, PackageJson::default())
                .with_package_manager(authoritative.clone())
                .with_package_discovery(ManagerDiscovery(discovered))
                .build()
                .await
                .unwrap();
            assert_eq!(graph.package_manager(), Some(&authoritative));
            assert!(
                graph
                    .repository_knowledge()
                    .workspace_roots()
                    .any(|root| root.kind() == family)
            );
        }
    }

    #[tokio::test]
    async fn multiple_contributed_cargo_roots_use_generic_core_validation() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-cargo-adapter"),
                roots: vec![
                    WorkspaceRoot::new("cargo", root.join_component("first")),
                    WorkspaceRoot::new("cargo", root.join_component("second")),
                ],
            }))
            .build()
            .await;
        assert!(matches!(
            result,
            Err(Error::MultipleWorkspaceRoots { ref toolchain, .. })
                if toolchain == &ToolchainId::new("future-cargo-adapter")
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_root_symlink_aliases_deduplicate_by_physical_path() {
        let tempdir = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
        let physical = root.join_component("physical");
        std::fs::create_dir_all(physical.as_std_path()).unwrap();
        let alias = root.join_component("alias");
        std::os::unix::fs::symlink(physical.as_std_path(), alias.as_std_path()).unwrap();

        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-symlink"),
                roots: vec![
                    WorkspaceRoot::new("future-build", physical),
                    WorkspaceRoot::new("future-build", alias),
                ],
            }))
            .build()
            .await
            .unwrap();
        assert_eq!(
            graph
                .repository_knowledge()
                .workspace_roots()
                .filter(|root| root.kind() == "future-build")
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_root_symlink_escape_is_rejected() {
        let repository = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(repository.path()).unwrap();
        let outside_root = AbsoluteSystemPathBuf::try_from(outside.path()).unwrap();
        let alias = root.join_component("escaped");
        std::os::unix::fs::symlink(outside_root.as_std_path(), alias.as_std_path()).unwrap();
        let unresolved_root = alias.join_component("not-created");

        let result = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::new()))
            .with_contributor(Arc::new(RootObservingContributor {
                id: ToolchainId::new("future-symlink-escape"),
                roots: vec![WorkspaceRoot::new("future-build", unresolved_root.clone())],
            }))
            .build()
            .await;
        assert!(matches!(
            result,
            Err(Error::WorkspaceRootOutsideRepository {
                ref kind,
                ref path,
                ..
            }) if kind == "future-build" && path == &unresolved_root
        ));
    }

    #[test]
    #[cfg(unix)]
    fn test_missing_name_field_warning_message() {
        let package_json_path =
            AbsoluteSystemPathBuf::new("/my-project/packages/app/package.json").unwrap();
        let missing_name_error = Error::PackageJsonMissingName(package_json_path.clone());

        let fake_repo_root = AbsoluteSystemPathBuf::new("/my-project").unwrap();
        let fake_package_manager = crate::package_manager::PackageManager::Npm;
        let extracted_path = extract_file_path_from_error(
            &missing_name_error,
            &fake_package_manager,
            &fake_repo_root,
        );
        assert_eq!(extracted_path, package_json_path);

        let warning_message = format!(
            "An issue occurred while attempting to parse {}. Turborepo will still function, but \
             some features may not be available:\n {:?}",
            package_json_path,
            miette::Report::new(missing_name_error)
        );

        insta::assert_snapshot!("missing_name_field_warning_message", warning_message);
    }
}
