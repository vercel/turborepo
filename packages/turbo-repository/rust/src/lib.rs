use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use either::Either;
use napi::Error;
use napi_derive::napi;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AnchoredSystemPath, AnchoredSystemPathBuf};
use turborepo_repository::{
    change_mapper::{
        ChangeMapper, DefaultPackageChangeMapper, DefaultPackageChangeMapperWithLockfile,
        LockfileContents, PackageChangeMapper, PackageChanges,
    },
    inference::RepoState as WorkspaceState,
    package_graph::{PackageGraph, PackageName, PackageNode, ROOT_PKG_NAME, WorkspacePackage},
};
use turborepo_scm::SCM;
mod internal;

#[napi]
#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct Package {
    pub name: String,
    /// The absolute path to the package root.
    #[napi(readonly)]
    pub absolute_path: String,
    /// The relative path from the workspace root to the package root.
    #[napi(readonly)]
    pub relative_path: String,
}

/// Wrapper for dependents and dependencies.
/// Each are a list of package paths, relative to the workspace root.
#[napi]
#[derive(Debug)]
pub struct PackageDetails {
    /// the package's dependencies
    #[napi(readonly)]
    pub dependencies: Vec<String>,
    /// the packages that depend on this package
    #[napi(readonly)]
    pub dependents: Vec<String>,
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
        let pkgs = match graph.immediate_ancestors(&node) {
            Some(pkgs) => pkgs,
            None => return vec![],
        };

        pkgs.iter()
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

