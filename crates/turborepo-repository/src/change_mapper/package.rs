use thiserror::Error;
use turbopath::{AnchoredSystemPath, AnchoredSystemPathBuf};
use wax::{BuildError, Program};

use crate::{
    change_mapper::{AllPackageChangeReason, PackageInclusionReason},
    package_graph::{PackageGraph, PackageName, PackageTaskContextKind, WorkspacePackage},
    package_manager::PackageManager,
};

pub enum PackageMapping {
    /// We've hit a global file, so all packages have changed
    All(AllPackageChangeReason),
    /// This change is meaningless, no packages have changed
    None,
    /// This change has affected one or more packages.
    Packages(Vec<(WorkspacePackage, PackageInclusionReason)>),
}

/// Maps a single file change to its affected packages (`Packages`), none of the
/// packages (`None`), or all of the packages (`All`).
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
pub struct DefaultPackageChangeMapper {
    /// Deepest-package index: package directory → all owning package names.
    /// Built once per mapper so a lookup walks the file's ancestors rather
    /// than scanning every package. Co-located owners are sorted by name for
    /// deterministic iteration, not to choose one owner over another.
    package_dirs: std::collections::HashMap<AnchoredSystemPathBuf, Vec<PackageName>>,
}

impl DefaultPackageChangeMapper {
    pub fn new(pkg_dep_graph: &PackageGraph) -> Self {
        let mut package_dirs = std::collections::HashMap::new();
        for context in pkg_dep_graph.package_task_contexts() {
            if context.kind() != PackageTaskContextKind::Package {
                continue;
            }
            let package_path = context.directory();
            // A package whose directory is the repo root would vacuously
            // match every file. Only the Root package may claim root-level
            // files, via the fallback.
            if package_path.components().next().is_none() {
                continue;
            }
            package_dirs
                .entry(package_path.to_owned())
                .or_insert_with(Vec::new)
                .push(context.package().clone());
        }
        for names in package_dirs.values_mut() {
            names.sort();
        }

        Self { package_dirs }
    }
}

impl PackageChangeMapper for DefaultPackageChangeMapper {
    fn detect_package(&self, file: &AnchoredSystemPath) -> PackageMapping {
        // Walk the file path and its ancestors deepest-first; the first
        // indexed directory is the deepest package containing the file.
        // Including the path itself preserves matching when the changed path
        // *is* a package directory. All owners at the first matching directory
        // are affected; shallower packages must not claim the same change.
        for ancestor in file.ancestors() {
            if let Some(names) = self.package_dirs.get(ancestor) {
                return PackageMapping::Packages(
                    names
                        .iter()
                        .map(|name| {
                            (
                                WorkspacePackage {
                                    name: name.clone(),
                                    path: ancestor.to_owned(),
                                },
                                PackageInclusionReason::FileChanged {
                                    file: file.to_owned(),
                                },
                            )
                        })
                        .collect(),
                );
            }
        }

        PackageMapping::All(AllPackageChangeReason::GlobalDepsChanged {
            file: file.to_owned(),
        })
    }
}

pub struct DefaultPackageChangeMapperWithLockfile {
    base: DefaultPackageChangeMapper,
}

impl DefaultPackageChangeMapperWithLockfile {
    pub fn new(pkg_dep_graph: &PackageGraph) -> Self {
        Self {
            base: DefaultPackageChangeMapper::new(pkg_dep_graph),
        }
    }
}

