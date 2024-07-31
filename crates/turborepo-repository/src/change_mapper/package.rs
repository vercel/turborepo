use thiserror::Error;
use turbopath::AnchoredSystemPath;
use wax::{BuildError, Program};

use crate::package_graph::{PackageGraph, PackageName, WorkspacePackage};

pub enum PackageMapping {
    /// We've hit a global file, so all packages have changed
    All,
    /// This change is meaningless, no packages have changed
    None,
    /// This change has affected one package
    Package(WorkspacePackage),
}

/// Maps a single file change to affected packages. This can be a single
/// package (`Package`), none of the packages (`None`), or all of the packages
/// (`All`).
pub trait PackageChangeMapper {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping;
}

/// Detects package by checking if the file is inside the package.
/// Does *not* use the `globalDependencies` in turbo.json.
/// Since we don't have these dependencies, any file that is
/// not in any package will automatically invalidate all
/// packages. This is fine for builds, but less fine
/// for situations like watch mode.
pub struct DefaultPackageChangeMapper<'a> {
    pkg_dep_graph: &'a PackageGraph,
}

impl<'a> DefaultPackageChangeMapper<'a> {
    pub fn new(pkg_dep_graph: &'a PackageGraph) -> Self {
        Self { pkg_dep_graph }
    }
    fn is_file_in_package(file: &AnchoredSystemPath, package_path: &AnchoredSystemPath) -> bool {
        file.components()
            .zip(package_path.components())
            .all(|(a, b)| a == b)
    }
}

impl<'a> PackageChangeMapper for DefaultPackageChangeMapper<'a> {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping {
        for (name, entry) in self.pkg_dep_graph.packages() {
            if name == &PackageName::Root {
                continue;
            }
            if let Some(package_path) = entry.package_json_path.parent() {
                if Self::is_file_in_package(file, package_path) {
                    return PackageMapping::Package(WorkspacePackage {
                        name: name.clone(),
                        path: package_path.to_owned(),
                    });
                }
            }
        }

        PackageMapping::All
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    InvalidFilter(#[from] BuildError),
}

/// A package detector that uses a global deps list to determine
/// if a file should cause all packages to be marked as changed.
/// This is less conservative than the `DefaultPackageChangeMapper`,
/// which assumes that any changed file that is not in a package
/// changes all packages. Since we have a list of global deps,
/// we can check against that and avoid invalidating in unnecessary cases.
pub struct GlobalDepsPackageChangeMapper<'a> {
    pkg_dep_graph: &'a PackageGraph,
    global_deps_matcher: wax::Any<'a>,
}

impl<'a> GlobalDepsPackageChangeMapper<'a> {
    pub fn new<S: wax::Pattern<'a>, I: Iterator<Item = S>>(
        pkg_dep_graph: &'a PackageGraph,
        global_deps: I,
    ) -> Result<Self, Error> {
        let global_deps_matcher = wax::any(global_deps)?;

        Ok(Self {
            pkg_dep_graph,
            global_deps_matcher,
        })
    }
}

impl<'a> PackageChangeMapper for GlobalDepsPackageChangeMapper<'a> {
    fn detect_package(&self, path: &AnchoredSystemPath) -> PackageMapping {
        match DefaultPackageChangeMapper::new(self.pkg_dep_graph).detect_package(path) {
            // Since `DefaultPackageChangeMapper` is overly conservative, we can check here if
            // the path is actually in globalDeps and if not, return it as
            // PackageDetection::Package(WorkspacePackage::root()).
            PackageMapping::All => {
                let cleaned_path = path.clean();
                let in_global_deps = self.global_deps_matcher.is_match(cleaned_path.as_str());

                if in_global_deps {
                    PackageMapping::All
                } else {
                    PackageMapping::Package(WorkspacePackage::root())
                }
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AnchoredSystemPathBuf};

    use super::{DefaultPackageChangeMapper, GlobalDepsPackageChangeMapper};
    use crate::{
        change_mapper::{AllPackageChangeReason, ChangeMapper, PackageChanges},
        discovery,
        discovery::PackageDiscovery,
        package_graph::{PackageGraphBuilder, WorkspacePackage},
        package_json::PackageJson,
    };

    #[allow(dead_code)]
    pub struct MockDiscovery;

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
            Ok(discovery::DiscoveryResponse {
                package_manager: crate::package_manager::PackageManager::Npm,
                workspaces: vec![],
            })
        }

        async fn discover_packages_blocking(
            &self,
        ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
            self.discover_packages().await
        }
    }

    #[tokio::test]
    async fn test_different_package_detectors() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let root_package_json = PackageJson::default();

        let pkg_graph = PackageGraphBuilder::new(
            AbsoluteSystemPath::from_std_path(repo_root.path())?,
            root_package_json,
        )
        .with_package_discovery(MockDiscovery)
        .build()
        .await?;

        let default_package_detector = DefaultPackageChangeMapper::new(&pkg_graph);
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], default_package_detector);

        let package_changes = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            None,
        )?;

        // We should return All because we don't have global deps and
        // therefore must be conservative about changes
        assert_eq!(
            package_changes,
            PackageChanges::All(AllPackageChangeReason::NonPackageFileChanged)
        );

        let turbo_package_detector =
            GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], turbo_package_detector);

        let package_changes = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            None,
        )?;

        // We only get a root workspace change since we have global deps specified and
        // README.md is not one of them
        assert_eq!(
            package_changes,
            PackageChanges::Some([WorkspacePackage::root()].into_iter().collect())
        );

        Ok(())
    }
}
