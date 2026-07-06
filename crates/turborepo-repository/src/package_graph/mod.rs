use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::OnceLock,
};

use itertools::Itertools;
use petgraph::{
    graph::{Edge, NodeIndex},
    visit::EdgeRef,
};
use serde::Serialize;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_lockfiles::Lockfile;

use crate::{
    discovery::LocalPackageDiscoveryBuilder, package_json::PackageJson,
    package_manager::PackageManager,
};

pub mod builder;
mod dep_splitter;

pub use builder::{Error, PackageGraphBuilder};

pub use crate::package_json::DependencyKind;

pub const ROOT_PKG_NAME: &str = "//";

#[derive(Debug)]
pub struct PackageGraph {
    graph: petgraph::Graph<PackageNode, DependencyKind>,
    root_node_index: NodeIndex,
    root_workspace_index: NodeIndex,
    #[allow(dead_code)]
    node_lookup: HashMap<PackageNode, petgraph::graph::NodeIndex>,
    packages: HashMap<PackageName, PackageInfo>,
    root_package_json: PackageJson,
    package_manager: PackageManager,
    lockfile: Option<Box<dyn Lockfile>>,
    repo_root: AbsoluteSystemPathBuf,
    external_dep_to_internal_dependents:
        OnceLock<HashMap<turborepo_lockfiles::Package, HashSet<PackageNode>>>,
    /// Lazily computed internal dependencies of the root package. They are
    /// implied dependencies of every package, so per-package operations like
    /// `dependencies` and `ancestors` consult them on every call; the set is
    /// invariant once the graph is built.
    root_internal_dependencies: OnceLock<HashSet<PackageNode>>,
    /// The toolchains that contributed packages to this graph. The single
    /// lookup path for toolchain concerns after graph construction (command
    /// resolution, summaries).
    toolchains: crate::toolchain::ToolchainRegistry,
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

/// PackageInfo represents a package within the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageInfo {
    /// The toolchain-neutral package descriptor (see
    /// [`crate::toolchain::DiscoveredPackage`]). For JavaScript packages
    /// this is the parsed `package.json`; other toolchains synthesize one
    /// from their native manifest.
    pub package_json: PackageJson,
    /// Path to the package's native manifest, anchored to the repo root.
    pub package_json_path: AnchoredSystemPathBuf,
    pub unresolved_external_dependencies: Option<BTreeMap<PackageKey, PackageVersion>>, /* name -> version */
    pub transitive_dependencies: Option<HashSet<turborepo_lockfiles::Package>>,
    /// The toolchain that discovered this package. Defaults to JavaScript.
    pub toolchain: crate::toolchain::ToolchainId,
}

impl PackageInfo {
    pub fn package_name(&self) -> Option<String> {
        self.package_json
            .name
            .as_ref()
            .map(|name| name.as_inner().clone())
    }

    pub fn package_json_path(&self) -> &AnchoredSystemPath {
        &self.package_json_path
    }

    /// Get the path to this package.
    ///
    /// note: This is infallible because `package_json_path` is guaranteed to
    /// have       at least one segment
    pub fn package_path(&self) -> &AnchoredSystemPath {
        self.package_json_path
            .parent()
            .expect("at least one segment")
    }
}

type PackageKey = String;
type PackageVersion = String;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalDependencyChange {
    pub package: WorkspacePackage,
    /// Dependencies that were added to the package
    pub added: Vec<turborepo_lockfiles::Package>,
    /// Dependencies that were removed from the package
    pub removed: Vec<turborepo_lockfiles::Package>,
}