impl PackageChangeMapper for DefaultPackageChangeMapperWithLockfile {
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
            PackageMapping::Packages(vec![(
                WorkspacePackage {
                    name: PackageName::Root,
                    path: AnchoredSystemPathBuf::from_raw("").unwrap(),
                },
                PackageInclusionReason::ConservativeRootLockfileChanged,
            )])
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
    base: DefaultPackageChangeMapperWithLockfile,
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
                    PackageMapping::Packages(vec![(
                        WorkspacePackage::root(),
                        PackageInclusionReason::FileChanged {
                            file: path.to_owned(),
                        },
                    )])
                }
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use super::{
        DefaultPackageChangeMapper, GlobalDepsPackageChangeMapper, PackageChangeMapper,
        PackageMapping,
    };
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
    async fn nested_package_owns_its_files() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let root = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let parent_manifest = root.join_components(&["packages", "parent", "package.json"]);
        let child_manifest = root.join_components(&["packages", "parent", "child", "package.json"]);
        parent_manifest.ensure_dir()?;
        parent_manifest.create_with_contents(r#"{"name":"parent"}"#)?;
        child_manifest.ensure_dir()?;
        child_manifest.create_with_contents(r#"{"name":"child"}"#)?;

        struct NestedDiscovery {
            parent_manifest: AbsoluteSystemPathBuf,
            child_manifest: AbsoluteSystemPathBuf,
        }
        impl PackageDiscovery for NestedDiscovery {
            async fn discover_packages(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                Ok(discovery::DiscoveryResponse {
                    package_manager: PackageManager::Npm,
                    // Parent first reproduces the observation order that used
                    // to make it incorrectly claim the child's files.
                    workspaces: vec![
                        discovery::WorkspaceData::new(self.parent_manifest.clone(), None)?,
                        discovery::WorkspaceData::new(self.child_manifest.clone(), None)?,
                    ],
                })
            }

            async fn discover_packages_blocking(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                self.discover_packages().await
            }
        }

        let graph = PackageGraphBuilder::new(root, PackageJson::default())
            .with_package_discovery(NestedDiscovery {
                parent_manifest,
                child_manifest,
            })
            .build()
            .await?;
        let file = AnchoredSystemPathBuf::from_raw(
            ["packages", "parent", "child", "src", "index.ts"].join(std::path::MAIN_SEPARATOR_STR),
        )?;

        let PackageMapping::Packages(packages) =
            DefaultPackageChangeMapper::new(&graph).detect_package(&file)
        else {
            panic!("expected a package mapping");
        };
        let [(package, _)] = packages.as_slice() else {
            panic!("expected only the child package");
        };
        assert_eq!(package.name.as_ref(), "child");
        assert_eq!(package.path.to_unix().as_str(), "packages/parent/child");

        Ok(())
    }

    /// A package whose directory is the repository root must never claim
    /// files during change mapping: the component-zip membership check is
    /// vacuously true for a zero-component package path, so such a package
    /// would nondeterministically steal every changed file from the real
    /// packages (package iteration order picks the winner). Only the Root
    /// package may claim root-level files, via the fallback.
    #[tokio::test]
    async fn root_directory_package_does_not_claim_files() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let root = AbsoluteSystemPath::from_std_path(repo_root.path())?;

        let write = |rel: &[&str], contents: &str| -> Result<(), anyhow::Error> {
            let path = root.join_components(rel);
            path.ensure_dir()?;
            path.create_with_contents(contents)?;
            Ok(())
        };
        write(&["package.json"], r#"{"name": "rooted-pkg"}"#)?;
        write(
            &["packages", "lib-a", "package.json"],
            r#"{"name": "lib-a"}"#,
        )?;

        struct RootedDiscovery {
            root: AbsoluteSystemPathBuf,
        }
        impl PackageDiscovery for RootedDiscovery {
            async fn discover_packages(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                Ok(discovery::DiscoveryResponse {
                    package_manager: PackageManager::Npm,
                    workspaces: vec![
                        discovery::WorkspaceData::new(
                            self.root.join_component("package.json"),
                            None,
                        )?,
                        discovery::WorkspaceData::new(
                            self.root
                                .join_components(&["packages", "lib-a", "package.json"]),
                            None,
                        )?,
                    ],
                })
            }

            async fn discover_packages_blocking(
                &self,
            ) -> Result<discovery::DiscoveryResponse, discovery::Error> {
                self.discover_packages().await
            }
        }

        let pkg_graph = PackageGraphBuilder::new(root, PackageJson::default())
            .with_package_discovery(RootedDiscovery {
                root: root.to_owned(),
            })
            .build()
            .await?;

        let detector = GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], detector);

        // A change in a real package maps to that package alone.
        let result = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw(
                ["packages", "lib-a", "index.js"].join(std::path::MAIN_SEPARATOR_STR),
            )?]
            .into_iter()
            .collect(),
            LockfileContents::Unchanged,
        )?;
        let PackageChanges::Some(packages) = result else {
            panic!("expected Some, got {result:?}");
        };
        let names: Vec<&str> = packages.keys().map(|p| p.name.as_ref()).collect();
        assert_eq!(names, vec!["lib-a"]);

        // A root-level file falls through to the root fallback; it must not
        // be attributed to the root-directory package.
        let result = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("README.md")?]
                .into_iter()
                .collect(),
            LockfileContents::Unchanged,
        )?;
        let PackageChanges::Some(packages) = result else {
            panic!("expected Some, got {result:?}");
        };
        assert!(
            packages.keys().all(|p| p.name.as_ref() != "rooted-pkg"),
            "root-level files must not map to a root-directory package, got {packages:?}"
        );

        Ok(())
    }

    async fn colocated_graph(
        root: &AbsoluteSystemPath,
        javascript_name: &str,
        native_packages: &[(crate::toolchain::ToolchainId, &str)],
        root_package_json: PackageJson,
    ) -> Result<crate::package_graph::PackageGraph, anyhow::Error> {
        use std::sync::Arc;

        use crate::toolchain::{
            DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
            ToolchainId, WorkspaceRoot,
        };

        struct SharedDirContributor {
            root: AbsoluteSystemPathBuf,
            id: ToolchainId,
            manifest: String,
        }
        impl RepositoryContributor for SharedDirContributor {
            fn id(&self) -> ToolchainId {
                self.id.clone()
            }
            fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
                Box::pin(async move {
                    Ok(DiscoveredPackages::new(
                        vec![
                            DiscoveredPackage::package(
                                Some(self.id.to_string()),
                                PackageJson::default(),
                                self.root
                                    .join_components(&["packages", "shared", &self.manifest]),
                            ),
                            DiscoveredPackage::package(
                                Some(format!("child-{}", self.id)),
                                PackageJson::default(),
                                self.root.join_components(&[
                                    "packages",
                                    "shared",
                                    "child",
                                    &self.manifest,
                                ]),
                            ),
                        ],
                        vec![WorkspaceRoot::new(self.id.as_str(), self.root.clone())],
                    ))
                })
            }
            fn discover_package_scopes(&self) -> crate::toolchain::DiscoverPackageScopesFuture<'_> {
                Box::pin(async move {
                    let output = self.discover_packages().await?;
                    Ok(
                        crate::toolchain::DiscoveredPackageScopes::from_full_observation(
                            output.packages(),
                            output.workspace_roots(),
                        ),
                    )
                })
            }
        }

        let mut manifests = std::collections::HashMap::new();
        for (directory, name) in [
            (vec!["packages"], "parent"),
            (vec!["packages", "shared"], javascript_name),
            (vec!["packages", "shared", "child"], "child-js"),
            (vec!["packages", "unrelated"], "unrelated"),
        ] {
            let path = root
                .join_components(&directory)
                .join_component("package.json");
            path.ensure_dir()?;
            path.create_with_contents(format!(r#"{{"name":"{name}"}}"#))?;
            manifests.insert(path.clone(), PackageJson::load(&path)?);
        }
        let mut builder = PackageGraphBuilder::new(root, root_package_json)
            .with_package_discovery(MockDiscovery)
            .with_package_jsons(Some(manifests));
        for (id, manifest) in native_packages {
            builder = builder.with_contributor(Arc::new(SharedDirContributor {
                root: root.to_owned(),
                id: id.clone(),
                manifest: manifest.to_string(),
            }));
        }
        Ok(builder.build().await?)
    }

    #[tokio::test]
    async fn colocated_packages_all_own_changed_files() -> Result<(), anyhow::Error> {
        use crate::toolchain::ToolchainId;

        let go = (ToolchainId::GO, "go.mod");
        let rust = (ToolchainId::RUST, "Cargo.toml");
        for native in [
            vec![go.clone()],
            vec![go.clone(), rust.clone()],
            vec![rust, go],
        ] {
            for javascript_name in ["aaa-js", "zzz-js"] {
                let repo_root = tempdir()?;
                let root = AbsoluteSystemPath::from_std_path(repo_root.path())?;
                let graph =
                    colocated_graph(root, javascript_name, &native, PackageJson::default()).await?;
                let detector =
                    GlobalDepsPackageChangeMapper::new(&graph, std::iter::empty::<&str>())?;
                let mapper = ChangeMapper::new(&graph, vec![], detector);
                for relative in [
                    "src/index.ts",
                    "src/lib.go",
                    "src/lib.rs",
                    "deleted.txt",
                    "",
                ] {
                    let directory = AnchoredSystemPathBuf::from_raw(
                        ["packages", "shared"].join(std::path::MAIN_SEPARATOR_STR),
                    )?;
                    let file = directory.join(&AnchoredSystemPathBuf::from_raw(
                        relative.replace('/', std::path::MAIN_SEPARATOR_STR),
                    )?);
                    let result = mapper
                        .changed_packages([file.clone()].into(), LockfileContents::Unchanged)?;
                    let PackageChanges::Some(packages) = result else {
                        panic!("expected directly changed packages, got {result:?}");
                    };
                    let expected = std::iter::once(javascript_name.to_string())
                        .chain(native.iter().map(|(id, _)| id.to_string()))
                        .collect::<std::collections::BTreeSet<_>>();
                    assert_eq!(
                        packages
                            .keys()
                            .map(|pkg| pkg.name.to_string())
                            .collect::<std::collections::BTreeSet<_>>(),
                        expected
                    );
                    for (package, reason) in packages {
                        assert_eq!(package.path, directory);
                        assert_eq!(
                            reason,
                            PackageInclusionReason::FileChanged { file: file.clone() }
                        );
                    }
                }
                // Co-location does not let parents claim a deeper package's files.
                let file = AnchoredSystemPathBuf::from_raw("packages/shared/child/src/file.txt")?;
                let PackageChanges::Some(packages) =
                    mapper.changed_packages([file].into(), LockfileContents::Unchanged)?
                else {
                    panic!("expected nested packages");
                };
                let expected = std::iter::once("child-js".to_string())
                    .chain(native.iter().map(|(id, _)| format!("child-{id}")))
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    packages
                        .keys()
                        .map(|pkg| pkg.name.to_string())
                        .collect::<std::collections::BTreeSet<_>>(),
                    expected
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn colocated_root_dependency_still_invalidates_all_packages() -> Result<(), anyhow::Error>
    {
        use crate::{package_graph::PackageName, toolchain::ToolchainId};

        let repo_root = tempdir()?;
        let root = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let graph = colocated_graph(
            root,
            "aaa-js",
            &[
                (ToolchainId::GO, "go.mod"),
                (ToolchainId::RUST, "Cargo.toml"),
            ],
            PackageJson {
                dependencies: Some([("rust".to_string(), "*".to_string())].into()),
                ..Default::default()
            },
        )
        .await?;
        let mapper = ChangeMapper::new(&graph, vec![], DefaultPackageChangeMapper::new(&graph));
        let file = AnchoredSystemPathBuf::from_raw("packages/shared/src/file.txt")?;
        assert_eq!(
            mapper.changed_packages([file].into(), LockfileContents::Unchanged)?,
            PackageChanges::All(AllPackageChangeReason::RootInternalDepChanged {
                root_internal_dep: PackageName::from("rust")
            })
        );
        Ok(())
    }

    /// A changed path that *is* a package directory must still map to that
    /// package (directory-level change events rely on this).
    #[tokio::test]
    async fn package_directory_path_maps_to_package() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let root = AbsoluteSystemPath::from_std_path(repo_root.path())?;
        let manifest = root.join_components(&["packages", "lib-a", "package.json"]);
        manifest.ensure_dir()?;
        manifest.create_with_contents(r#"{"name":"lib-a"}"#)?;

        let graph = PackageGraphBuilder::new(root, PackageJson::default())
            .with_package_discovery({
                struct D(AbsoluteSystemPathBuf);
                impl PackageDiscovery for D {
                    async fn discover_packages(
                        &self,
                    ) -> Result<discovery::DiscoveryResponse, discovery::Error>
                    {
                        Ok(discovery::DiscoveryResponse {
                            package_manager: PackageManager::Npm,
                            workspaces: vec![discovery::WorkspaceData::new(self.0.clone(), None)?],
                        })
                    }
                    async fn discover_packages_blocking(
                        &self,
                    ) -> Result<discovery::DiscoveryResponse, discovery::Error>
                    {
                        self.discover_packages().await
                    }
                }
                D(manifest)
            })
            .build()
            .await?;

        let dir_path = AnchoredSystemPathBuf::from_raw(
            ["packages", "lib-a"].join(std::path::MAIN_SEPARATOR_STR),
        )?;
        let PackageMapping::Packages(packages) =
            DefaultPackageChangeMapper::new(&graph).detect_package(&dir_path)
        else {
            panic!("expected a package mapping for the package directory itself");
        };
        let [(package, _)] = packages.as_slice() else {
            panic!("expected only lib-a");
        };
        assert_eq!(package.name.as_ref(), "lib-a");

        Ok(())
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

    #[tokio::test]
    async fn root_package_json_not_global_with_global_deps_mapper() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let pkg_graph = PackageGraphBuilder::new(
            AbsoluteSystemPath::from_std_path(repo_root.path())?,
            PackageJson::default(),
        )
        .with_package_discovery(MockDiscovery)
        .build()
        .await?;

        let detector = GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], detector);

        // root package.json is not in the global hash when a lockfile exists,
        // so it should only affect the root workspace — not all packages.
        let result = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("package.json")?]
                .into_iter()
                .collect(),
            LockfileContents::Unchanged,
        )?;

        assert_eq!(
            result,
            PackageChanges::Some(
                [(
                    WorkspacePackage::root(),
                    PackageInclusionReason::FileChanged {
                        file: AnchoredSystemPathBuf::from_raw("package.json")?,
                    }
                )]
                .into_iter()
                .collect()
            )
        );

        Ok(())
    }

    #[tokio::test]
    async fn turbo_json_still_global() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let pkg_graph = PackageGraphBuilder::new(
            AbsoluteSystemPath::from_std_path(repo_root.path())?,
            PackageJson::default(),
        )
        .with_package_discovery(MockDiscovery)
        .build()
        .await?;

        let detector = GlobalDepsPackageChangeMapper::new(&pkg_graph, std::iter::empty::<&str>())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], detector);

        // turbo.json task definitions are part of every task hash,
        // so it must remain a global trigger.
        let result = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("turbo.json")?]
                .into_iter()
                .collect(),
            LockfileContents::Unchanged,
        )?;

        assert_eq!(
            result,
            PackageChanges::All(AllPackageChangeReason::DefaultGlobalFileChanged {
                file: AnchoredSystemPathBuf::from_raw("turbo.json")?,
            })
        );

        Ok(())
    }

    #[tokio::test]
    async fn package_json_in_global_deps_triggers_all() -> Result<(), anyhow::Error> {
        let repo_root = tempdir()?;
        let pkg_graph = PackageGraphBuilder::new(
            AbsoluteSystemPath::from_std_path(repo_root.path())?,
            PackageJson::default(),
        )
        .with_package_discovery(MockDiscovery)
        .build()
        .await?;

        // Users can opt-in to the old behavior via globalDependencies.
        let detector =
            GlobalDepsPackageChangeMapper::new(&pkg_graph, ["package.json"].into_iter())?;
        let change_mapper = ChangeMapper::new(&pkg_graph, vec![], detector);

        let result = change_mapper.changed_packages(
            [AnchoredSystemPathBuf::from_raw("package.json")?]
                .into_iter()
                .collect(),
            LockfileContents::Unchanged,
        )?;

        assert_eq!(
            result,
            PackageChanges::All(AllPackageChangeReason::GlobalDepsChanged {
                file: AnchoredSystemPathBuf::from_raw("package.json")?,
            })
        );

        Ok(())
    }
}
