use std::{collections::HashMap, hash::Hash};

use napi_derive::napi;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::inference::RepoState as WorkspaceState;
mod internal;

#[napi]
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Package {
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
}

impl Package {
    fn new(workspace_path: &AbsoluteSystemPath, package_path: &AbsoluteSystemPath) -> Self {
        let relative_path = workspace_path
            .anchor(package_path)
            .expect("Package path is within the workspace");
        Self {
            absolute_path: package_path.to_string(),
            relative_path: relative_path.to_string(),
        }
    }

    // TODO: implement this
    fn dependents(&self) -> Vec<Package> {
        vec![]
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
    pub async fn package_graph(&self) -> HashMap<String, Vec<String>> {
        let mut map = HashMap::new();
        let packages = self.find_packages().await.unwrap();

        for (_i, package) in packages.iter().enumerate() {
            let deps = package.dependents(); // Get upstream dependencies
            let dep_names = deps.iter().map(|p| p.relative_path.clone()).collect();

            // TODO: use name instead of relative_path for both the key and value?
            map.insert(package.relative_path.clone(), dep_names);
        }

        return map;
    }
}
