use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, OnceLock},
};

use itertools::Itertools;
use petgraph::{
    graph::{Edge, NodeIndex},
    visit::EdgeRef,
};
use serde::Serialize;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_lockfiles::Lockfile;

use crate::{
    discovery::LocalPackageDiscoveryBuilder,
    external_resolution::{
        ExternalDeclarations, ExternalPackageIdentity, ExternalResolutionData,
        ExternalResolutionGeneration, ExternalResolutionStatus, PackageExternalDeclarations,
        PackageResolutionState,
    },
    knowledge::{RelationshipKnowledge, RepositoryKnowledge},
    package_json::PackageJson,
    package_manager::PackageManager,
};

pub mod builder;
mod dep_splitter;
mod javascript;
mod projections;

pub use builder::{Error, PackageGraphBuilder};
pub use javascript::ChangedPackagesError;
pub use projections::{
    AffectedRelationships, FilteringRelationships, HashRelationships, OrderingRelationships,
    PruneDependencyMode, PruneRelationships, RelationshipProjectionError,
};

pub use crate::package_json::DependencyKind;

pub const ROOT_PKG_NAME: &str = "//";

#[derive(Debug)]
struct ExternalResolutionKnowledge {
    status: ExternalResolutionStatus,
    generation: Option<Arc<ExternalResolutionGeneration>>,
}

impl ExternalResolutionKnowledge {
    fn absent() -> Self {
        Self {
            status: ExternalResolutionStatus::Complete,
            generation: None,
        }
    }

    fn complete(generation: Arc<ExternalResolutionGeneration>) -> Self {
        Self {
            status: ExternalResolutionStatus::Complete,
            generation: Some(generation),
        }
    }
}

#[derive(Debug)]
pub struct PackageGraph {
    graph: petgraph::Graph<PackageNode, DependencyKind>,
    root_node_index: NodeIndex,
    root_workspace_index: NodeIndex,
    #[allow(dead_code)]
    node_lookup: HashMap<PackageNode, petgraph::graph::NodeIndex>,
    package_payloads: HashMap<PackageName, PackageInfo>,
    root_package_json: Option<PackageJson>,
    package_manager: Option<PackageManager>,
    lockfile: Option<Arc<dyn Lockfile>>,
    knowledge: Arc<RepositoryKnowledge>,
    /// The exact normalized relationship generation projected into `graph` and
    /// unresolved external declaration maps.
    #[allow(dead_code)]
    relationship_knowledge: Arc<RelationshipKnowledge>,
    external_declarations: OnceLock<ExternalDeclarations>,
    relationship_projections: OnceLock<projections::RelationshipProjections>,
    /// The sole owner of external-resolution terminal knowledge across
    /// toolchains. Resolution is complete when the package graph is returned
    /// from construction.
    external_resolution: Mutex<ExternalResolutionKnowledge>,
    /// Lazy reverse index from exact external identities to internal
    /// dependents. Built once from the resolution generation, not from
    /// `PackageInfo` compatibility payloads.
    external_dep_to_internal_dependents: OnceLock<ExternalDependencyIndex>,
    /// Lazily computed internal dependencies of the root package. They are
    /// implied dependencies of every package, so per-package operations like
    /// `dependencies` and `ancestors` consult them on every call; the set is
    /// invariant once the graph is built.
    root_internal_dependencies: OnceLock<HashSet<PackageNode>>,
    /// Toolchains registered during graph construction. Authoritative contexts
    /// determine which registrations were active for a particular concern.
    toolchains: crate::toolchain::ToolchainRegistry,
    /// Immutable native-task catalog produced during repository construction.
    native_task_knowledge: Arc<crate::native_tasks::NativeTaskKnowledge>,
}

/// The WorkspacePackage.
///
/// It follows the Vercel glossary of terms where "Workspace"
/// is the collection of packages and "Package" is a single package within the
/// workspace. https://vercel.com/docs/vercel-platform/glossary
/// There are other structs in this module that have "Workspace" in the name,
/// but they do NOT follow the glossary, and instead mean "package" when they
/// say Workspace. Some of these are labeled as such.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct WorkspacePackage {
    pub name: PackageName,
    pub path: AnchoredSystemPathBuf,
}

impl WorkspacePackage {
    pub fn root() -> Self {
        Self {
            name: PackageName::Root,
            path: AnchoredSystemPathBuf::default(),
        }
    }
}

/// Compatibility data retained for descriptor relationship classification
/// and task consumers.
///
/// Package identity, directory ownership, definition source, provenance, and
/// external-resolution state are authoritative in repository knowledge, not in
/// this projection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageInfo {
    /// A temporary compatibility descriptor for relationship classification
    /// and tasks.
    /// Its native `name` may be retained for later payload semantics, but must
    /// never be used as package identity or to derive paths or provenance.
    pub package_json: PackageJson,
}

// PackageName refers to a real package's name or the root package.
// It's not the best name, because root isn't a real package, but it's
// the best we have right now.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum PackageName {
    Root,
    Other(String),
}

impl Serialize for PackageName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PackageName::Root => serializer.serialize_str(ROOT_PKG_NAME),
            PackageName::Other(other) => serializer.serialize_str(other),
        }
    }
}

impl PackageName {
    pub fn as_str(&self) -> &str {
        match self {
            PackageName::Root => ROOT_PKG_NAME,
            PackageName::Other(name) => name,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum PackageNode {
    Root,
    Workspace(PackageName),
}

impl PackageNode {
    pub fn as_package_name(&self) -> &PackageName {
        match self {
            PackageNode::Workspace(name) => name,
            PackageNode::Root => &PackageName::Root,
        }
    }
}

/// The role of an identity-bearing scope or structural node in the package
/// graph. In particular, the graph sentinel and the root JavaScript execution
/// scope are separate nodes even though both have historically been called
/// "root".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageGraphNodeKind {
    Package,
    Aggregate,
    RootJavaScript,
    GraphSentinel,
}

/// Manifest-independent facts about a package graph node.
///
/// Paths and provenance come from the graph's immutable repository knowledge,
/// not from the [`PackageInfo`] compatibility projection. The graph sentinel
/// has no directory, native definition, or toolchain provenance.
#[derive(Debug, Clone, Copy)]
pub struct PackageGraphNodeView<'a> {
    kind: PackageGraphNodeKind,
    directory: Option<&'a AnchoredSystemPath>,
    definition_path: Option<&'a AnchoredSystemPath>,
    toolchain: Option<&'a crate::toolchain::ToolchainId>,
}

/// Identity-bearing execution context for a Turbo task namespace.
///
/// Only [`PackageGraph::package_task_context`] can construct this value, so
/// its identity, authoritative directory, and optional compatibility payload
/// cannot be assembled from unrelated graph entries. The root Turbo namespace
/// exists at the repository directory even when the repository has no root
/// JavaScript scope (for example, a pure Cargo workspace).
#[derive(Debug, Clone)]
pub struct PackageTaskContext<'a> {
    package: PackageName,
    repository_root: &'a AbsoluteSystemPath,
    directory: &'a AnchoredSystemPath,
    package_info: Option<&'a PackageInfo>,
    external_declarations: &'a ExternalDeclarations,
    native_tasks: &'a crate::native_tasks::ScopeNativeTasks,
    kind: PackageTaskContextKind,
    toolchain: Option<&'a crate::toolchain::ToolchainId>,
    requires_compatibility_payload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageTaskContextKind {
    Root,
    Package,
    Aggregate,
}

impl<'a> PackageTaskContext<'a> {
    #[cfg(test)]
    #[rustfmt::skip]
    pub(crate) fn new_for_test(package: PackageName, repository_root: &'a AbsoluteSystemPath, directory: &'a AnchoredSystemPath, package_info: Option<&'a PackageInfo>, kind: PackageTaskContextKind, toolchain: Option<&'a crate::toolchain::ToolchainId>) -> Self {
        Self::new_for_test_with_native_tasks(package, repository_root, directory, package_info, kind, toolchain, None)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test_with_native_tasks(
        package: PackageName,
        repository_root: &'a AbsoluteSystemPath,
        directory: &'a AnchoredSystemPath,
        package_info: Option<&'a PackageInfo>,
        kind: PackageTaskContextKind,
        toolchain: Option<&'a crate::toolchain::ToolchainId>,
        native_tasks: Option<Vec<crate::native_tasks::NativeTask>>,
    ) -> Self {
        static EXTERNAL_DECLARATIONS: OnceLock<ExternalDeclarations> = OnceLock::new();
        static UNKNOWN_NATIVE_TASKS: crate::native_tasks::ScopeNativeTasks =
            crate::native_tasks::ScopeNativeTasks::UnknownScope;
        let external_declarations =
            EXTERNAL_DECLARATIONS.get_or_init(ExternalDeclarations::default);
        let native_tasks = if let Some(tasks) = native_tasks {
            let scope = if tasks.is_empty() {
                crate::native_tasks::ScopeNativeTasks::Empty
            } else {
                crate::native_tasks::ScopeNativeTasks::Available(tasks.into_boxed_slice())
            };
            Box::leak(Box::new(scope)) as &'static _
        } else if let Some(info) = package_info {
            let observation = crate::native_tasks::observation_from_package_json(
                package.as_str(),
                &info.package_json,
            );
            let scope = if observation.tasks.is_empty() {
                crate::native_tasks::ScopeNativeTasks::Empty
            } else {
                crate::native_tasks::ScopeNativeTasks::Available(
                    observation.tasks.into_boxed_slice(),
                )
            };
            Box::leak(Box::new(scope)) as &'static _
        } else {
            &UNKNOWN_NATIVE_TASKS
        };
        let requires_compatibility_payload = package != PackageName::Root
            || toolchain == Some(&crate::toolchain::ToolchainId::JAVASCRIPT);
        Self {
            package,
            repository_root,
            directory,
            package_info,
            external_declarations,
            native_tasks,
            kind,
            toolchain,
            requires_compatibility_payload,
        }
    }

    pub fn package(&self) -> &PackageName {
        &self.package
    }

    pub fn repository_root(&self) -> &'a AbsoluteSystemPath {
        self.repository_root
    }

    pub fn directory(&self) -> &'a AnchoredSystemPath {
        self.directory
    }

    pub fn package_info(&self) -> Option<&'a PackageInfo> {
        self.package_info
    }

    pub fn external_declarations(&self) -> PackageExternalDeclarations<'_> {
        self.external_declarations
            .for_package(self.package.as_str())
    }

    pub fn native_tasks(&self) -> &'a crate::native_tasks::ScopeNativeTasks {
        self.native_tasks
    }

    pub fn kind(&self) -> PackageTaskContextKind {
        self.kind
    }

    pub fn toolchain(&self) -> Option<&'a crate::toolchain::ToolchainId> {
        self.toolchain
    }

    pub fn requires_compatibility_payload(&self) -> bool {
        self.requires_compatibility_payload
    }
}

impl<'a> PackageGraphNodeView<'a> {
    pub fn kind(&self) -> PackageGraphNodeKind {
        self.kind
    }

    pub fn directory(&self) -> Option<&'a AnchoredSystemPath> {
        self.directory
    }

    pub fn definition_path(&self) -> Option<&'a AnchoredSystemPath> {
        self.definition_path
    }

    pub fn toolchain(&self) -> Option<&'a crate::toolchain::ToolchainId> {
        self.toolchain
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalDependencyChange {
    pub package: WorkspacePackage,
    /// Dependencies that were added to the package
    pub added: Vec<turborepo_lockfiles::Package>,
    /// Dependencies that were removed from the package
    pub removed: Vec<turborepo_lockfiles::Package>,
}

