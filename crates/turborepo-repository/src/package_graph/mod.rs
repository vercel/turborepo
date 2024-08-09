use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
};

use itertools::Itertools;
use petgraph::visit::{depth_first_search, Reversed};
use serde::Serialize;
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};
use turborepo_graph_utils as graph;
use turborepo_lockfiles::Lockfile;

use crate::{
    discovery::LocalPackageDiscoveryBuilder, package_json::PackageJson,
    package_manager::PackageManager,
};

pub mod builder;
mod dep_splitter;
mod npmrc;

pub use builder::{Error, PackageGraphBuilder};

pub const ROOT_PKG_NAME: &str = "//";

#[derive(Debug)]
pub struct PackageGraph {
    graph: petgraph::Graph<PackageNode, ()>,
    #[allow(dead_code)]
    node_lookup: HashMap<PackageNode, petgraph::graph::NodeIndex>,
    packages: HashMap<PackageName, PackageInfo>,
    package_manager: PackageManager,
    lockfile: Option<Box<dyn Lockfile>>,
    repo_root: AbsoluteSystemPathBuf,
}

/// The WorkspacePackage follows the Vercel glossary of terms where "Workspace"
/// is the collection of packages and "Package" is a single package within the
/// workspace. https://vercel.com/docs/vercel-platform/glossary
/// There are other structs in this module that have "Workspace" in the name,
/// but they do NOT follow the glossary, and instead mean "package" when they
/// say Workspace. Some of these are labeled as such.
#[derive(Debug, Eq, PartialEq, Hash)]
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
    pub package_json: PackageJson,
    pub package_json_path: AnchoredSystemPathBuf,
    pub unresolved_external_dependencies: Option<BTreeMap<PackageKey, PackageVersion>>, /* name -> version */
    pub transitive_dependencies: Option<HashSet<turborepo_lockfiles::Package>>,
}