impl PackageGraph {
    pub fn builder(
        repo_root: &AbsoluteSystemPath,
        root_package_json: PackageJson,
    ) -> PackageGraphBuilder<'_, LocalPackageDiscoveryBuilder> {
        PackageGraphBuilder::new(repo_root, root_package_json)
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
        for (package_name, info) in self.packages.iter() {
            if matches!(package_name, PackageName::Root) {
                continue;
            }
            let name = info.package_json.name.as_ref().map(|name| name.as_str());
            match name {
                Some("") | None => {
                    let package_json_path = self.repo_root.resolve(info.package_json_path());
                    return Err(Error::PackageJsonMissingName(package_json_path));
                }
                Some(_) => continue,
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
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn package_manager(&self) -> &PackageManager {
        &self.package_manager
    }

    /// The toolchains that contributed packages to this graph.
    pub fn toolchains(&self) -> &crate::toolchain::ToolchainRegistry {
        &self.toolchains
    }

    pub fn repo_root(&self) -> &AbsoluteSystemPath {
        &self.repo_root
    }

    pub fn lockfile(&self) -> Option<&dyn Lockfile> {
        self.lockfile.as_deref()
    }

    pub fn package_json(&self, package: &PackageName) -> Option<&PackageJson> {
        let entry = self.packages.get(package)?;
        Some(&entry.package_json)
    }

    pub fn package_dir(&self, package: &PackageName) -> Option<&AnchoredSystemPath> {
        let entry = self.packages.get(package)?;
        Some(
            entry
                .package_json_path()
                .parent()
                .unwrap_or_else(|| AnchoredSystemPath::new("").unwrap()),
        )
    }

    pub fn package_info(&self, package: &PackageName) -> Option<&PackageInfo> {
        self.packages.get(package)
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

    /// Like [`Self::immediate_dependencies`], but includes the dependency kind
    /// for each outgoing edge.
    pub fn immediate_dependencies_with_kinds(
        &self,
        package: &PackageNode,
    ) -> Option<HashMap<&PackageNode, DependencyKind>> {
        let index = self.node_lookup.get(package)?;
        Some(
            self.graph
                .edges(*index)
                .map(|edge| {
                    let target = self
                        .graph
                        .node_weight(edge.target())
                        .expect("node index from neighbors should be present");
                    (target, *edge.weight())
                })
                .collect(),
        )
    }

    pub fn packages(&self) -> impl Iterator<Item = (&PackageName, &PackageInfo)> {
        self.packages.iter()
    }

    pub fn get_page_rank(&self) -> Vec<f64> {
        petgraph::algo::page_rank::page_rank(&self.graph, 0.85, 1)
    }

    pub fn root_package_json(&self) -> &PackageJson {
        &self.root_package_json
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
        let index = self.node_lookup.get(package)?;
        Some(
            self.graph
                .neighbors_directed(*index, petgraph::Outgoing)
                .map(|index| {
                    self.graph
                        .node_weight(index)
                        .expect("node index from neighbors should be present")
                })
                .collect(),
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

    pub fn transitive_external_dependencies<'a, I: IntoIterator<Item = &'a PackageName>>(
        &self,
        packages: I,
    ) -> HashSet<&turborepo_lockfiles::Package> {
        packages
            .into_iter()
            .filter_map(|package| self.packages.get(package))
            .filter_map(|entry| entry.transitive_dependencies.as_ref())
            .flatten()
            .collect()
    }

    /// Returns a list of changed packages based on the contents of a previous
    /// `Lockfile`. This assumes that none of the package.json in the package
    /// change, it is the responsibility of the caller to verify this.
    pub fn changed_packages_from_lockfile(
        &self,
        previous: &dyn Lockfile,
    ) -> Result<Vec<ExternalDependencyChange>, ChangedPackagesError> {
        let current = self.lockfile().ok_or(ChangedPackagesError::NoLockfile)?;

        let external_deps = self
            .packages()
            .filter_map(|(_name, info)| {
                info.unresolved_external_dependencies.as_ref().map(|dep| {
                    (
                        info.package_path().to_unix().to_string(),
                        dep.iter()
                            .map(|(name, version)| (name.to_owned(), version.to_owned()))
                            .collect(),
                    )
                })
            })
            .collect::<HashMap<_, BTreeMap<_, _>>>();

        // We're comparing to a previous lockfile, it's possible that a package was
        // added and thus won't exist in the previous lockfile. In that case,
        // we're fine to ignore it. Assuming there is not a commit with a stale
        // lockfile, the same commit should add the package, so it will get
        // picked up as changed.
        let closures = turborepo_lockfiles::all_transitive_closures(previous, external_deps, true)?;

        let global_change = current.global_change(previous);

        let changed = if global_change {
            None
        } else {
            self.packages
                .iter()
                .filter_map(|(name, info)| {
                    let previous_closure = closures.get(info.package_path().to_unix().as_str());
                    let not_equal = previous_closure != info.transitive_dependencies.as_ref();
                    if not_equal {
                        if let (Some(prev), Some(curr)) =
                            (previous_closure, info.transitive_dependencies.as_ref())
                        {
                            debug!(
                                "package {name} has differing closure: {:?}",
                                prev.symmetric_difference(curr)
                            );
                        }
                        let empty_set = HashSet::default();
                        let prev_deps = previous_closure.unwrap_or(&empty_set);
                        let curr_deps = info.transitive_dependencies.as_ref().unwrap_or(&empty_set);
                        // {a, b} -> {a, c}
                        // b was removed
                        // c was added
                        let added = curr_deps
                            .difference(prev_deps)
                            .cloned()
                            .sorted()
                            .collect::<Vec<_>>();
                        let removed = prev_deps
                            .difference(curr_deps)
                            .cloned()
                            .sorted()
                            .collect::<Vec<_>>();
                        Some((name, info, added, removed))
                    } else {
                        None
                    }
                })
                .map(|(name, info, added, removed)| match name {
                    PackageName::Other(n) => {
                        let w_name = PackageName::Other(n.to_owned());
                        let package = WorkspacePackage {
                            name: w_name.clone(),
                            path: info.package_path().to_owned(),
                        };
                        Some(ExternalDependencyChange {
                            package,
                            added,
                            removed,
                        })
                    }
                    // if the root package has changed, then we should report `None`
                    // since all packages need to be revalidated
                    PackageName::Root => None,
                })
                .collect::<Option<Vec<_>>>()
        };

        Ok(changed.unwrap_or_else(|| {
            self.packages
                .iter()
                .map(|(name, info)| {
                    let package = WorkspacePackage {
                        name: name.clone(),
                        path: info.package_path().to_owned(),
                    };
                    ExternalDependencyChange {
                        package,
                        added: Vec::new(),
                        removed: Vec::new(),
                    }
                })
                .collect()
        }))
    }

    pub fn internal_dependencies_for_external_dependency(
        &self,
        external_package: &turborepo_lockfiles::Package,
    ) -> Option<&HashSet<PackageNode>> {
        // In order to answer this once we have to calculate the info for every external
        // package so we store the results
        let map = self
            .external_dep_to_internal_dependents
            .get_or_init(|| self.build_external_dep_to_internal_dependents_map());
        map.get(external_package)
    }

    /// Builds a map from external dependencies to the set of internal workspace
    /// packages that depend on them (including transitive dependents).
    fn build_external_dep_to_internal_dependents_map(
        &self,
    ) -> HashMap<turborepo_lockfiles::Package, HashSet<PackageNode>> {
        // TODO: provide size hint from Lockfile trait
        let mut map: HashMap<turborepo_lockfiles::Package, HashSet<PackageNode>> = HashMap::new();
        // First find which packages directly depend on each external package
        for (pkg, info) in self.packages.iter() {
            for dep in info.transitive_dependencies.iter().flatten() {
                let rdeps = map.entry(dep.clone()).or_default();
                rdeps.insert(PackageNode::Workspace(pkg.clone()));
            }
        }
        // Now trace through all ancestors of the direct dependants
        let root_internal_dependencies = self.root_internal_dependencies();
        let root_external_dependencies =
            self.transitive_external_dependencies(Some(&PackageName::Root));
        for (external_pkg, rdeps) in map.iter_mut() {
            // If one of the reverse dependencies of this external package is a root
            // dependency, everything depends on this
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
        map
    }

    // Returns a map of package name and version for external dependencies
    #[allow(dead_code)]
    fn external_dependencies(
        &self,
        package: &PackageName,
    ) -> Option<&BTreeMap<PackageKey, PackageVersion>> {
        let entry = self.packages.get(package)?;
        entry.unresolved_external_dependencies.as_ref()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ChangedPackagesError {
    #[error("No lockfile")]
    NoLockfile,
    #[error("Lockfile error")]
    Lockfile(#[from] turborepo_lockfiles::Error),
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
    use std::{fs, path::Path, process::Command};

    use serde_json::json;
    use turborepo_errors::Spanned;

    use super::*;
    use crate::discovery::PackageDiscovery;

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
        let status = Command::new("patch")
            .args([target, patch_file])
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "patch {target} {patch_file} failed");
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

            let previous = package_manager
                .read_lockfile(&root, &root_package_json)
                .unwrap();

            apply_patch(tempdir.path(), lockfile, dep_patch);
            let dep_graph = build_lockfile_aware_graph(&root, package_manager.clone());
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
                root_changed_names.contains(&PackageName::from("a"))
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
        let b_external = pkg_graph
            .packages
            .get(&PackageName::from("b"))
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();

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

        let foo_deps = pkg_graph
            .packages
            .get(&foo)
            .unwrap()
            .transitive_dependencies
            .as_ref()
            .unwrap();
        let bar_deps = pkg_graph
            .packages
            .get(&bar)
            .unwrap()
            .transitive_dependencies
            .as_ref()
            .unwrap();
        let a = turborepo_lockfiles::Package::new("key:a", "1");
        let b = turborepo_lockfiles::Package::new("key:b", "1");
        let c = turborepo_lockfiles::Package::new("key:c", "1");
        assert_eq!(foo_deps, &HashSet::from_iter(vec![a.clone(), c.clone(),]));
        assert_eq!(bar_deps, &HashSet::from_iter(vec![b.clone(), c.clone(),]));
        assert_eq!(
            pkg_graph.transitive_external_dependencies([&foo, &bar].iter().copied()),
            HashSet::from_iter(vec![&a, &b, &c,])
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
            "[workspace]\nmembers = [\"rust/*\"]\nresolver = \"2\"\n",
        );
        write(
            &["rust", "app", "Cargo.toml"],
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
             \"2021\"\n\n[dependencies]\nlib-a = { path = \"../lib-a\" }\n",
        );
        write(&["rust", "app", "src", "main.rs"], "fn main() {}\n");
        write(
            &["rust", "lib-a", "Cargo.toml"],
            "[package]\nname = \"lib-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(&["rust", "lib-a", "src", "lib.rs"], "");
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
        for _ in 0..8 {
            let pkg_graph = PackageGraph::builder(
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

            // All packages present, tagged with their toolchain.
            let app = pkg_graph.package_info(&PackageName::from("app")).unwrap();
            assert_eq!(app.toolchain, crate::toolchain::ToolchainId::CARGO);
            let js_pkg = pkg_graph
                .package_info(&PackageName::from("js-pkg"))
                .unwrap();
            assert_eq!(js_pkg.toolchain, crate::toolchain::ToolchainId::JAVASCRIPT);
            let workspace_pkg = pkg_graph.package_info(&PackageName::from("cargo")).unwrap();
            assert_eq!(
                workspace_pkg.toolchain,
                crate::toolchain::ToolchainId::CARGO
            );

            // Crate path dependencies became graph edges.
            let app_deps = pkg_graph
                .immediate_dependencies(&PackageNode::Workspace(PackageName::from("app")))
                .unwrap();
            assert!(
                app_deps.contains(&PackageNode::Workspace(PackageName::from("lib-a"))),
                "app should depend on lib-a, got {app_deps:?}"
            );
            let workspace_deps = pkg_graph
                .immediate_dependencies(&PackageNode::Workspace(PackageName::from("cargo")))
                .unwrap();
            assert!(
                workspace_deps.contains(&PackageNode::Workspace(PackageName::from("app")))
                    && workspace_deps.contains(&PackageNode::Workspace(PackageName::from("lib-a"))),
                "workspace package should depend on every crate, got {workspace_deps:?}"
            );

            // The root's JS lockfile closure is attributed to the root, not
            // stolen by the cargo workspace package sharing its directory.
            let root_closure = pkg_graph
                .package_info(&PackageName::Root)
                .unwrap()
                .transitive_dependencies
                .as_ref()
                .expect("root should have a lockfile closure");
            assert_eq!(
                root_closure,
                &vec![
                    turborepo_lockfiles::Package::new("key:a", "1"),
                    turborepo_lockfiles::Package::new("key:c", "1"),
                ]
                .into_iter()
                .collect::<HashSet<_>>(),
            );
            assert_eq!(
                workspace_pkg.transitive_dependencies, None,
                "the cargo workspace package has no JS lockfile closure"
            );
        }
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
