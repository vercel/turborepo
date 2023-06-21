use std::{
    collections::{HashMap, HashSet},
    fmt,
    rc::Rc,
};

use anyhow::Result;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf, RelativeUnixPathBuf,
};
use turborepo_lockfiles::Lockfile;

use crate::{package_json::PackageJson, package_manager::PackageManager};

mod builder;

pub use builder::PackageGraphBuilder;

#[derive(Default)]
pub struct WorkspaceCatalog {}

pub struct PackageGraph {
    workspace_graph: petgraph::Graph<WorkspaceNode, ()>,
    node_lookup: HashMap<WorkspaceNode, petgraph::graph::NodeIndex>,
    workspaces: HashMap<WorkspaceName, Entry>,
    package_manager: PackageManager,
    lockfile: Option<Box<dyn Lockfile>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Entry {
    json: PackageJson,
    package_json_path: AnchoredSystemPathBuf,
    unresolved_external_dependencies: Option<HashSet<Package>>,
    transitive_dependencies: Option<HashSet<turborepo_lockfiles::Package>>,
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

    pub fn len(&self) -> usize {
        self.workspace_graph.node_count()
    }

    pub fn package_manager(&self) -> &PackageManager {
        &self.package_manager
    }

    pub fn lockfile(&self) -> Option<&dyn Lockfile> {
        self.lockfile.as_deref()
    }

    pub fn package_json(&self, workspace: &WorkspaceName) -> Option<&PackageJson> {
        let entry = self.workspaces.get(workspace)?;
        Some(&entry.json)
    }

    pub fn root_package_json(&self) -> &PackageJson {
        self.package_json(&WorkspaceName::Root)
            .expect("package graph was built without root package.json")
    }

    fn transitive_closure(&self, node: &WorkspaceNode) -> Option<HashSet<&WorkspaceNode>> {
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

    fn external_dependencies(&self, workspace: &WorkspaceName) -> Option<&HashSet<Package>> {
        let entry = self.workspaces.get(workspace)?;
        entry.unresolved_external_dependencies.as_ref()
    }
}

impl fmt::Display for WorkspaceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceName::Root => f.write_str("Root workspace"),
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
}
