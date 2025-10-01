use thiserror::Error;
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
use wax::{BuildError, Program};

use crate::{
    change_mapper::{AllPackageChangeReason, PackageInclusionReason},
    package_graph::{PackageGraph, PackageName, WorkspacePackage},
    package_manager::PackageManager,
};

pub enum PackageMapping {
    /// We've hit a global file, so all packages have changed
    All(AllPackageChangeReason),
    /// This change is meaningless, no packages have changed
    None,
    /// This change has affected one package
    Package((WorkspacePackage, PackageInclusionReason)),
}

/// Maps a single file change to affected packages. This can be a single
/// package (`Package`), none of the packages (`None`), or all of the packages
/// (`All`).
pub trait PackageChangeMapper {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping;
}

impl<L, R> PackageChangeMapper for either::Either<L, R>
where
    L: PackageChangeMapper,
    R: PackageChangeMapper,
{
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping {
        match self {
            either::Either::Left(l) => l.detect_package(file),
            either::Either::Right(r) => r.detect_package(file),
        }
    }
}

/// Detects package by checking if the file is inside the package.
///
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

impl PackageChangeMapper for DefaultPackageChangeMapper<'_> {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping {
        for (name, entry) in self.pkg_dep_graph.packages() {
            if name == &PackageName::Root {
                continue;
            }
            if let Some(package_path) = entry.package_json_path.parent()
                && Self::is_file_in_package(file, package_path)
            {
                return PackageMapping::Package((
                    WorkspacePackage {
                        name: name.clone(),
                        path: package_path.to_owned(),
                    },
                    PackageInclusionReason::FileChanged {
                        file: file.to_owned(),
                    },
                ));
            }
        }

        PackageMapping::All(AllPackageChangeReason::GlobalDepsChanged {
            file: file.to_owned(),
        })
    }
}

pub struct DefaultPackageChangeMapperWithLockfile<'a> {
    base: DefaultPackageChangeMapper<'a>,
}

impl<'a> DefaultPackageChangeMapperWithLockfile<'a> {
    pub fn new(pkg_dep_graph: &'a PackageGraph) -> Self {
        Self {
            base: DefaultPackageChangeMapper::new(pkg_dep_graph),
        }
    }
}

impl PackageChangeMapper for DefaultPackageChangeMapperWithLockfile<'_> {
    fn detect_package(&self, path: &AnchoredSystemPath) -> PackageMapping {
        // If we have a lockfile change, we consider this as a root package change,
        // since there's a chance that the root package uses a workspace package
        // dependency (this is cursed behavior but sadly possible). There's a chance
        // that we can make this more accurate by checking which package
        // manager, since not all package managers may permit root pulling from
        // workspace package dependencies
        if PackageManager::supported_managers()
            .iter()
            .any(|pm| pm.lockfile_name() == path.as_str())
        {
            PackageMapping::Package((
                WorkspacePackage {
                    name: PackageName::Root,
                    path: AnchoredSystemPathBuf::from_raw("").unwrap(),
                },
                PackageInclusionReason::ConservativeRootLockfileChanged,
            ))
        } else {
            self.base.detect_package(path)
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    InvalidFilter(#[from] BuildError),
}

/// A package detector.
///
/// It uses a global deps list to determine
/// if a file should cause all packages to be marked as changed.
/// This is less conservative than the `DefaultPackageChangeMapper`,
/// which assumes that any changed file that is not in a package
/// changes all packages. Since we have a list of global deps,
/// we can check against that and avoid invalidating in unnecessary cases.
pub struct GlobalDepsPackageChangeMapper<'a> {
    base: DefaultPackageChangeMapperWithLockfile<'a>,
    global_deps_matcher: wax::Any<'a>,
}

impl<'a> GlobalDepsPackageChangeMapper<'a> {
    pub fn new<S: wax::Pattern<'a>, I: Iterator<Item = S>>(
        pkg_dep_graph: &'a PackageGraph,
        global_deps: I,
    ) -> Result<Self, Error> {
        let base = DefaultPackageChangeMapperWithLockfile::new(pkg_dep_graph);
        let global_deps_matcher = wax::any(global_deps)?;

        Ok(Self {
            base,
            global_deps_matcher,
        })
    }
}

impl PackageChangeMapper for GlobalDepsPackageChangeMapper<'_> {
    fn detect_package(&self, path: &AnchoredSystemPath) -> PackageMapping {
        match self.base.detect_package(path) {
            // Since `DefaultPackageChangeMapper` is overly conservative, we can check here if
            // the path is actually in globalDeps and if not, return it as
            // PackageDetection::Package(WorkspacePackage::root()).
            PackageMapping::All(_) => {
                let cleaned_path = path.clean();
                let in_global_deps = self.global_deps_matcher.is_match(cleaned_path.as_str());

                if in_global_deps {
                    PackageMapping::All(AllPackageChangeReason::GlobalDepsChanged {
                        file: path.to_owned(),
                    })
                } else {
                    PackageMapping::Package((
                        WorkspacePackage::root(),
                        PackageInclusionReason::FileChanged {
                            file: path.to_owned(),
                        },
                    ))
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
        change_mapper::{
            AllPackageChangeReason, ChangeMapper, LockfileContents, PackageChanges,
            PackageInclusionReason,
        },
        discovery::{self, PackageDiscovery},
        package_graph::{PackageGraphBuilder, WorkspacePackage},
        package_json::PackageJson,
        package_manager::PackageManager,
    };

    #[allow(dead_code)]
    pub struct MockDiscovery;

    impl PackageDiscovery for MockDiscovery {
        async fn discover_packages(
            &self,
        ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
            Ok(discovery::DiscoveryResponse {
                package_manager: PackageManager::Npm,
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
            LockfileContents::Unchanged,
        )?;

        // We should return All because we don't have global deps and
        // therefore must be conservative about changes
        assert_eq!(
            package_changes,
            PackageChanges::All(AllPackageChangeReason::GlobalDepsChanged {
                file: AnchoredSystemPathBuf::from_raw("README.md")?,
            })
        );

        let turbo_package_detector =
            GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], turbo_package_detector);

        let package_changes = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            LockfileContents::Unchanged,
        )?;

        // We only get a root workspace change since we have global deps specified and
        // README.md is not one of them
        assert_eq!(
            package_changes,
            PackageChanges::Some(
                [(
                    WorkspacePackage::root(),
                    PackageInclusionReason::FileChanged {
                        file: AnchoredSystemPathBuf::from_raw("README.md")?,
                    }
                )]
                .into_iter()
                .collect()
            )
        );

        Ok(())
    }
}
