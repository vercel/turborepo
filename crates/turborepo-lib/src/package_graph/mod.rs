use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use anyhow::Result;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_lockfiles::Lockfile;

use crate::{package_json::PackageJson, package_manager::PackageManager};

mod builder;

pub use builder::PackageGraphBuilder;

pub struct PackageGraph {
    workspace_graph: petgraph::Graph<WorkspaceNode, ()>,
    #[allow(dead_code)]
    node_lookup: HashMap<WorkspaceNode, petgraph::graph::NodeIndex>,
    workspaces: HashMap<WorkspaceName, Entry>,
    package_manager: PackageManager,
    lockfile: Option<Box<dyn Lockfile>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    package_json: PackageJson,
    package_json_path: AnchoredSystemPathBuf,
    unresolved_external_dependencies: Option<HashSet<Package>>,
    transitive_dependencies: Option<HashSet<turborepo_lockfiles::Package>>,
}

impl Entry {
    pub fn package_json_path(&self) -> &AnchoredSystemPathBuf {
        &self.package_json_path
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
struct Package {
    name: String,
    version: String,
}

/// Name of workspaces with a special marker for the workspace root
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum WorkspaceName {
    Root,
    Other(String),
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum WorkspaceNode {
    Root,
    Workspace(WorkspaceName),
}

impl PackageGraph {
    pub fn builder(
        repo_root: &AbsoluteSystemPath,
        root_package_json: PackageJson,
    ) -> PackageGraphBuilder {
        PackageGraphBuilder::new(repo_root, root_package_json)
    }

    pub fn validate(&self) -> Result<()> {
        // TODO
        Ok(())
    }

    /// Returns the number of workspaces in the repo
    /// *including* the root workspace.
    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    pub fn package_manager(&self) -> &PackageManager {
        &self.package_manager
    }

    pub fn lockfile(&self) -> Option<&dyn Lockfile> {
        self.lockfile.as_deref()
    }

    pub fn package_json(&self, workspace: &WorkspaceName) -> Option<&PackageJson> {
        let entry = self.workspaces.get(workspace)?;
        Some(&entry.package_json)
    }

    pub fn workspaces(&self) -> impl Iterator<Item = (&WorkspaceName, &Entry)> {
        self.workspaces.iter()
    }

    pub fn root_package_json(&self) -> &PackageJson {
        self.package_json(&WorkspaceName::Root)
            .expect("package graph was built without root package.json")
    }

    pub fn transitive_closure(&self, node: &WorkspaceNode) -> Option<HashSet<&WorkspaceNode>> {
        let idx = self.node_lookup.get(node)?;
        let mut visited = HashSet::new();
        petgraph::visit::depth_first_search(&self.workspace_graph, Some(*idx), |event| {
            if let petgraph::visit::DfsEvent::Discover(n, _) = event {
                visited.insert(
                    self.workspace_graph
                        .node_weight(n)
                        .expect("node index found during dfs doesn't exist"),
                );
            }
        });
        Some(visited)
    }

    #[allow(dead_code)]
    fn external_dependencies(&self, workspace: &WorkspaceName) -> Option<&HashSet<Package>> {
        let entry = self.workspaces.get(workspace)?;
        entry.unresolved_external_dependencies.as_ref()
    }
}

impl fmt::Display for WorkspaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceName::Root => f.write_str("//"),
            WorkspaceName::Other(other) => f.write_str(other),
        }
    }
}

impl From<String> for WorkspaceName {
    fn from(value: String) -> Self {
        Self::Other(value)
    }
}

impl<'a> From<&'a str> for WorkspaceName {
    fn from(value: &'a str) -> Self {
        Self::from(value.to_string())
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    #[test]
    fn test_single_package_is_depends_on_root() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(&root, PackageJson::default())
            .with_package_manger(Some(PackageManager::Npm))
            .with_single_package_mode(true)
            .build()
            .unwrap();

        let closure = pkg_graph
            .transitive_closure(&WorkspaceNode::Workspace(WorkspaceName::Root))
            .unwrap();
        assert!(closure.contains(&WorkspaceNode::Root));
    }

    #[test]
    fn test_internal_dependencies_get_split_out() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_manger(Some(PackageManager::Npm))
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
        .unwrap();

        let closure = pkg_graph
            .transitive_closure(&WorkspaceNode::Workspace("a".into()))
            .unwrap();
        assert!(closure.contains(&WorkspaceNode::Workspace("b".into())));
        let b_external = pkg_graph
            .workspaces
            .get(&WorkspaceName::from("b"))
            .unwrap()
            .unresolved_external_dependencies
            .as_ref()
            .unwrap();
        assert!(b_external.contains(&Package {
            name: "c".into(),
            version: "1.2.3".into()
        }));
    }

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
                    vec![("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )),
                "key:b" => Ok(Some(
                    vec![("c", "1")]
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )),
                "key:c" => Ok(None),
                _ => Ok(None),
            }
        }
    }

    #[test]
    fn test_lockfile_traversal() {
        let root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { r"C:\repo" } else { "/repo" }).unwrap();
        let pkg_graph = PackageGraph::builder(
            &root,
            PackageJson::from_value(json!({ "name": "root" })).unwrap(),
        )
        .with_package_manger(Some(PackageManager::Npm))
        .with_package_jsons(Some({
            let mut map = HashMap::new();
            map.insert(
                root.join_component("package_a"),
                PackageJson::from_value(json!({
                    "name": "foo",
                    "dependencies": {
                        "a": "1"
                    }
                }))
                .unwrap(),
            );
            map.insert(
                root.join_component("package_b"),
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
        .unwrap();

        let foo_deps = pkg_graph
            .workspaces
            .get(&WorkspaceName::from("foo"))
            .unwrap()
            .transitive_dependencies
            .as_ref()
            .unwrap();
        let bar_deps = pkg_graph
            .workspaces
            .get(&WorkspaceName::from("bar"))
            .unwrap()
            .transitive_dependencies
            .as_ref()
            .unwrap();
        assert_eq!(
            foo_deps,
            &HashSet::from_iter(vec![
                turborepo_lockfiles::Package::new("key:a", "1"),
                turborepo_lockfiles::Package::new("key:c", "1")
            ])
        );
        assert_eq!(
            bar_deps,
            &HashSet::from_iter(vec![
                turborepo_lockfiles::Package::new("key:b", "1"),
                turborepo_lockfiles::Package::new("key:c", "1")
            ])
        );
    }
}