/// Lazy reverse index from exact external identities to internal dependents.
#[derive(Debug, Default)]
struct ExternalDependencyIndex {
    /// Unique identities retaining producer-supplied display names.
    identities: Vec<ExternalPackageIdentity>,
    /// Exact identity -> internal workspace dependents (including transitive).
    dependents: HashMap<ExternalPackageIdentity, HashSet<PackageNode>>,
}

impl PackageGraph {
    fn external_declaration_view(&self) -> &ExternalDeclarations {
        self.external_declarations
            .get_or_init(|| ExternalDeclarations::build(&self.relationship_knowledge))
    }

    pub fn external_declarations<'a>(
        &'a self,
        package: &'a PackageName,
    ) -> PackageExternalDeclarations<'a> {
        self.external_declaration_view()
            .for_package(package.as_str())
    }

    fn relationship_projections(&self) -> &projections::RelationshipProjections {
        self.relationship_projections.get_or_init(|| {
            projections::RelationshipProjections::build(
                &self.knowledge,
                &self.relationship_knowledge,
            )
        })
    }

    /// Returns direct graph-forming relationships for task ordering.
    ///
    /// The view shares this graph generation's immutable relationship index and
    /// never exposes the structural [`PackageNode::Root`] sentinel.
    pub fn ordering_relationships(&self) -> &OrderingRelationships {
        self.relationship_projections().ordering()
    }

    /// Returns transitive relationships with root-implied filtering semantics.
    pub fn filtering_relationships(&self) -> &FilteringRelationships {
        self.relationship_projections().filtering()
    }

    /// Returns reverse ordering relationships for affectedness propagation.
    pub fn affected_relationships(&self) -> &AffectedRelationships {
        self.relationship_projections().affected()
    }

    /// Returns full transitive internal dependency inputs for hashing.
    pub fn hash_relationships(&self) -> &HashRelationships {
        self.relationship_projections().hash()
    }

    /// Returns install-oriented package relationships for pruning.
    ///
    /// Required peers whose declaration names match authoritative workspaces
    /// are included independently of classification and graph declaration
    /// precedence; optional peers are excluded.
    pub fn prune_relationships(&self) -> &PruneRelationships {
        self.relationship_projections().prune()
    }

    pub fn builder(
        repo_root: &AbsoluteSystemPath,
        root_package_json: PackageJson,
    ) -> PackageGraphBuilder<'_, LocalPackageDiscoveryBuilder> {
        PackageGraphBuilder::new(repo_root, root_package_json)
    }

    /// Build over a repository that may have no root `package.json` (a pure
    /// Cargo workspace). See [`PackageGraphBuilder::new_optional`].
    pub fn builder_optional(
        repo_root: &AbsoluteSystemPath,
        root_package_json: Option<PackageJson>,
    ) -> PackageGraphBuilder<'_, LocalPackageDiscoveryBuilder> {
        PackageGraphBuilder::new_optional(repo_root, root_package_json)
    }

    /// Validates that every non-root package has a `name` field in its
    /// package.json.
    ///
    /// Structural invariants (cycles, self-dependencies) are intentionally not
    /// checked here — those are caught at the task graph level by the engine
    /// builder, since package-level cycles don't necessarily produce invalid
    /// task execution orders. A warning is logged when cycles or
    /// self-dependencies are detected so users have visibility into the graph
    /// structure.
    ///
    /// # Errors
    ///
    /// Returns `Error::PackageJsonMissingName` if any non-root package is
    /// missing a `name` field in its package.json.
    #[tracing::instrument(skip(self))]
    pub fn validate(&self) -> Result<(), Error> {
        for package in self.knowledge.packages() {
            if package.user_facing_name().is_empty() {
                return Err(Error::PackageJsonMissingName(
                    self.repo_root().resolve(package.definition_path()),
                ));
            }
        }
        for aggregate in self.knowledge.aggregate_scopes() {
            if aggregate.user_facing_name().is_empty() {
                return Err(Error::PackageJsonMissingName(
                    self.repo_root().resolve(aggregate.definition_path()),
                ));
            }
        }

        for edge in self.graph.edge_references() {
            if edge.source() == edge.target()
                && let Some(PackageNode::Workspace(PackageName::Other(name))) =
                    self.graph.node_weight(edge.source())
            {
                tracing::warn!("Package \"{name}\" depends on itself");
            }
        }

        if petgraph::algo::is_cyclic_directed(&self.graph) {
            let sccs = petgraph::algo::tarjan_scc(&self.graph);
            let cycle_members: Vec<String> = sccs
                .into_iter()
                .filter(|scc| scc.len() > 1)
                .flat_map(|scc| {
                    scc.into_iter()
                        .filter_map(|idx| match self.graph.node_weight(idx)? {
                            PackageNode::Workspace(PackageName::Other(name)) => {
                                Some(name.to_string())
                            }
                            _ => None,
                        })
                })
                .collect();
            if !cycle_members.is_empty() {
                tracing::warn!(
                    "Circular package dependency detected: {}",
                    cycle_members.join(", ")
                );
            }
        }

        Ok(())
    }

    /// Returns strongly connected components with more than one member,
    /// representing circular dependency chains in the package graph.
    /// Each inner Vec is ordered to trace a representative cycle path
    /// through the SCC, rotated so the lexicographically smallest name
    /// comes first.
    pub fn find_cycles(&self) -> Vec<Vec<PackageName>> {
        if !petgraph::algo::is_cyclic_directed(&self.graph) {
            return Vec::new();
        }

        let sccs = petgraph::algo::tarjan_scc(&self.graph);
        let mut cycles: Vec<Vec<PackageName>> = sccs
            .into_iter()
            .filter(|scc| scc.len() > 1)
            .filter_map(|scc| {
                let scc_set: HashSet<NodeIndex> = scc.into_iter().collect();
                self.trace_cycle_path(&scc_set)
            })
            .collect();

        // Sort for deterministic output
        cycles.sort();
        cycles
    }

    /// Trace a representative cycle path through an SCC by following edges
    /// deterministically. Starts from the smallest NodeIndex and always
    /// picks the smallest NodeIndex neighbor to ensure consistent results
    /// across runs and platforms.
    fn trace_cycle_path(&self, scc: &HashSet<NodeIndex>) -> Option<Vec<PackageName>> {
        let start = *scc.iter().min()?;
        let mut path: Vec<NodeIndex> = Vec::new();
        let mut visited: HashMap<NodeIndex, usize> = HashMap::new();
        let mut current = start;

        loop {
            if let Some(&cycle_start_idx) = visited.get(&current) {
                let cycle_indices = &path[cycle_start_idx..];
                let mut names: Vec<PackageName> = cycle_indices
                    .iter()
                    .filter_map(|idx| match self.graph.node_weight(*idx)? {
                        PackageNode::Workspace(name) if !matches!(name, PackageName::Root) => {
                            Some(name.clone())
                        }
                        _ => None,
                    })
                    .collect();

                if names.is_empty() {
                    return None;
                }

                // Rotate so the lexicographically smallest name comes first
                if let Some(min_pos) = names
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, name)| (*name).clone())
                    .map(|(i, _)| i)
                {
                    names.rotate_left(min_pos);
                }

                return Some(names);
            }

            visited.insert(current, path.len());
            path.push(current);

            // Pick the smallest NodeIndex neighbor within the SCC for
            // deterministic traversal
            current = self
                .graph
                .neighbors_directed(current, petgraph::Outgoing)
                .filter(|n| scc.contains(n))
                .min()?;
        }
    }

    pub fn remove_package_dependencies(&mut self) {
        self.graph.retain_edges(|graph, index| {
            let Some((_src, dst)) = graph.edge_endpoints(index) else {
                return false;
            };
            dst == self.root_node_index
        });
    }

    /// Returns the number of packages in the repo
    /// *including* the root package.
    pub fn len(&self) -> usize {
        self.package_task_contexts().count()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn package_manager(&self) -> Option<&PackageManager> {
        self.package_manager.as_ref()
    }

    /// Toolchains registered during graph construction.
    pub fn toolchains(&self) -> &crate::toolchain::ToolchainRegistry {
        &self.toolchains
    }

    /// The watch behavior of toolchains that contributed at least one
    /// authoritative execution scope to this graph. Registered toolchains
    /// that were inactive (notably extras in single-package mode) are omitted.
    pub fn active_watch_spec(&self) -> crate::toolchain::WatchSpec {
        let active: HashSet<_> = self
            .package_task_contexts()
            .filter_map(|context| context.toolchain().cloned())
            .collect();
        let mut watch_spec = crate::toolchain::WatchSpec::default();
        for toolchain in self.toolchains.iter() {
            if active.contains(&toolchain.id()) {
                watch_spec.extend(toolchain.watch_spec());
            }
        }
        watch_spec
    }

    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        self.knowledge.repository_root()
    }

    #[cfg(test)]
    pub(crate) fn repository_knowledge(&self) -> &RepositoryKnowledge {
        &self.knowledge
    }

    /// Whether a root `package.json` contributed a JavaScript execution scope.
    pub fn has_root_javascript_scope(&self) -> bool {
        self.knowledge.root_javascript_scope().is_some()
    }

    /// The root JavaScript scope's user-facing package name. The outer option
    /// distinguishes no JavaScript scope (pure Cargo) from an unnamed root
    /// JavaScript scope.
    pub fn root_javascript_scope_name(&self) -> Option<Option<&str>> {
        self.knowledge
            .root_javascript_scope()
            .map(|scope| scope.user_facing_name())
    }

    /// Looks up manifest-independent facts for an identity-bearing scope or
    /// the structural graph sentinel. In a pure Cargo repository,
    /// `Workspace(Root)` has no corresponding scope and therefore returns
    /// `None`; `Root` still returns the graph sentinel view.
    pub fn node_view(&self, node: &PackageNode) -> Option<PackageGraphNodeView<'_>> {
        match node {
            PackageNode::Root => Some(PackageGraphNodeView {
                kind: PackageGraphNodeKind::GraphSentinel,
                directory: None,
                definition_path: None,
                toolchain: None,
            }),
            PackageNode::Workspace(package) => self.package_view(package),
        }
    }

    /// Looks up manifest-independent facts for a compatibility package
    /// identity. Unlike [`Self::package_dir`], this returns `None` for the
    /// synthetic root workspace when no root JavaScript scope exists.
    pub fn package_view(&self, package: &PackageName) -> Option<PackageGraphNodeView<'_>> {
        match package {
            PackageName::Root => {
                let scope = self.knowledge.root_javascript_scope()?;
                Some(PackageGraphNodeView {
                    kind: PackageGraphNodeKind::RootJavaScript,
                    directory: Some(self.knowledge.repository_directory()),
                    definition_path: Some(scope.definition_path()),
                    toolchain: Some(scope.toolchain()),
                })
            }
            PackageName::Other(name) => {
                let scope = self.knowledge.scope(name)?;
                let kind = match scope.kind() {
                    crate::knowledge::ScopeKind::Package => PackageGraphNodeKind::Package,
                    crate::knowledge::ScopeKind::Aggregate => PackageGraphNodeKind::Aggregate,
                };
                Some(PackageGraphNodeView {
                    kind,
                    directory: Some(scope.directory()),
                    definition_path: Some(scope.definition_path()),
                    toolchain: Some(scope.toolchain()),
                })
            }
        }
    }

    /// Resolves the complete context for tasks in `package`.
    ///
    /// Non-root identities must exist in repository knowledge. Compatibility
    /// payload is associated when present, but is not authoritative and is not
    /// required to construct a context. The root identity denotes Turbo's root
    /// task namespace and is always anchored at the repository directory.
    pub fn package_task_context(&self, package: &PackageName) -> Option<PackageTaskContext<'_>> {
        let (package, directory, kind, toolchain, requires_compatibility_payload) = match package {
            PackageName::Root => (
                PackageName::Root,
                self.knowledge.repository_directory(),
                PackageTaskContextKind::Root,
                self.knowledge
                    .root_javascript_scope()
                    .map(|scope| scope.toolchain()),
                self.knowledge.root_javascript_scope().is_some(),
            ),
            PackageName::Other(name) => {
                let scope = self.knowledge.scope(name)?;
                let kind = match scope.kind() {
                    crate::knowledge::ScopeKind::Package => PackageTaskContextKind::Package,
                    crate::knowledge::ScopeKind::Aggregate => PackageTaskContextKind::Aggregate,
                };
                (
                    PackageName::Other(scope.identity().to_owned()),
                    scope.directory(),
                    kind,
                    Some(scope.toolchain()),
                    true,
                )
            }
        };
        let package_info = self.package_payloads.get(&package);
        let native_tasks = self.native_task_knowledge.for_scope(package.as_str());

        Some(PackageTaskContext {
            package,
            repository_root: self.knowledge.repository_root(),
            directory,
            package_info,
            external_declarations: self.external_declaration_view(),
            native_tasks,
            kind,
            toolchain,
            requires_compatibility_payload,
        })
    }

    /// Iterates every authoritative Turbo task namespace.
    ///
    /// The root namespace is always first and present exactly once, including
    /// in repositories without a root JavaScript scope. All other identities
    /// follow authoritative repository observation order; compatibility
    /// payload entries cannot add or remove namespaces from this iterator.
    pub fn package_task_contexts(&self) -> impl Iterator<Item = PackageTaskContext<'_>> + '_ {
        std::iter::once(PackageName::Root)
            .chain(
                self.knowledge
                    .scopes()
                    .map(|scope| PackageName::Other(scope.identity().to_owned())),
            )
            .map(|package| match self.package_task_context(&package) {
                Some(context) => context,
                None => unreachable!("authoritative package name must resolve to a task context"),
            })
    }

    /// Test hook for exercising knowledge-backed consumers without a
    /// compatibility projection.
    #[cfg(any(test, feature = "test-util"))]
    pub fn remove_package_info_for_test(&mut self, package: &PackageName) -> Option<PackageInfo> {
        self.package_payloads.remove(package)
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn set_compatibility_manifest_name_for_test(
        &mut self,
        package: &PackageName,
        compatibility_manifest_name: Option<String>,
    ) -> bool {
        let Some(payload) = self.package_payloads.get_mut(package) else {
            return false;
        };
        payload.package_json.name = compatibility_manifest_name.map(turborepo_errors::Spanned::new);
        true
    }

    /// Iterates the structural graph sentinel followed by every authoritative
    /// execution scope. The root JavaScript scope is omitted when no root
    /// `package.json` exists.
    pub fn node_views(&self) -> impl Iterator<Item = (PackageNode, PackageGraphNodeView<'_>)> + '_ {
        std::iter::once((PackageNode::Root, self.node_view(&PackageNode::Root)))
            .chain(std::iter::once((
                PackageNode::Workspace(PackageName::Root),
                self.node_view(&PackageNode::Workspace(PackageName::Root)),
            )))
            .chain(self.knowledge.scopes().map(|scope| {
                let node = PackageNode::Workspace(PackageName::Other(scope.identity().to_string()));
                let view = PackageGraphNodeView {
                    kind: match scope.kind() {
                        crate::knowledge::ScopeKind::Package => PackageGraphNodeKind::Package,
                        crate::knowledge::ScopeKind::Aggregate => PackageGraphNodeKind::Aggregate,
                    },
                    directory: Some(scope.directory()),
                    definition_path: Some(scope.definition_path()),
                    toolchain: Some(scope.toolchain()),
                };
                (node, Some(view))
            }))
            .filter_map(|(node, view)| view.map(|view| (node, view)))
    }

    /// User-facing identities of real packages, excluding root and aggregate
    /// execution scopes.
    pub fn real_package_names(&self) -> impl Iterator<Item = &str> {
        self.knowledge.packages().map(|scope| scope.identity())
    }

    /// User-facing identities of non-package aggregate execution scopes.
    pub fn aggregate_scope_names(&self) -> impl Iterator<Item = &str> {
        self.knowledge
            .aggregate_scopes()
            .map(|scope| scope.identity())
    }

    /// Iterates authoritative execution-scope identities and their directories.
    /// The structural graph sentinel is excluded, while aggregate scopes are
    /// included and the root JavaScript scope is present only when it exists.
    pub fn package_scope_directories(
        &self,
    ) -> impl Iterator<Item = (PackageName, &AnchoredSystemPath)> + '_ {
        self.node_views().filter_map(|(node, view)| match node {
            PackageNode::Root => None,
            PackageNode::Workspace(package) => {
                view.directory().map(|directory| (package, directory))
            }
        })
    }

    /// Native definition path for a package or execution scope. A pure Cargo
    /// repository's compatibility root node has no native definition.
    pub fn package_definition_path(&self, package: &PackageName) -> Option<&AnchoredSystemPath> {
        self.package_view(package)?.definition_path()
    }

    /// Toolchain provenance for a package or execution scope.
    pub fn package_toolchain(
        &self,
        package: &PackageName,
    ) -> Option<&crate::toolchain::ToolchainId> {
        self.package_view(package)?.toolchain()
    }

    /// Whether this identity represents a real package rather than an
    /// execution-only scope.
    pub fn is_real_package(&self, package: &PackageName) -> bool {
        self.package_view(package)
            .is_some_and(|view| view.kind() == PackageGraphNodeKind::Package)
    }

    /// Whether this identity represents a non-package aggregate scope.
    pub fn is_aggregate_scope(&self, package: &PackageName) -> bool {
        self.package_view(package)
            .is_some_and(|view| view.kind() == PackageGraphNodeKind::Aggregate)
    }

    pub fn lockfile(&self) -> Option<&dyn Lockfile> {
        self.lockfile.as_deref()
    }

    pub fn external_resolution_status(&self) -> ExternalResolutionStatus {
        self.external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
    }

    pub fn package_resolution_states(&self) -> HashMap<String, PackageResolutionState> {
        let resolution = self
            .external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.package_task_contexts()
            .map(|context| {
                let state = match context.toolchain() {
                    None => PackageResolutionState::NotApplicable,
                    Some(toolchain) => resolution
                        .generation
                        .as_deref()
                        .and_then(|generation| {
                            generation
                                .domains()
                                .iter()
                                .find(|domain| domain.toolchain() == toolchain)
                        })
                        .map_or(PackageResolutionState::Missing, |domain| {
                            match domain.data() {
                                ExternalResolutionData::Resolved {
                                    completeness,
                                    packages,
                                    ..
                                } => packages
                                    .iter()
                                    .find(|package| package.package() == context.package().as_str())
                                    .and_then(|package| package.fingerprint())
                                    .map_or(PackageResolutionState::Missing, |fingerprint| {
                                        PackageResolutionState::Resolved {
                                            completeness: completeness.clone(),
                                            fingerprint: fingerprint.clone(),
                                        }
                                    }),
                                ExternalResolutionData::Unavailable(reason) => {
                                    PackageResolutionState::Unavailable(reason.clone())
                                }
                            }
                        }),
                };
                (context.package().as_str().to_string(), state)
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn external_resolution_generation(
        &self,
    ) -> Option<Arc<ExternalResolutionGeneration>> {
        self.external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .generation
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn remove_external_resolution_for_test(&mut self) {
        self.external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .generation = None;
    }

    pub fn package_json(&self, package: &PackageName) -> Option<&PackageJson> {
        let entry = self.package_payloads.get(package)?;
        Some(&entry.package_json)
    }

    pub fn package_dir(&self, package: &PackageName) -> Option<&AnchoredSystemPath> {
        match package {
            // Compatibility: the synthetic root workspace has historically
            // had the repository directory even when no JavaScript scope
            // exists. `node_view` is the API that distinguishes those cases.
            PackageName::Root => Some(self.knowledge.repository_directory()),
            PackageName::Other(_) => self.package_view(package)?.directory(),
        }
    }

    pub fn package_info(&self, package: &PackageName) -> Option<&PackageInfo> {
        self.package_payloads.get(package)
    }

    fn package_dir_for_node(&self, node: &PackageNode) -> Option<&AnchoredSystemPath> {
        match node {
            PackageNode::Workspace(package) => self.package_dir(package),
            PackageNode::Root => None,
        }
    }

    pub fn get_package_by_index(&self, index: NodeIndex) -> Option<&PackageNode> {
        self.graph.node_weight(index)
    }

    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> {
        self.graph.node_indices()
    }

    pub fn edges(&self) -> &[Edge<DependencyKind>] {
        self.graph.raw_edges()
    }

    /// Returns the dependency kind for a directed edge between two workspace
    /// packages, if one exists.
    pub fn dependency_kind(&self, from: &PackageNode, to: &PackageNode) -> Option<DependencyKind> {
        let from_index = self.node_lookup.get(from)?;
        let to_index = self.node_lookup.get(to)?;
        self.graph
            .edges_connecting(*from_index, *to_index)
            .next()
            .map(|edge| *edge.weight())
    }

    pub fn root_package_json(&self) -> Option<&PackageJson> {
        self.root_package_json.as_ref()
    }

    /// Gets all the nodes that directly depend on this one, that is to say
    /// have a edge to `package`.
    ///
    /// Example:
    ///
    /// a -> b -> c
    ///
    /// immediate_dependencies(a) -> {b}
    pub fn immediate_dependencies(&self, package: &PackageNode) -> Option<HashSet<&PackageNode>> {
        Some(self.immediate_dependencies_iter(package)?.collect())
    }

    /// [`PackageGraph::immediate_dependencies`] without materializing a
    /// set. Hot paths that only iterate the dependencies (engine graph
    /// construction queries this once per task) skip hashing every
    /// package name into a `HashSet`.
    pub fn immediate_dependencies_iter(
        &self,
        package: &PackageNode,
    ) -> Option<impl Iterator<Item = &PackageNode> + Clone + '_> {
        let index = self.node_lookup.get(package)?;
        Some(
            self.graph
                .neighbors_directed(*index, petgraph::Outgoing)
                .map(|index| {
                    self.graph
                        .node_weight(index)
                        .expect("node index from neighbors should be present")
                }),
        )
    }

    /// Gets all the nodes that directly depend on this one, that is to say
    /// have a edge to `package`.
    ///
    /// Example:
    ///
    /// a -> b -> c
    ///
    /// immediate_ancestors(c) -> {b}
    #[allow(dead_code)]
    pub fn immediate_ancestors(&self, package: &PackageNode) -> Option<HashSet<&PackageNode>> {
        let index = self.node_lookup.get(package)?;
        Some(
            self.graph
                .neighbors_directed(*index, petgraph::Incoming)
                .map(|index| {
                    self.graph
                        .node_weight(index)
                        .expect("node index from neighbors should be present")
                })
                .collect(),
        )
    }

    /// For a given package in the repo, returns the set of packages
    /// that this one depends on, excluding those that are unresolved.
    ///
    /// Example:
    ///
    /// a -> b -> c (external)
    ///
    /// dependencies(a) = {b, c}
    ///
    /// If the package graph contains cycles, the returned set will include
    /// all members of any cycle reachable from `node`.
    #[allow(dead_code)]
    pub fn dependencies<'a>(&'a self, node: &PackageNode) -> HashSet<&'a PackageNode> {
        let mut dependencies = turborepo_graph_utils::transitive_closure(
            &self.graph,
            self.node_lookup.get(node).cloned(),
            petgraph::Direction::Outgoing,
        );
        // Add in all root dependencies as they're implied dependencies for every
        // package in the graph.
        dependencies.extend(self.root_internal_dependencies());
        dependencies.remove(node);
        dependencies
    }

    /// For a given package in the repo, returns the set of packages
    /// that depend on this one, excluding those that are unresolved.
    ///
    /// Example:
    ///
    /// a -> b -> c (external)
    ///
    /// ancestors(c) = {a, b}
    ///
    /// If the package graph contains cycles, the returned set will include
    /// all members of any cycle reachable from `node`.
    pub fn ancestors(&self, node: &PackageNode) -> HashSet<&PackageNode> {
        // If node is a root dep, then *every* package is an ancestor of this one
        let mut dependents = if self.root_internal_dependencies().contains(node) {
            return self.graph.node_weights().collect();
        } else {
            turborepo_graph_utils::transitive_closure(
                &self.graph,
                self.node_lookup.get(node).cloned(),
                petgraph::Direction::Incoming,
            )
        };
        dependents.remove(node);
        dependents
    }

    pub fn root_internal_package_dependencies(&self) -> HashSet<WorkspacePackage> {
        let dependencies = self.root_internal_dependencies();
        dependencies
            .iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(package) => {
                    let path = self.package_dir_for_node(node)?;
                    Some(WorkspacePackage {
                        name: package.clone(),
                        path: path.to_owned(),
                    })
                }
                PackageNode::Root => None,
            })
            .collect()
    }

    pub fn root_internal_package_dependencies_paths(&self) -> Vec<&AnchoredSystemPath> {
        let dependencies = self.root_internal_dependencies();
        dependencies
            .iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(_) => self.package_dir_for_node(node),
                PackageNode::Root => None,
            })
            .sorted()
            .collect()
    }

    /// Provides a path from the root package to package.
    ///
    /// Currently only provides the shortest path as calculating all paths can
    /// be O(n!). If the package graph contains cycles, the shortest path may
    /// traverse through cycle members.
    pub fn root_internal_dependency_explanation(
        &self,
        package: &WorkspacePackage,
    ) -> Option<String> {
        let from = self.root_workspace_index;
        let to = *self
            .node_lookup
            .get(&PackageNode::Workspace(package.name.clone()))?;
        let (_cost, path) =
            petgraph::algo::astar(&self.graph, from, |node| node == to, |_| 1, |_| 1)?;
        self.path_display(&path)
    }

    fn path_display(&self, path: &[petgraph::graph::NodeIndex]) -> Option<String> {
        let mut package_names = Vec::with_capacity(path.len());
        for index in path {
            let node = self.graph.node_weight(*index)?;
            let name = node.as_package_name().to_string();
            package_names.push(name);
        }

        Some(package_names.join(" -> "))
    }

    fn root_internal_dependencies(&self) -> &HashSet<PackageNode> {
        self.root_internal_dependencies.get_or_init(|| {
            // We cannot call self.dependencies(&PackageNode::Workspace(PackageName::Root))
            // as it will infinitely recurse.
            let mut dependencies: HashSet<PackageNode> = turborepo_graph_utils::transitive_closure(
                &self.graph,
                Some(self.root_workspace_index),
                petgraph::Direction::Outgoing,
            )
            .into_iter()
            .cloned()
            .collect();
            dependencies.remove(&PackageNode::Workspace(PackageName::Root));
            dependencies
        })
    }

    /// Returns the transitive closure of the given nodes in the package
    /// graph. Note that this includes the nodes themselves. If you want just
    /// the dependencies, or the dependents, use `dependencies` or `ancestors`.
    /// Alternatively, if you need just direct dependents, use
    /// `immediate_dependents`.
    ///
    /// If the package graph contains cycles, the returned set will include
    /// all members of any cycle reachable from the starting nodes.
    pub fn transitive_closure<'a, 'b, I: IntoIterator<Item = &'b PackageNode>>(
        &'a self,
        nodes: I,
    ) -> HashSet<&'a PackageNode> {
        turborepo_graph_utils::transitive_closure(
            &self.graph,
            nodes
                .into_iter()
                .flat_map(|node| self.node_lookup.get(node).cloned()),
            petgraph::Direction::Outgoing,
        )
    }

    /// Like [`Self::transitive_closure`], but only follows edges with
    /// [`DependencyKind::Production`].
    pub fn production_transitive_closure<'a, 'b, I: IntoIterator<Item = &'b PackageNode>>(
        &'a self,
        nodes: I,
    ) -> HashSet<&'a PackageNode> {
        let mut visited = HashSet::new();
        let mut stack: Vec<NodeIndex> = nodes
            .into_iter()
            .filter_map(|node| self.node_lookup.get(node).cloned())
            .collect();

        while let Some(index) = stack.pop() {
            let Some(node) = self.graph.node_weight(index) else {
                continue;
            };
            if !visited.insert(node) {
                continue;
            }

            for edge in self.graph.edges(index) {
                if matches!(*edge.weight(), DependencyKind::Production) {
                    stack.push(edge.target());
                }
            }
        }

        visited
    }

    /// Unique external package identities from the resolution generation.
    ///
    /// Display names are the producer-supplied human names captured during
    /// observation. This is the query authority for external package listing.
    pub fn external_package_identities(&self) -> &[ExternalPackageIdentity] {
        &self.external_dependency_index().identities
    }

    /// Exact external identities attributed to the given packages by the
    /// resolution generation. Used by prune lockfile-key unions so they no
    /// read resolution generation identities.
    pub fn external_package_identities_for_packages<'a, I>(
        &self,
        packages: I,
    ) -> Vec<ExternalPackageIdentity>
    where
        I: IntoIterator<Item = &'a PackageName>,
    {
        let wanted: HashSet<&str> = packages.into_iter().map(PackageName::as_str).collect();
        if wanted.is_empty() {
            return Vec::new();
        }
        let Some(generation) = self.resolution_generation() else {
            return Vec::new();
        };

        let mut seen = HashSet::new();
        let mut identities = Vec::new();
        for domain in generation.domains() {
            let ExternalResolutionData::Resolved {
                packages: resolved, ..
            } = domain.data()
            else {
                continue;
            };
            for package in resolved {
                if !wanted.contains(package.package()) {
                    continue;
                }
                for identity in package.identities() {
                    if seen.insert(identity.clone()) {
                        identities.push(identity.clone());
                    }
                }
            }
        }
        identities.sort();
        identities
    }

    /// Global-hash file fallback when JavaScript resolution is unavailable.
    ///
    /// Mirrors the historical behavior of hashing `package.json` plus the
    /// package-manager lockfile path when a lockfile object was not loaded:
    /// include the repository root `package.json` and any existing resolution
    /// definition sources from the JavaScript domain.
    pub fn external_resolution_global_file_fallback(&self) -> Option<Vec<AbsoluteSystemPathBuf>> {
        let generation = self.resolution_generation()?;
        let domain = generation
            .domains()
            .iter()
            .find(|domain| domain.toolchain() == &crate::toolchain::ToolchainId::JAVASCRIPT)?;
        match domain.data() {
            ExternalResolutionData::Resolved { .. } => None,
            ExternalResolutionData::Unavailable(_) => {
                let mut paths = vec![self.repo_root().join_component("package.json")];
                for source in domain.definition_sources() {
                    let path = self.repo_root().resolve(source);
                    if path.exists() {
                        paths.push(path);
                    }
                }
                Some(paths)
            }
        }
    }

    /// Resolve a lockfile package to the generation identity that shares its
    /// exact `(key, version)`, retaining any stored human name.
    pub fn resolve_external_package_identity(
        &self,
        package: &turborepo_lockfiles::Package,
    ) -> Option<&ExternalPackageIdentity> {
        let needle = ExternalPackageIdentity::new(package.key.clone(), package.version.clone());
        self.external_dependency_index()
            .identities
            .iter()
            .find(|identity| *identity == &needle)
    }

    /// JavaScript-domain external identities only. Used by N-API lockfile
    /// listing which historically filtered through the JS lockfile human-name
    /// path and therefore excluded Cargo identities.
    pub fn javascript_external_package_identities(&self) -> Vec<ExternalPackageIdentity> {
        let Some(generation) = self.resolution_generation() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut identities = Vec::new();
        for domain in generation.domains() {
            if domain.toolchain() != &crate::toolchain::ToolchainId::JAVASCRIPT {
                continue;
            }
            let ExternalResolutionData::Resolved { packages, .. } = domain.data() else {
                continue;
            };
            for package in packages {
                for identity in package.identities() {
                    if seen.insert(identity.clone()) {
                        identities.push(identity.clone());
                    }
                }
            }
        }
        identities.sort();
        identities
    }

    pub fn internal_dependencies_for_external_dependency(
        &self,
        external_package: &turborepo_lockfiles::Package,
    ) -> Option<&HashSet<PackageNode>> {
        let identity = ExternalPackageIdentity::new(
            external_package.key.clone(),
            external_package.version.clone(),
        );
        self.internal_dependencies_for_external_identity(&identity)
    }

    pub fn internal_dependencies_for_external_identity(
        &self,
        identity: &ExternalPackageIdentity,
    ) -> Option<&HashSet<PackageNode>> {
        self.external_dependency_index().dependents.get(identity)
    }

    fn external_dependency_index(&self) -> &ExternalDependencyIndex {
        self.external_dep_to_internal_dependents
            .get_or_init(|| self.build_external_dependency_index())
    }

    fn resolution_generation(&self) -> Option<Arc<ExternalResolutionGeneration>> {
        self.external_resolution
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .generation
            .clone()
    }

    /// Builds a compact reverse index from the resolution generation.
    fn build_external_dependency_index(&self) -> ExternalDependencyIndex {
        let Some(generation) = self.resolution_generation() else {
            return ExternalDependencyIndex::default();
        };

        let mut dependents: HashMap<ExternalPackageIdentity, HashSet<PackageNode>> = HashMap::new();
        let mut identity_order: Vec<ExternalPackageIdentity> = Vec::new();
        let mut seen_identities = HashSet::new();
        let mut root_external_dependencies = HashSet::new();

        for domain in generation.domains() {
            let ExternalResolutionData::Resolved { packages, .. } = domain.data() else {
                continue;
            };
            for package_resolution in packages {
                let workspace = PackageNode::Workspace(PackageName::from(
                    package_resolution.package().to_string(),
                ));
                let is_root = package_resolution.package() == ROOT_PKG_NAME
                    || package_resolution.package() == "//";
                for identity in package_resolution.identities() {
                    if seen_identities.insert(identity.clone()) {
                        identity_order.push(identity.clone());
                    }
                    if is_root {
                        root_external_dependencies.insert(identity.clone());
                    }
                    dependents
                        .entry(identity.clone())
                        .or_default()
                        .insert(workspace.clone());
                }
            }
        }

        identity_order.sort();

        let root_internal_dependencies = self.root_internal_dependencies();
        for (external_pkg, rdeps) in dependents.iter_mut() {
            if root_external_dependencies.contains(external_pkg)
                || !root_internal_dependencies.is_disjoint(rdeps)
            {
                rdeps.extend(self.graph.node_weights().cloned());
            } else {
                let transitive_rdeps = turborepo_graph_utils::transitive_closure(
                    &self.graph,
                    rdeps
                        .iter()
                        .filter_map(|node| self.node_lookup.get(node).copied()),
                    petgraph::Direction::Incoming,
                );
                rdeps.extend(transitive_rdeps.into_iter().cloned());
            }
        }

        ExternalDependencyIndex {
            identities: identity_order,
            dependents,
        }
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageName::Root => f.write_str("//"),
            PackageName::Other(other) => f.write_str(other),
        }
    }
}