impl PackageInfo {
    pub fn package_name(&self) -> Option<String> {
        self.package_json.name.clone()
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

impl PackageGraph {
    pub fn builder(
        repo_root: &AbsoluteSystemPath,
        root_package_json: PackageJson,
    ) -> PackageGraphBuilder<LocalPackageDiscoveryBuilder> {
        PackageGraphBuilder::new(repo_root, root_package_json)
    }

    #[tracing::instrument(skip(self))]
    pub fn validate(&self) -> Result<(), Error> {
        for info in self.packages.values() {
            let name = info.package_json.name.as_deref();
            if matches!(name, None | Some("")) {
                let package_json_path = self.repo_root.resolve(info.package_json_path());
                return Err(Error::PackageJsonMissingName(package_json_path));
            }
        }
        graph::validate_graph(&self.graph).map_err(Error::InvalidPackageGraph)?;

        Ok(())
    }

    pub fn remove_package_dependencies(&mut self) {
        let root_index = self
            .node_lookup
            .get(&PackageNode::Root)
            .expect("graph should have root package node");
        self.graph.retain_edges(|graph, index| {
            let Some((_src, dst)) = graph.edge_endpoints(index) else {
                return false;
            };
            dst == *root_index
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

    pub fn packages(&self) -> impl Iterator<Item = (&PackageName, &PackageInfo)> {
        self.packages.iter()
    }

    pub fn root_package_json(&self) -> &PackageJson {
        self.package_json(&PackageName::Root)
            .expect("package graph was built without root package.json")
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
    #[allow(dead_code)]
    pub fn dependencies<'a>(&'a self, node: &PackageNode) -> HashSet<&'a PackageNode> {
        let mut dependencies =
            self.transitive_closure_inner(Some(node), petgraph::Direction::Outgoing);
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
    pub fn ancestors(&self, node: &PackageNode) -> HashSet<&PackageNode> {
        // If node is a root dep, then *every* package is an ancestor of this one
        let mut dependents = if self.root_internal_dependencies().contains(node) {
            return self.graph.node_weights().collect();
        } else {
            self.transitive_closure_inner(Some(node), petgraph::Direction::Incoming)
        };
        dependents.remove(node);
        dependents
    }

    pub fn root_internal_package_dependencies(&self) -> HashSet<WorkspacePackage> {
        let dependencies = self.root_internal_dependencies();
        dependencies
            .into_iter()
            .filter_map(|package| match package {
                PackageNode::Workspace(package) => {
                    let path = self
                        .package_dir(package)
                        .expect("packages in graph should have info");
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
            .into_iter()
            .filter_map(|package| match package {
                PackageNode::Workspace(package) => Some(
                    self.package_dir(package)
                        .expect("packages in graph should have info"),
                ),
                PackageNode::Root => None,
            })
            .sorted()
            .collect()
    }

    /// Provides a path from the root package to package
    ///
    /// Currently only provides the shortest path as calculating all paths can
    /// be O(n!)
    pub fn root_internal_dependency_explanation(
        &self,
        package: &WorkspacePackage,
    ) -> Option<String> {
        let from = *self
            .node_lookup
            .get(&PackageNode::Workspace(PackageName::Root))
            .expect("all graphs should have a root");
        let to = *self
            .node_lookup
            .get(&PackageNode::Workspace(package.name.clone()))?;
        let (_cost, path) =
            petgraph::algo::astar(&self.graph, from, |node| node == to, |_| 1, |_| 1)?;
        Some(
            self.path_display(&path)
                .expect("path should only contain valid node indices"),
        )
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

    fn root_internal_dependencies(&self) -> HashSet<&PackageNode> {
        // We cannot call self.dependencies(&PackageNode::Workspace(PackageName::Root))
        // as it will infinitely recurse.
        let mut dependencies = self.transitive_closure_inner(
            Some(&PackageNode::Workspace(PackageName::Root)),
            petgraph::Direction::Outgoing,
        );
        dependencies.remove(&PackageNode::Workspace(PackageName::Root));
        dependencies
    }

    /// Returns the transitive closure of the given nodes in the package
    /// graph. Note that this includes the nodes themselves. If you want just
    /// the dependencies, or the dependents, use `dependencies` or `ancestors`.
    /// Alternatively, if you need just direct dependents, use
    /// `immediate_dependents`.
    pub fn transitive_closure<'a, 'b, I: IntoIterator<Item = &'b PackageNode>>(
        &'a self,
        nodes: I,
    ) -> HashSet<&'a PackageNode> {
        self.transitive_closure_inner(nodes, petgraph::Direction::Outgoing)
    }

    fn transitive_closure_inner<'a, 'b, I: IntoIterator<Item = &'b PackageNode>>(
        &'a self,
        nodes: I,
        direction: petgraph::Direction,
    ) -> HashSet<&'a PackageNode> {
        let indices = nodes
            .into_iter()
            .filter_map(|node| self.node_lookup.get(node))
            .copied();

        let mut visited = HashSet::new();

        let visitor = |event| {
            if let petgraph::visit::DfsEvent::Discover(n, _) = event {
                visited.insert(
                    self.graph
                        .node_weight(n)
                        .expect("node index found during dfs doesn't exist"),
                );
            }
        };

        match direction {
            petgraph::Direction::Outgoing => depth_first_search(&self.graph, indices, visitor),
            petgraph::Direction::Incoming => {
                depth_first_search(Reversed(&self.graph), indices, visitor)
            }
        };

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
    ) -> Result<Vec<WorkspacePackage>, ChangedPackagesError> {
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
            .collect::<HashMap<_, HashMap<_, _>>>();

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
                .filter(|(name, info)| {
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
                    }
                    not_equal
                })
                .map(|(name, info)| match name {
                    PackageName::Other(n) => {
                        let w_name = PackageName::Other(n.to_owned());
                        Some(WorkspacePackage {
                            name: w_name.clone(),
                            path: info.package_path().to_owned(),
                        })
                    }
                    // if the root package has changed, then we should report `None`
                    // since all packages need to be revalidated
                    PackageName::Root => None,
                })
                .collect::<Option<Vec<WorkspacePackage>>>()
        };

        Ok(changed.unwrap_or_else(|| {
            self.packages
                .iter()
                .map(|(name, info)| WorkspacePackage {
                    name: name.clone(),
                    path: info.package_path().to_owned(),
                })
                .collect()
        }))
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
    use std::assert_matches::assert_matches;

    use serde_json::json;

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

    #[tokio::test]
    async fn test_single_package_is_depends_on_root() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson {
                name: Some("my-package".to_owned()),
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
        assert!(result.is_ok(), "expected ok {:?}", result);
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
        ) -> std::result::Result<Option<HashMap<String, String>>, turborepo_lockfiles::Error>
        {
            match key {
                "key:a" => Ok(Some(
                    [("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )),
                "key:b" => Ok(Some(
                    [("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )),
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

        assert_matches!(
            pkg_graph.validate(),
            Err(builder::Error::InvalidPackageGraph(
                graph::Error::CyclicDependencies(_)
            ))
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

        assert_matches!(
            pkg_graph.validate(),
            Err(builder::Error::InvalidPackageGraph(
                graph::Error::SelfDependency(_)
            ))
        );
    }
}
