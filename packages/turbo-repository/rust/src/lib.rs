use std::{collections::HashMap, hash::Hash};

use napi::Error;
use napi_derive::napi;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    inference::RepoState as WorkspaceState,
    package_graph::{PackageGraph, WorkspaceName, WorkspaceNode},
};
mod internal;

#[napi]
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Package {
    pub name: String,
    /// The absolute path to the package root.
    #[napi(readonly)]
    pub absolute_path: String,
    /// The relative path from the workspace root to the package root.
    #[napi(readonly)]
    pub relative_path: String,
}

#[derive(Clone)]
#[napi]

pub struct PackageManager {
    /// The package manager name in lower case.
    #[napi(readonly)]
    pub name: String,
}

#[napi]
pub struct Workspace {
    workspace_state: WorkspaceState,
    /// The absolute path to the workspace root.
    #[napi(readonly)]
    pub absolute_path: String,
    /// `true` when the workspace is a multi-package workspace.
    #[napi(readonly)]
    pub is_multi_package: bool,
    /// The package manager used by the workspace.
    #[napi(readonly)]
    pub package_manager: PackageManager,
    /// The package graph for the workspace.
    graph: PackageGraph,
}

#[napi]
impl Package {
    fn new(
        name: String,
        workspace_path: &AbsoluteSystemPath,
        package_path: &AbsoluteSystemPath,
    ) -> Self {
        let relative_path = workspace_path
            .anchor(package_path)
            .expect("Package path is within the workspace");
        Self {
            name,
            absolute_path: package_path.to_string(),
            relative_path: relative_path.to_string(),
        }
    }

    fn dependents(
        &self,
        graph: &PackageGraph,
        workspace_path: &AbsoluteSystemPath,
    ) -> Vec<Package> {
        let node = WorkspaceNode::Workspace(WorkspaceName::Other(self.name.clone()));
        let ancestors = graph.immediate_ancestors(&node).unwrap();

        ancestors
            .iter()
            .map(|node| {
                let info = graph.workspace_info(node.as_workspace()).unwrap();
                let name = info.package_name().unwrap();
                let anchored_package_path = info.package_path();
                let package_path = workspace_path.resolve(anchored_package_path);
                Package::new(name, workspace_path, &package_path)
            })
            .collect()
    }
}

#[napi]
impl Workspace {
    /// Finds the workspace root from the given path, and returns a new
    /// Workspace.
    #[napi(factory)]
    pub async fn find(path: Option<String>) -> Result<Workspace, napi::Error> {
        Self::find_internal(path).await.map_err(|e| e.into())
    }

    /// Finds and returns packages within the workspace.
    #[napi]
    pub async fn find_packages(&self) -> std::result::Result<Vec<Package>, napi::Error> {
        self.packages_internal().await.map_err(|e| e.into())
    }

    #[napi]
    pub async fn package_graph(&self) -> Result<HashMap<String, Vec<String>>, Error> {
        let mut map = HashMap::new();
        let packages = self.find_packages().await?;
        let workspace_path = AbsoluteSystemPath::new(self.absolute_path.as_str()).unwrap();

        for (_i, package) in packages.iter().enumerate() {
            let deps = package.dependents(&self.graph, workspace_path); // Get upstream dependencies
            let dep_names = deps.iter().map(|p| p.relative_path.clone()).collect();

            // TODO: use name instead of relative_path for both the key and value?
            map.insert(package.relative_path.clone(), dep_names);
        }

        return Ok(map);
    }
}