impl fmt::Display for PackageNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageNode::Root => f.write_str("___ROOT___"),
            PackageNode::Workspace(package) => package.fmt(f),
        }
    }
}
impl From<String> for PackageName {
    fn from(value: String) -> Self {
        match value == "//" {
            true => Self::Root,
            false => Self::Other(value),
        }
    }
}

impl<'a> From<&'a str> for PackageName {
    fn from(value: &'a str) -> Self {
        Self::from(value.to_string())
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        match self {
            PackageName::Root => "//",
            PackageName::Other(package) => package,
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::BTreeMap, fs, path::Path, process::Command};

    use serde_json::json;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_errors::Spanned;

    use super::*;
    use crate::{
        change_mapper::{
            AllPackageChangeReason, ChangeMapper, GlobalDepsPackageChangeMapper, LockfileContents,
            PackageChanges,
        },
        discovery::PackageDiscovery,
    };

    struct MockDiscovery;
    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            Ok(crate::discovery::DiscoveryResponse {
                package_manager: PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<crate::discovery::DiscoveryResponse, crate::discovery::Error> {
            self.discover_packages().await
        }
    }

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crate is under <repo>/crates")
            .to_owned()
    }

    fn copy_dir_all(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            let dest = to.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_all(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }

    fn apply_patch(dir: &Path, target: &str, patch_file: &str) {
        let patch = fs::read_to_string(dir.join(patch_file))
            .unwrap()
            .lines()
            .map(|line| {
                line.strip_prefix("+++ ")
                    .map_or_else(|| line.to_string(), |_| format!("+++ {target}"))
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let rewritten = tempfile::NamedTempFile::new().unwrap();
        fs::write(rewritten.path(), patch).unwrap();
        let status = Command::new("git")
            .args(["apply", "--unsafe-paths"])
            .arg(rewritten.path())
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git apply {patch_file} failed");
    }

    fn setup_lockfile_aware_fixture(dir: &Path, pm_name: &str) {
        let root = repo_root();
        copy_dir_all(
            &root.join("turborepo-tests/integration/fixtures/lockfile_aware_caching"),
            dir,
        );
        copy_dir_all(
            &root.join(format!(
                "turborepo-tests/integration/tests/lockfile-aware-caching/{pm_name}"
            )),
            dir,
        );
    }

    fn build_lockfile_aware_graph(
        root: &AbsoluteSystemPath,
        package_manager: PackageManager,
    ) -> PackageGraph {
        let root_package_json = PackageJson::load(&root.join_component("package.json")).unwrap();
        let builder = PackageGraph::builder(root, root_package_json)
            .with_package_manager(package_manager)
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(HashMap::from([
                (
                    root.join_components(&["apps", "a", "package.json"]),
                    PackageJson::load(&root.join_components(&["apps", "a", "package.json"]))
                        .unwrap(),
                ),
                (
                    root.join_components(&["apps", "b", "package.json"]),
                    PackageJson::load(&root.join_components(&["apps", "b", "package.json"]))
                        .unwrap(),
                ),
            ])));

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(builder.build())
            .unwrap()
    }

    #[test]
    fn lockfile_changes_are_scoped_by_package_manager() {
        let cases = [
            (
                PackageManager::Npm,
                "npm",
                "package-lock.json",
                "package-lock.patch",
                "turbo-bump.patch",
            ),
            (
                PackageManager::Yarn,
                "yarn",
                "yarn.lock",
                "yarn-lock.patch",
                "turbo-bump.patch",
            ),
            (
                PackageManager::Pnpm,
                "pnpm",
                "pnpm-lock.yaml",
                "pnpm-lock.patch",
                "turbo-bump.patch",
            ),
            (
                PackageManager::Berry,
                "berry",
                "yarn.lock",
                "yarn-lock.patch",
                "turbo-bump.patch",
            ),
            (
                PackageManager::Bun,
                "bun",
                "bun.lock",
                "bun-lock.patch",
                "turbo-bump.patch",
            ),
        ];

        for (package_manager, pm_name, lockfile, dep_patch, root_patch) in cases {
            let tempdir = tempfile::tempdir().unwrap();
            setup_lockfile_aware_fixture(tempdir.path(), pm_name);
            let root = AbsoluteSystemPathBuf::try_from(tempdir.path()).unwrap();
            let root_package_json =
                PackageJson::load(&root.join_component("package.json")).unwrap();

            let previous_contents = fs::read(tempdir.path().join(lockfile)).unwrap();
            let previous = package_manager
                .read_lockfile(&root, &root_package_json)
                .unwrap();

            apply_patch(tempdir.path(), lockfile, dep_patch);
            let mut dep_graph = build_lockfile_aware_graph(&root, package_manager.clone());
            let mut dep_changed = dep_graph
                .changed_packages_from_lockfile(previous.as_ref())
                .unwrap();
            dep_changed.sort_by(|a, b| a.package.name.cmp(&b.package.name));

            assert_eq!(
                dep_changed
                    .iter()
                    .map(|change| change.package.name.clone())
                    .collect::<Vec<_>>(),
                vec![PackageName::from("b")],
                "{pm_name}: dependency lockfile change should only affect b"
            );

            {
                let detector =
                    GlobalDepsPackageChangeMapper::new(&dep_graph, std::iter::empty::<&str>())
                        .unwrap();
                let mapper = ChangeMapper::new(&dep_graph, Vec::new(), detector);
                assert_eq!(
                    mapper
                        .changed_packages(
                            HashSet::new(),
                            LockfileContents::Changed(b"invalid lockfile".to_vec()),
                        )
                        .unwrap(),
                    PackageChanges::All(AllPackageChangeReason::LockfileChangeDetectionFailed)
                );
            }

            let missing = PackageName::from("b");
            assert!(dep_graph.remove_package_info_for_test(&missing).is_some());
            assert_eq!(
                dep_graph
                    .changed_packages_from_lockfile(previous.as_ref())
                    .unwrap()
                    .into_iter()
                    .map(|change| change.package.name)
                    .collect::<Vec<_>>(),
                vec![missing]
            );

            dep_graph.remove_external_resolution_for_test();
            let detector =
                GlobalDepsPackageChangeMapper::new(&dep_graph, std::iter::empty::<&str>()).unwrap();
            let mapper = ChangeMapper::new(&dep_graph, Vec::new(), detector);
            assert_eq!(
                mapper
                    .changed_packages(HashSet::new(), LockfileContents::Changed(previous_contents),)
                    .unwrap(),
                PackageChanges::All(AllPackageChangeReason::LockfileChangeDetectionFailed)
            );

            let previous_dep = package_manager
                .read_lockfile(&root, &root_package_json)
                .unwrap();
            apply_patch(tempdir.path(), lockfile, root_patch);
            let root_graph = build_lockfile_aware_graph(&root, package_manager);
            let mut root_changed = root_graph
                .changed_packages_from_lockfile(previous_dep.as_ref())
                .unwrap();
            root_changed.sort_by(|a, b| a.package.name.cmp(&b.package.name));

            let root_changed_names = root_changed
                .iter()
                .map(|change| change.package.name.clone())
                .collect::<HashSet<_>>();
            assert!(
                root_changed_names.contains(&PackageName::Root)
                    && root_changed_names.contains(&PackageName::from("a"))
                    && root_changed_names.contains(&PackageName::from("b")),
                "{pm_name}: root lockfile change should affect all workspaces: \
                 {root_changed_names:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_single_package_is_depends_on_root() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson {
                name: Some(Spanned::new("my-package".to_owned())),
                ..Default::default()
            },
        )
        .with_package_discovery(MockDiscovery)
        .with_single_package_mode(true)
        .build()
        .await
        .unwrap();

        let closure =
            pkg_graph.transitive_closure(Some(&PackageNode::Workspace(PackageName::Root)));
        assert!(closure.contains(&PackageNode::Root));
        let result = pkg_graph.validate();
        assert!(result.is_ok(), "expected ok {result:?}");
    }

    #[tokio::test]
    async fn test_internal_dependencies_get_split_out() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(
                json!({ "name": "root", "dependencies": { "a": "workspace:*"} }),
            )
            .unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "a",
                    "dependencies": {
                        "b": "workspace:*"
                    }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({
                    "name": "b",
                    "dependencies": {
                        "c": "1.2.3",
                    }
                }))
                .unwrap(),
            );
            map
        }))
        .build()
        .await
        .unwrap();

        assert!(pkg_graph.validate().is_ok());
        let closure = pkg_graph.transitive_closure(Some(&PackageNode::Workspace("a".into())));
        assert_eq!(
            closure,
            [
                PackageNode::Root,
                PackageNode::Workspace("a".into()),
                PackageNode::Workspace("b".into())
            ]
            .iter()
            .collect::<HashSet<_>>()
        );
        let b_external: std::collections::BTreeMap<_, _> = pkg_graph
            .external_declarations(&PackageName::from("b"))
            .iter()
            .map(|declaration| {
                (
                    declaration.package_name().to_string(),
                    declaration.specifier().to_string(),
                )
            })
            .collect();

        let pkg_version = b_external.get("c").unwrap();
        assert_eq!(pkg_version, "1.2.3");
        let closure =
            pkg_graph.transitive_closure(Some(&PackageNode::Workspace(PackageName::Root)));
        assert_eq!(
            closure,
            [
                PackageNode::Root,
                PackageNode::Workspace(PackageName::Root),
                PackageNode::Workspace("a".into()),
                PackageNode::Workspace("b".into()),
            ]
            .iter()
            .collect::<HashSet<_>>()
        );
    }

    #[derive(Debug)]
    struct MockLockfile {}
    impl turborepo_lockfiles::Lockfile for MockLockfile {
        fn resolve_package(
            &self,
            _workspace_path: &str,
            name: &str,
            _version: &str,
        ) -> std::result::Result<Option<turborepo_lockfiles::Package>, turborepo_lockfiles::Error>
        {
            Ok(match name {
                "a" => Some(turborepo_lockfiles::Package::new("key:a", "1")),
                "b" => Some(turborepo_lockfiles::Package::new("key:b", "1")),
                "c" => Some(turborepo_lockfiles::Package::new("key:c", "1")),
                _ => None,
            })
        }

        fn all_dependencies(
            &self,
            key: &str,
        ) -> std::result::Result<
            Option<std::borrow::Cow<'_, BTreeMap<String, String>>>,
            turborepo_lockfiles::Error,
        > {
            match key {
                "key:a" => Ok(Some(std::borrow::Cow::Owned(
                    [("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ))),
                "key:b" => Ok(Some(std::borrow::Cow::Owned(
                    [("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ))),
                "key:c" => Ok(None),
                _ => Ok(None),
            }
        }

        fn subgraph(
            &self,
            _workspace_packages: &[String],
            _packages: &[String],
        ) -> std::result::Result<Box<dyn Lockfile>, turborepo_lockfiles::Error> {
            unreachable!("lockfile pruning not necessary for package graph construction")
        }

        fn encode(&self) -> std::result::Result<Vec<u8>, turborepo_lockfiles::Error> {
            unreachable!("lockfile encoding not necessary for package graph construction")
        }

        fn global_change(&self, _other: &dyn Lockfile) -> bool {
            unreachable!("global change detection not necessary for package graph construction")
        }

        fn turbo_version(&self) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn test_lockfile_traversal() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_components(&["package_a", "package.json"]),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": {
                        "a": "1"
                    }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_components(&["package_b", "package.json"]),
                PackageJson::from_value(json!({
                    "name": "bar",
                    "dependencies": {
                        "b": "1",
                    }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        assert!(pkg_graph.validate().is_ok());
        let foo = PackageName::from("foo");
        let bar = PackageName::from("bar");

        let a = turborepo_lockfiles::Package::new("key:a", "1");
        let _b = turborepo_lockfiles::Package::new("key:b", "1");
        let _c = turborepo_lockfiles::Package::new("key:c", "1");

        let identities = pkg_graph.external_package_identities();
        assert_eq!(
            identities
                .iter()
                .map(|identity| (identity.key(), identity.version()))
                .collect::<HashSet<_>>(),
            HashSet::from([("key:a", "1"), ("key:b", "1"), ("key:c", "1")])
        );

        let a_dependents = pkg_graph
            .internal_dependencies_for_external_dependency(&a)
            .expect("a should have dependents");
        assert!(a_dependents.contains(&PackageNode::Workspace(foo.clone())));
        assert!(!a_dependents.contains(&PackageNode::Workspace(bar.clone())));

        let c_dependents = pkg_graph
            .internal_dependencies_for_external_identity(&ExternalPackageIdentity::new(
                "key:c", "1",
            ))
            .expect("c should have dependents");
        assert!(c_dependents.contains(&PackageNode::Workspace(foo.clone())));
        assert!(c_dependents.contains(&PackageNode::Workspace(bar.clone())));

        let foo_identities = pkg_graph.external_package_identities_for_packages([&foo]);
        assert_eq!(
            foo_identities
                .iter()
                .map(|identity| (identity.key(), identity.version()))
                .collect::<Vec<_>>(),
            vec![("key:a", "1"), ("key:c", "1")]
        );
        let bar_identities = pkg_graph.external_package_identities_for_packages([&bar]);
        assert_eq!(
            bar_identities
                .iter()
                .map(|identity| identity.key())
                .collect::<HashSet<_>>(),
            HashSet::from(["key:b", "key:c"])
        );
    }

    #[tokio::test]
    async fn external_dependency_reverse_index_is_lazy_and_cached() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let mut package_jsons = HashMap::new();
        for index in 0..64 {
            package_jsons.insert(
                root.join_components(&[&format!("package_{index}"), "package.json"]),
                PackageJson::from_value(json!({
                    "name": format!("pkg-{index}"),
                    "dependencies": { "a": "1" }
                }))
                .unwrap(),
            );
        }
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some(package_jsons))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let start = std::time::Instant::now();
        let first = pkg_graph
            .internal_dependencies_for_external_dependency(&turborepo_lockfiles::Package::new(
                "key:a", "1",
            ))
            .expect("reverse index should resolve key:a");
        let first_elapsed = start.elapsed();
        assert!(first.len() >= 64);

        let start = std::time::Instant::now();
        let second = pkg_graph
            .internal_dependencies_for_external_dependency(&turborepo_lockfiles::Package::new(
                "key:a", "1",
            ))
            .expect("cached reverse index should resolve key:a");
        let second_elapsed = start.elapsed();

        assert!(std::ptr::eq(first, second));
        assert!(
            second_elapsed * 10 < first_elapsed.max(std::time::Duration::from_micros(50)),
            "cached reverse-index lookup should be much cheaper than the first build \
             (first={first_elapsed:?}, second={second_elapsed:?})"
        );
        assert!(
            first_elapsed < std::time::Duration::from_secs(2),
            "reverse-index build took too long: {first_elapsed:?}"
        );
    }

    #[test]
    fn external_package_identity_equality_ignores_human_name() {
        let left = ExternalPackageIdentity::new("key:a", "1").with_human_name("a@1");
        let right = ExternalPackageIdentity::new("key:a", "1");
        assert_eq!(left, right);
        assert_eq!(left.display_name(), "a@1");
        assert_eq!(right.display_name(), "key:a");
    }

    #[tokio::test]
    async fn javascript_resolution_distinguishes_resolved_empty_from_unavailable() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let root_package = || PackageJson::from_value(json!({ "name": "root" })).unwrap();

        let resolved = PackageGraph::builder(&root, root_package())
            .with_package_discovery(MockDiscovery)
            .with_lockfile(Some(Box::new(MockLockfile {})))
            .build()
            .await
            .unwrap();
        let resolved_generation = resolved
            .external_resolution_generation()
            .expect("resolved generation should be retained");
        let resolved_domain = &resolved_generation.domains()[0];
        assert_eq!(
            resolved_domain.definition_sources()[0].as_str(),
            "package-lock.json"
        );
        let crate::external_resolution::ExternalResolutionData::Resolved {
            completeness,
            fingerprint,
            packages,
        } = resolved_domain.data()
        else {
            panic!("expected resolved terminal data")
        };
        assert_eq!(
            completeness,
            &crate::external_resolution::ResolutionCompleteness::Complete
        );
        assert!(!fingerprint.as_str().is_empty());
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package(), ROOT_PKG_NAME);
        assert!(packages[0].identities().is_empty());
        assert!(
            resolved
                .external_package_identities_for_packages([&PackageName::Root])
                .is_empty()
        );

        let unavailable = PackageGraph::builder(&root, root_package())
            .with_package_discovery(MockDiscovery)
            .build()
            .await
            .unwrap();
        let unavailable_generation = unavailable
            .external_resolution_generation()
            .expect("unavailable generation should be retained");
        let crate::external_resolution::ExternalResolutionData::Unavailable(reason) =
            unavailable_generation.domains()[0].data()
        else {
            panic!("expected unavailable terminal data")
        };
        assert_eq!(reason.code(), "lockfile-unavailable");
        assert!(
            unavailable
                .external_resolution_global_file_fallback()
                .is_some()
        );

        let fallback = unavailable
            .external_resolution_global_file_fallback()
            .expect("unavailable JS resolution should expose a global file fallback");
        assert!(
            fallback
                .iter()
                .any(|path| path.file_name() == Some("package.json"))
        );
        assert!(
            resolved
                .external_resolution_global_file_fallback()
                .is_none(),
            "resolved domains must not use the global file fallback"
        );
    }

    #[tokio::test]
    async fn test_circular_dependency() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": {
                        "bar": "*"
                    }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({
                    "name": "bar",
                    "dependencies": {
                        "baz": "*",
                    }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_c"),
                PackageJson::from_value(json!({
                    "name": "baz",
                    "dependencies": {
                        "foo": "*",
                    }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        // Package graph cycles are intentionally allowed (#2559) — only task
        // graph cycles block execution (checked in the engine builder).
        assert!(pkg_graph.validate().is_ok());

        let foo_node = PackageNode::Workspace("foo".into());
        let bar_node = PackageNode::Workspace("bar".into());
        let baz_node = PackageNode::Workspace("baz".into());

        // transitive_closure starting from any cycle member includes all members
        let closure = pkg_graph.transitive_closure(Some(&foo_node));
        assert!(
            closure.contains(&foo_node)
                && closure.contains(&bar_node)
                && closure.contains(&baz_node),
            "transitive_closure on a cycle member should include all cycle members: {closure:?}"
        );

        // dependencies of a cycle member includes the other cycle members
        let deps = pkg_graph.dependencies(&foo_node);
        assert!(
            deps.contains(&bar_node) && deps.contains(&baz_node),
            "dependencies on a cycle member should include other cycle members: {deps:?}"
        );

        // ancestors of a cycle member includes the other cycle members
        let anc = pkg_graph.ancestors(&foo_node);
        assert!(
            anc.contains(&bar_node) && anc.contains(&baz_node),
            "ancestors on a cycle member should include other cycle members: {anc:?}"
        );
    }

    #[tokio::test]
    async fn test_self_dependency() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": {
                        "foo": "*"
                    }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        // Package graph self-dependencies are intentionally allowed (#2559) —
        // if this causes a task-level cycle it will be caught by the engine
        // builder.
        assert!(pkg_graph.validate().is_ok());

        let foo_node = PackageNode::Workspace("foo".into());

        // Self-dep doesn't cause infinite loops in traversal methods
        let closure = pkg_graph.transitive_closure(Some(&foo_node));
        assert!(
            closure.contains(&foo_node),
            "transitive_closure on self-dep should include the package itself: {closure:?}"
        );

        let deps = pkg_graph.dependencies(&foo_node);
        assert!(
            !deps.contains(&foo_node),
            "dependencies() excludes the node itself: {deps:?}"
        );

        let anc = pkg_graph.ancestors(&foo_node);
        assert!(
            !anc.contains(&foo_node),
            "ancestors() excludes the node itself: {anc:?}"
        );
    }

    #[tokio::test]
    async fn test_find_cycles_simple() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": { "bar": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({
                    "name": "bar",
                    "dependencies": { "baz": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_c"),
                PackageJson::from_value(json!({
                    "name": "baz",
                    "dependencies": { "foo": "*" }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let cycles = pkg_graph.find_cycles();
        assert_eq!(cycles.len(), 1, "expected exactly one cycle: {cycles:?}");

        let cycle = &cycles[0];
        assert_eq!(cycle.len(), 3, "cycle should contain 3 packages: {cycle:?}");
        // Rotated so lexicographically smallest name comes first
        assert_eq!(cycle[0], PackageName::from("bar"));
        let members: HashSet<_> = cycle.iter().collect();
        assert!(members.contains(&PackageName::from("foo")));
        assert!(members.contains(&PackageName::from("bar")));
        assert!(members.contains(&PackageName::from("baz")));
    }

    #[tokio::test]
    async fn test_find_cycles_two_independent() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            // Cycle 1: a -> b -> a
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "a",
                    "dependencies": { "b": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({
                    "name": "b",
                    "dependencies": { "a": "*" }
                }))
                .unwrap(),
            );
            // Cycle 2: x -> y -> x
            map.insert(
                root.join_component("package_x"),
                PackageJson::from_value(json!({
                    "name": "x",
                    "dependencies": { "y": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_y"),
                PackageJson::from_value(json!({
                    "name": "y",
                    "dependencies": { "x": "*" }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let cycles = pkg_graph.find_cycles();
        assert_eq!(
            cycles.len(),
            2,
            "expected two independent cycles: {cycles:?}"
        );

        // Sorted by first element: "a" < "x"
        let first_members: HashSet<_> = cycles[0].iter().collect();
        assert!(first_members.contains(&PackageName::from("a")));
        assert!(first_members.contains(&PackageName::from("b")));

        let second_members: HashSet<_> = cycles[1].iter().collect();
        assert!(second_members.contains(&PackageName::from("x")));
        assert!(second_members.contains(&PackageName::from("y")));
    }

    #[tokio::test]
    async fn test_find_cycles_self_dep_excluded() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": { "foo": "*" }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let cycles = pkg_graph.find_cycles();
        assert!(
            cycles.is_empty(),
            "self-dependency should not produce a cycle: {cycles:?}"
        );
    }

    #[tokio::test]
    async fn test_find_cycles_no_cycles() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "a",
                    "dependencies": { "b": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({ "name": "b" })).unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let cycles = pkg_graph.find_cycles();
        assert!(
            cycles.is_empty(),
            "acyclic graph should produce no cycles: {cycles:?}"
        );
    }

    #[tokio::test]
    async fn test_find_cycles_complex_scc() {
        // a -> b -> c -> a and b -> d -> c creates one large SCC {a, b, c, d}
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "a",
                    "dependencies": { "b": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
                PackageJson::from_value(json!({
                    "name": "b",
                    "dependencies": { "c": "*", "d": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_c"),
                PackageJson::from_value(json!({
                    "name": "c",
                    "dependencies": { "a": "*" }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_d"),
                PackageJson::from_value(json!({
                    "name": "d",
                    "dependencies": { "c": "*" }
                }))
                .unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .build()
        .await
        .unwrap();

        let cycles = pkg_graph.find_cycles();
        assert_eq!(
            cycles.len(),
            1,
            "overlapping cycles should form one SCC: {cycles:?}"
        );

        // The traced path covers a representative cycle within the SCC.
        // It must contain at least 2 members (it's a cycle) and all members
        // must be from the SCC.
        let all_scc_members: HashSet<PackageName> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| PackageName::from(*s))
            .collect();
        let traced: HashSet<_> = cycles[0].iter().cloned().collect();
        assert!(
            traced.len() >= 2,
            "traced cycle should have at least 2 members: {traced:?}"
        );
        assert!(
            traced.is_subset(&all_scc_members),
            "all traced members should be in the SCC: {traced:?}"
        );
        // First element is lexicographic min of traced members
        let min_traced = traced.iter().min().unwrap();
        assert_eq!(&cycles[0][0], min_traced);
    }

    fn write_cargo_workspace_fixture(root: &AbsoluteSystemPathBuf) {
        let write = |rel: &[&str], contents: &str| {
            let path = root.join_components(rel);
            std::fs::create_dir_all(path.parent().unwrap().as_std_path()).unwrap();
            std::fs::write(path.as_std_path(), contents).unwrap();
        };
        write(
            &["Cargo.toml"],
            "[workspace]\nmembers = [\"rust/*\"]\nresolver = \"2\"\n\n[workspace.metadata]\nname \
             = \"acme\"\n",
        );
        write(
            &["rust", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
        );
        write(&["rust", "app", "src", "main.rs"], "fn main() {}\n");
        write(
            &["rust", "lib-a", "Cargo.toml"],
            "[package]\nname = \"lib-a\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-b = { path = \"../lib-b\" }\n",
        );
        write(&["rust", "lib-a", "src", "lib.rs"], "");
        write(
            &["rust", "lib-b", "Cargo.toml"],
            "[package]\nname = \"lib-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&["rust", "lib-b", "src", "lib.rs"], "");
        write(
            &["Cargo.lock"],
            r#"version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["lib-a"]

[[package]]
name = "lib-a"
version = "0.1.0"
dependencies = ["lib-b"]

[[package]]
name = "lib-b"
version = "0.1.0"
"#,
        );
    }

    fn canonical_tempdir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        // dunce: `cargo metadata` reports plain (non-verbatim) paths on
        // Windows, so the fixture root must be plain too.
        let root = AbsoluteSystemPathBuf::new(
            dunce::canonicalize(tmp.path())
                .unwrap()
                .to_string_lossy()
                .to_string(),
        )
        .unwrap();
        (tmp, root)
    }

    /// Crates from a registered Cargo toolchain join the graph alongside JS
    /// packages: crate->crate edges come from Cargo path dependencies, the
    /// synthetic `cargo` workspace package depends on every crate, and the
    /// JS lockfile closure of the root package is untouched by the Cargo
    /// packages (the workspace package shares the repo-root directory with
    /// the root package, which previously made closure attribution
    /// nondeterministic).
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_toolchain_packages_in_graph() {
        let (_tmp, root) = canonical_tempdir();
        write_cargo_workspace_fixture(&root);

        // Several iterations: the closure-attribution regression this guards
        // was decided by HashMap iteration order, roughly a coin flip per
        // process/build.
        let mut expected_cargo_fingerprint = None;
        for iteration in 0..8 {
            let _defer_resolution = iteration % 2 == 1;
            let mut pkg_graph = PackageGraph::builder(
                &root,
                PackageJson::from_value(json!({ "name": "root", "dependencies": { "a": "1" } }))
                    .unwrap(),
            )
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some({
                let mut map = HashMap::new();
                map.insert(
                    root.join_components(&["js-pkg", "package.json"]),
                    PackageJson::from_value(json!({ "name": "js-pkg" })).unwrap(),
                );
                map
            }))
            .with_lockfile(Some(Box::new(MockLockfile {})))
            .with_toolchain(crate::cargo::CargoToolchain::new(root.clone()))
            .build()
            .await
            .unwrap();

            assert!(pkg_graph.validate().is_ok());
            assert!(pkg_graph.package_task_contexts().all(|context| {
                context.requires_compatibility_payload() && context.package_info().is_some()
            }));

            let app_name = PackageName::from("app");
            assert!(pkg_graph.set_compatibility_manifest_name_for_test(
                &app_name,
                Some("stale-payload-name".into())
            ));
            assert_eq!(
                pkg_graph.package_task_context(&app_name).unwrap().package(),
                &app_name
            );
            assert!(
                pkg_graph
                    .package_task_context(&PackageName::from("stale-payload-name"))
                    .is_none()
            );

            // All packages are present with knowledge-backed provenance.
            assert_eq!(
                pkg_graph.package_toolchain(&PackageName::from("app")),
                Some(&crate::toolchain::ToolchainId::RUST)
            );
            assert_eq!(
                pkg_graph.package_toolchain(&PackageName::from("js-pkg")),
                Some(&crate::toolchain::ToolchainId::JAVASCRIPT)
            );
            let _workspace_pkg = pkg_graph.package_info(&PackageName::from("acme")).unwrap();
            assert_eq!(
                pkg_graph.package_toolchain(&PackageName::from("acme")),
                Some(&crate::toolchain::ToolchainId::RUST)
            );

            let knowledge = pkg_graph.repository_knowledge();
            let cargo_app = knowledge.scope("app").expect("Cargo crate is a package");
            assert_eq!(
                cargo_app.definition_path().to_unix().as_str(),
                "rust/app/Cargo.toml"
            );
            assert_eq!(cargo_app.toolchain(), &crate::toolchain::ToolchainId::RUST);
            assert_eq!(cargo_app.kind(), crate::knowledge::ScopeKind::Package);
            let cargo_app_view = pkg_graph
                .package_view(&PackageName::from("app"))
                .expect("Cargo contributes a real package");
            assert_eq!(cargo_app_view.kind(), PackageGraphNodeKind::Package);
            assert_eq!(
                cargo_app_view
                    .directory()
                    .map(|path| path.to_unix().to_string()),
                Some("rust/app".to_string())
            );
            assert_eq!(
                cargo_app_view
                    .definition_path()
                    .map(|path| path.to_unix().to_string()),
                Some("rust/app/Cargo.toml".to_string())
            );
            assert_eq!(
                cargo_app_view.toolchain(),
                Some(&crate::toolchain::ToolchainId::RUST)
            );
            let cargo_workspace = knowledge
                .scope("acme")
                .expect("Cargo workspace compatibility package is an aggregate scope");
            assert_eq!(
                cargo_workspace.kind(),
                crate::knowledge::ScopeKind::Aggregate
            );
            assert_eq!(cargo_workspace.directory().to_unix().as_str(), "");
            assert_eq!(
                cargo_workspace.definition_path().to_unix().as_str(),
                "Cargo.toml"
            );
            assert_eq!(
                pkg_graph.package_dir(&PackageName::from("acme")),
                Some(cargo_workspace.directory())
            );
            let cargo_workspace_view = pkg_graph
                .node_view(&PackageNode::Workspace(PackageName::from("acme")))
                .expect("Cargo contributes an aggregate execution scope");
            assert_eq!(cargo_workspace_view.kind(), PackageGraphNodeKind::Aggregate);
            assert_eq!(
                cargo_workspace_view
                    .definition_path()
                    .map(|path| path.to_unix().to_string()),
                Some("Cargo.toml".to_string())
            );
            assert_eq!(
                cargo_workspace_view.toolchain(),
                Some(&crate::toolchain::ToolchainId::RUST)
            );

            // Crate path dependencies became graph edges.
            let app_deps = pkg_graph
                .immediate_dependencies(&PackageNode::Workspace(PackageName::from("app")))
                .unwrap();
            assert!(
                app_deps.contains(&PackageNode::Workspace(PackageName::from("lib-a"))),
                "app should depend on lib-a, got {app_deps:?}"
            );
            assert_eq!(
                pkg_graph
                    .ordering_relationships()
                    .direct_dependencies(&PackageName::from("app"))
                    .map(|dependencies| dependencies.cloned().collect::<Vec<_>>()),
                Some(vec![PackageName::from("lib-a")])
            );
            assert_eq!(
                pkg_graph
                    .hash_relationships()
                    .dependency_inputs(&PackageName::from("app")),
                Some(vec![PackageName::from("lib-a"), PackageName::from("lib-b")]),
                "mixed repositories retain transitive native hash inputs"
            );
            let workspace_deps = pkg_graph
                .immediate_dependencies(&PackageNode::Workspace(PackageName::from("acme")))
                .unwrap();
            assert!(
                workspace_deps.contains(&PackageNode::Workspace(PackageName::from("app")))
                    && workspace_deps.contains(&PackageNode::Workspace(PackageName::from("lib-a"))),
                "workspace package should depend on every crate, got {workspace_deps:?}"
            );

            // The root's JS resolution identities are attributed to the root
            // package, not stolen by the Cargo workspace package sharing its
            // directory.
            let root_identities =
                pkg_graph.external_package_identities_for_packages([&PackageName::Root]);
            assert_eq!(
                root_identities
                    .iter()
                    .map(|identity| (identity.key(), identity.version()))
                    .collect::<Vec<_>>(),
                vec![("key:a", "1"), ("key:c", "1")],
            );
            // Cargo packages contribute identities through the Rust resolution
            // domain, never JS lockfile entries.

            let resolution = pkg_graph
                .external_resolution_generation()
                .expect("mixed repository should retain external resolution knowledge");
            assert_eq!(resolution.domains().len(), 2);
            let cargo_domain = resolution
                .domains()
                .iter()
                .find(|domain| domain.toolchain() == &crate::toolchain::ToolchainId::RUST)
                .expect("Cargo should contribute one resolution domain");
            assert_eq!(cargo_domain.definition_sources()[0].as_str(), "Cargo.lock");
            let crate::external_resolution::ExternalResolutionData::Resolved {
                completeness,
                fingerprint,
                packages,
            } = cargo_domain.data()
            else {
                panic!("Cargo resolution must be available")
            };
            assert_eq!(
                completeness,
                &crate::external_resolution::ResolutionCompleteness::Complete
            );
            if let Some(expected) = &expected_cargo_fingerprint {
                assert_eq!(fingerprint, expected);
            } else {
                expected_cargo_fingerprint = Some(fingerprint.clone());
            }
            assert_eq!(packages.len(), 4);
            for package in packages {
                assert!(
                    package
                        .identities()
                        .iter()
                        .any(|identity| identity.key() == "rustc"),
                    "{} must include the compiler identity",
                    package.package()
                );
                assert!(
                    package
                        .identities()
                        .iter()
                        .all(|identity| identity.key() == "rustc"),
                    "{} should only carry the compiler identity in this fixture, got {:?}",
                    package.package(),
                    package.identities()
                );
            }
        }
    }

    /// A pure Cargo workspace has no root package.json and no JavaScript
    /// package manager: the graph is built entirely from the Cargo toolchain,
    /// and both `package_manager()` and `root_package_json()` report `None`.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pure_cargo_workspace_has_no_javascript() {
        let (_tmp, root) = canonical_tempdir();
        write_cargo_workspace_fixture(&root);

        let mut pkg_graph = PackageGraph::builder_optional(&root, None)
            .with_toolchain(crate::cargo::CargoToolchain::new(root.clone()))
            .build()
            .await
            .unwrap();

        assert!(pkg_graph.validate().is_ok());
        let resolution = pkg_graph
            .external_resolution_generation()
            .expect("pure Cargo repository should retain external resolution knowledge");
        assert_eq!(resolution.domains().len(), 1);
        assert_eq!(
            resolution.domains()[0].toolchain(),
            &crate::toolchain::ToolchainId::RUST
        );
        assert!(
            pkg_graph.relationship_projections.get().is_none(),
            "relationship projections must remain lazy for current consumers"
        );

        assert_eq!(
            pkg_graph
                .ordering_relationships()
                .direct_dependencies(&PackageName::Root)
                .map(|dependencies| dependencies.cloned().collect::<Vec<_>>()),
            Some(Vec::new()),
            "pure Cargo recognizes the root Turbo namespace without JS edges"
        );
        assert!(pkg_graph.relationship_projections.get().is_some());
        let app = PackageName::from("app");
        let expected_dependencies = vec![PackageName::from("lib-a"), PackageName::from("lib-b")];
        assert_eq!(
            pkg_graph.hash_relationships().dependency_inputs(&app),
            Some(expected_dependencies.clone()),
            "Cargo hash inputs include transitive path dependencies"
        );
        assert_eq!(
            pkg_graph
                .filtering_relationships()
                .transitive_dependencies(&app),
            Some(expected_dependencies.clone())
        );
        let mut graph_dependencies: Vec<_> = pkg_graph
            .dependencies(&PackageNode::Workspace(app.clone()))
            .into_iter()
            .filter_map(|node| match node {
                PackageNode::Workspace(name) if name != &app => Some(name.clone()),
                PackageNode::Root | PackageNode::Workspace(_) => None,
            })
            .collect();
        graph_dependencies.sort();
        assert_eq!(graph_dependencies, expected_dependencies);
        assert_eq!(
            pkg_graph.prune_relationships().package_closure(
                std::slice::from_ref(&app),
                PruneDependencyMode::ProductionOnly,
            ),
            Ok(vec![
                PackageName::Root,
                PackageName::from("app"),
                PackageName::from("lib-a"),
                PackageName::from("lib-b"),
            ])
        );

        assert!(
            pkg_graph
                .repository_knowledge()
                .root_javascript_scope()
                .is_none(),
            "a pure Cargo repository has no root JavaScript execution scope"
        );
        assert!(!pkg_graph.has_root_javascript_scope());
        let root_context = pkg_graph
            .package_task_context(&PackageName::Root)
            .expect("pure Cargo still has a root Turbo task namespace");
        assert_eq!(root_context.package(), &PackageName::Root);
        assert_eq!(
            root_context.directory(),
            pkg_graph.repository_knowledge().repository_directory()
        );
        assert_eq!(root_context.kind(), PackageTaskContextKind::Root);
        assert_eq!(root_context.toolchain(), None);
        assert!(root_context.package_info().is_none());
        assert!(pkg_graph.package_info(&PackageName::Root).is_none());
        assert_eq!(pkg_graph.package_definition_path(&PackageName::Root), None);
        assert_eq!(pkg_graph.package_toolchain(&PackageName::Root), None);
        assert!(
            pkg_graph
                .node_view(&PackageNode::Workspace(PackageName::Root))
                .is_none(),
            "the compatibility root workspace is not a JavaScript scope"
        );
        assert_eq!(
            pkg_graph
                .node_view(&PackageNode::Root)
                .map(|view| view.kind()),
            Some(PackageGraphNodeKind::GraphSentinel)
        );
        assert!(
            pkg_graph
                .node_views()
                .all(|(_, view)| view.kind() != PackageGraphNodeKind::RootJavaScript)
        );
        let scope_directories = pkg_graph
            .package_scope_directories()
            .map(|(name, directory)| (name, directory.to_owned()))
            .collect::<BTreeMap<_, _>>();
        assert!(!scope_directories.contains_key(&PackageName::Root));
        assert_eq!(
            scope_directories.get(&PackageName::from("acme")),
            Some(&AnchoredSystemPathBuf::from_raw("").unwrap())
        );
        assert!(
            pkg_graph
                .repository_knowledge()
                .scope("acme")
                .is_some_and(|scope| scope.kind() == crate::knowledge::ScopeKind::Aggregate),
            "existing workspace-scoped Cargo behavior is represented as an aggregate"
        );

        // No JavaScript project: no package manager, no root manifest.
        assert!(
            pkg_graph.package_manager().is_none(),
            "a pure Cargo workspace has no JavaScript package manager"
        );
        assert!(
            pkg_graph.root_package_json().is_none(),
            "a pure Cargo workspace has no root package.json"
        );

        // The Cargo crates and synthetic workspace package populate the graph.
        assert_eq!(
            pkg_graph.package_toolchain(&PackageName::from("app")),
            Some(&crate::toolchain::ToolchainId::RUST)
        );
        assert_eq!(
            pkg_graph.package_toolchain(&PackageName::from("lib-a")),
            Some(&crate::toolchain::ToolchainId::RUST)
        );
        assert_eq!(
            pkg_graph.package_toolchain(&PackageName::from("acme")),
            Some(&crate::toolchain::ToolchainId::RUST)
        );

        // Command resolution receives identity, path, and provenance from the
        // graph-created context.
        let app_name = PackageName::from("app");
        let app_context = pkg_graph.package_task_context(&app_name).unwrap();
        assert_eq!(app_context.package(), &app_name);
        assert_eq!(
            app_context.directory(),
            AnchoredSystemPath::new("rust/app").unwrap()
        );
        assert_eq!(
            app_context.toolchain(),
            Some(&crate::toolchain::ToolchainId::RUST)
        );
        let command = pkg_graph
            .toolchains()
            .get(app_context.toolchain().unwrap())
            .unwrap()
            .task_command(&app_context, "build", None, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            command.args,
            vec![
                std::ffi::OsString::from("build"),
                std::ffi::OsString::from("--package=app"),
                std::ffi::OsString::from("--locked"),
            ]
        );
        assert_eq!(command.cwd, root);

        // Crate path dependencies still become graph edges without a package
        // manager: the `workspace:*` protocol resolves internally regardless.
        let app_deps = pkg_graph
            .immediate_dependencies(&PackageNode::Workspace(PackageName::from("app")))
            .unwrap();
        assert!(
            app_deps.contains(&PackageNode::Workspace(PackageName::from("lib-a"))),
            "app should depend on lib-a, got {app_deps:?}"
        );

        assert!(
            pkg_graph
                .remove_package_info_for_test(&PackageName::Root)
                .is_none()
        );
        assert!(pkg_graph.remove_package_info_for_test(&app_name).is_some());
        let contexts = pkg_graph.package_task_contexts().collect::<Vec<_>>();
        assert_eq!(
            contexts.first().map(|context| context.package()),
            Some(&PackageName::Root)
        );
        assert_eq!(
            contexts
                .iter()
                .filter(|context| context.package() == &PackageName::Root)
                .count(),
            1
        );
        assert!(contexts.iter().any(|context| {
            context.package() == &app_name && context.kind() == PackageTaskContextKind::Package
        }));
        assert!(contexts.iter().any(|context| {
            context.package() == &PackageName::from("acme")
                && context.kind() == PackageTaskContextKind::Aggregate
        }));
        for enumerated in &contexts {
            let point = pkg_graph
                .package_task_context(enumerated.package())
                .expect("enumerated authoritative name must support point lookup");
            assert_eq!(enumerated.package(), point.package());
            assert_eq!(enumerated.repository_root(), point.repository_root());
            assert_eq!(enumerated.directory(), point.directory());
            assert_eq!(enumerated.kind(), point.kind());
            assert_eq!(enumerated.toolchain(), point.toolchain());
            assert_eq!(
                enumerated.requires_compatibility_payload(),
                point.requires_compatibility_payload()
            );
            assert_eq!(
                enumerated.package_info().map(std::ptr::from_ref),
                point.package_info().map(std::ptr::from_ref)
            );
        }
        assert!(
            contexts
                .iter()
                .find(|context| context.package() == &app_name)
                .is_some_and(|context| context.package_info().is_none()),
            "removing compatibility payload must not remove the package namespace"
        );
        let root_without_payload = pkg_graph
            .package_task_context(&PackageName::Root)
            .expect("root context is independent of compatibility payload");
        assert!(root_without_payload.package_info().is_none());
        assert_eq!(root_without_payload.repository_root(), root.as_ref());
    }

    /// A crate and a JS package sharing a name is a hard error, like any
    /// other duplicate package name.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cargo_js_name_collision_hard_errors() {
        let (_tmp, root) = canonical_tempdir();
        write_cargo_workspace_fixture(&root);

        let result = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_discovery(MockDiscovery)
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_components(&["js-app", "package.json"]),
                PackageJson::from_value(json!({ "name": "app" })).unwrap(),
            );
            map
        }))
        .with_lockfile(Some(Box::new(MockLockfile {})))
        .with_toolchain(crate::cargo::CargoToolchain::new(root.clone()))
        .build()
        .await;

        let err = result.expect_err("cross-toolchain name collision must error");
        assert!(
            err.to_string().contains("app"),
            "error should name the colliding package: {err}"
        );
    }

    #[tokio::test]
    async fn test_does_not_require_name_for_root_package_json() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(&root, PackageJson::from_value(json!({})).unwrap())
            .with_package_discovery(MockDiscovery)
            .build()
            .await
            .unwrap();

        assert!(pkg_graph.validate().is_ok());
    }
}
