use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use napi::Error;
use napi_derive::napi;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{ChangeMapper, PackageChanges},
    inference::RepoState as WorkspaceState,
    package_graph::{PackageGraph, PackageName, PackageNode, WorkspacePackage, ROOT_PKG_NAME},
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
        let node = PackageNode::Workspace(PackageName::Other(self.name.clone()));
        let ancestors = match graph.immediate_ancestors(&node) {
            Some(ancestors) => ancestors,
            None => return vec![],
        };

        ancestors
            .iter()
            .filter_map(|node| {
                let info = graph.package_info(node.as_package_name())?;
                // If we don't get a package name back, we'll just skip it.
                let name = info.package_name()?;
                let anchored_package_path = info.package_path();
                let package_path = workspace_path.resolve(anchored_package_path);
                Some(Package::new(name, workspace_path, &package_path))
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

    /// Finds and returns a map of packages within the workspace and its
    /// dependents (i.e. the packages that depend on each of those packages).
    #[napi]
    pub async fn find_packages_and_dependents(
        &self,
    ) -> Result<HashMap<String, Vec<String>>, Error> {
        let packages = self.find_packages().await?;

        let workspace_path = match AbsoluteSystemPath::new(self.absolute_path.as_str()) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let map: HashMap<String, Vec<String>> = packages
            .into_iter()
            .map(|package| {
                let deps = package.dependents(&self.graph, workspace_path);
                let dep_names = deps
                    .into_iter()
                    .map(|p| p.relative_path)
                    .collect::<Vec<String>>();

                (package.relative_path, dep_names)
            })
            .collect();

        Ok(map)
    }

    /// Given a set of "changed" files, returns a set of packages that are
    /// "affected" by the changes. The `files` argument is expected to be a list
    /// of strings relative to the monorepo root and use the current system's
    /// path separator.
    #[napi]
    pub async fn affected_packages(&self, files: Vec<String>) -> Result<Vec<Package>, Error> {
        let workspace_root = match AbsoluteSystemPath::new(&self.absolute_path) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let hash_set_of_paths: HashSet<AnchoredSystemPathBuf> = files
            .into_iter()
            .filter_map(|path| {
                let path_components = path.split(std::path::MAIN_SEPARATOR).collect::<Vec<&str>>();
                let absolute_path = workspace_root.join_components(&path_components);
                workspace_root.anchor(&absolute_path).ok()
            })
            .collect();

        // Create a ChangeMapper with no custom global deps or ignore patterns
        let mapper = ChangeMapper::new(&self.graph, vec![], vec![]);
        let package_changes = match mapper.changed_packages(hash_set_of_paths, None) {
            Ok(changes) => changes,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let packages = match package_changes {
            PackageChanges::All => self
                .graph
                .packages()
                .map(|(name, info)| WorkspacePackage {
                    name: name.to_owned(),
                    path: info.package_path().to_owned(),
                })
                .collect::<Vec<WorkspacePackage>>(),
            PackageChanges::Some(packages) => packages.into_iter().collect(),
        };

        let mut serializable_packages: Vec<Package> = packages
            .into_iter()
            .filter(|p| match &p.name {
                PackageName::Root => false,
                PackageName::Other(name) => name != ROOT_PKG_NAME,
            })
            .map(|p| {
                let package_path = workspace_root.resolve(&p.path);
                Package::new(p.name.to_string(), workspace_root, &package_path)
            })
            .collect();

        serializable_packages.sort_by_key(|p| p.name.clone());

        Ok(serializable_packages)
    }
}