    fn dependencies(
        &self,
        graph: &PackageGraph,
        workspace_path: &AbsoluteSystemPath,
    ) -> Vec<Package> {
        let node = PackageNode::Workspace(PackageName::Other(self.name.clone()));
        let pkgs = match graph.immediate_dependencies(&node) {
            Some(pkgs) => pkgs,
            None => return vec![],
        };

        pkgs.iter()
            .filter(|node| !matches!(node, PackageNode::Root))
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

    /// Returns a map of packages within the workspace, its dependencies and
    /// dependents. The response looks like this:
    ///  {
    ///    "package-path": {
    ///      "dependents": ["dependent1_path", "dependent2_path"],
    ///      "dependencies": ["dependency1_path", "dependency2_path"]
    ///      }
    ///  }
    #[napi]
    pub async fn find_packages_with_graph(&self) -> Result<HashMap<String, PackageDetails>, Error> {
        let packages = self.find_packages().await?;

        let workspace_path = match AbsoluteSystemPath::new(self.absolute_path.as_str()) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let map: HashMap<String, PackageDetails> = packages
            .into_iter()
            .map(|package| {
                let details = PackageDetails {
                    dependencies: package
                        .dependencies(&self.graph, workspace_path)
                        .into_iter()
                        .map(|p| p.relative_path)
                        .collect(),
                    dependents: package
                        .dependents(&self.graph, workspace_path)
                        .into_iter()
                        .map(|p| p.relative_path)
                        .collect(),
                };

                (package.relative_path, details)
            })
            .collect();

        Ok(map)
    }

    pub fn get_lockfile_contents(
        &self,
        changed_files: &HashSet<AnchoredSystemPathBuf>,
        workspace_root: &AbsoluteSystemPath,
        from_commit: &str,
    ) -> LockfileContents {
        let lockfile_name = self.graph.package_manager().lockfile_name();
        if changed_files.contains(AnchoredSystemPath::new(&lockfile_name).unwrap()) {
            let git = SCM::new(workspace_root);
            let anchored_path = workspace_root.join_component(lockfile_name);
            git.previous_content(Some(from_commit), &anchored_path)
                .map(LockfileContents::Changed)
                .inspect_err(|e| debug!("{e}"))
                .ok()
                .unwrap_or(LockfileContents::UnknownChange)
        } else {
            LockfileContents::Unchanged
        }
    }

    /// Given a set of "changed" files, returns a set of packages that are
    /// "affected" by the changes. The `files` argument is expected to be a list
    /// of strings relative to the monorepo root and use the current system's
    /// path separator.
    #[napi]
    pub async fn affected_packages(
        &self,
        files: Vec<String>,
        base: Option<&str>, // this is required when optimize_global_invalidations is true
        optimize_global_invalidations: Option<bool>,
    ) -> Result<Vec<Package>, Error> {
        let base = optimize_global_invalidations
            .unwrap_or(false)
            .then(|| {
                base.ok_or_else(|| {
                    Error::from_reason("optimizeGlobalInvalidations true, but no base commit given")
                })
            })
            .transpose()?;
        let workspace_root = match AbsoluteSystemPath::new(&self.absolute_path) {
            Ok(path) => path,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };
        let changed_files: HashSet<AnchoredSystemPathBuf> = files
            .into_iter()
            .filter_map(|path| {
                let path_components = path.split(std::path::MAIN_SEPARATOR).collect::<Vec<&str>>();
                let absolute_path = workspace_root.join_components(&path_components);
                workspace_root.anchor(&absolute_path).ok()
            })
            .collect();

        // Create a ChangeMapper with no ignore patterns
        let change_detector = if base.is_some() {
            Either::Left(DefaultPackageChangeMapperWithLockfile::new(&self.graph))
        } else {
            Either::Right(DefaultPackageChangeMapper::new(&self.graph))
        };
        let mapper = ChangeMapper::new(&self.graph, vec![], change_detector);

        let lockfile_contents = if let Some(base) = base {
            self.get_lockfile_contents(&changed_files, workspace_root, base)
        } else if changed_files.contains(
            AnchoredSystemPath::new(self.graph.package_manager().lockfile_name())
                .expect("the lockfile name will not be an absolute path"),
        ) {
            LockfileContents::UnknownChange
        } else {
            LockfileContents::Unchanged
        };

        let package_changes = match mapper.changed_packages(changed_files, lockfile_contents) {
            Ok(changes) => changes,
            Err(e) => return Err(Error::from_reason(e.to_string())),
        };

        let packages = match package_changes {
            PackageChanges::All(_) => self
                .graph
                .packages()
                .map(|(name, info)| WorkspacePackage {
                    name: name.to_owned(),
                    path: info.package_path().to_owned(),
                })
                .collect::<Vec<WorkspacePackage>>(),
            PackageChanges::Some(packages) => packages.into_keys().collect(),
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

    /// Given a path (relative to the workspace root), returns the
    /// package that contains it.
    ///
    /// This is a naive implementation that simply "iterates-up". If this
    /// function is expected to be called many times for files that are deep
    /// within the same package, we could optimize this by caching the
    /// containing-package of every ancestor.
    #[napi]
    pub async fn find_package_by_path(&self, path: String) -> Result<Package, Error> {
        let package_mapper = DefaultPackageChangeMapper::new(&self.graph);
        let anchored_path = AnchoredSystemPath::new(&path)
            .map_err(|e| Error::from_reason(e.to_string()))?
            .clean();
        match package_mapper.detect_package(&anchored_path) {
            turborepo_repository::change_mapper::PackageMapping::All(
                _all_package_change_reason,
            ) => Err(Error::from_reason("file belongs to many packages")),
            turborepo_repository::change_mapper::PackageMapping::None => Err(Error::from_reason(
                "iterated to the root of the workspace and found no package",
            )),
            turborepo_repository::change_mapper::PackageMapping::Package((package, _reason)) => {
                let workspace_root = match AbsoluteSystemPath::new(&self.absolute_path) {
                    Ok(path) => path,
                    Err(e) => return Err(Error::from_reason(e.to_string())),
                };
                let package_path = workspace_root.resolve(&package.path);
                Ok(Package::new(
                    package.name.to_string(),
                    workspace_root,
                    &package_path,
                ))
            }
        }
    }
}
