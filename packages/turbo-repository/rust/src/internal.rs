use napi::Status;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};
use turborepo_repository::{
    inference::{self, RepoMode as WorkspaceType, RepoState as WorkspaceState},
    package_graph::{PackageGraph, PackageGraphBuilder},
    package_json::PackageJson,
    package_manager,
};
use turborepo_turbo_json::{
    RawTurboJson, TurboJson, TurboJsonPath, TurboJsonReader, load_from_path,
};

use crate::{Package, PackageManager, Workspace};

/// This module is used to isolate code with defined errors
/// from code in lib.rs that needs to have errors coerced to strings /
/// napi::Error for return to javascript.
/// Dividing the source code up this way allows us to be stricter here, and have
/// the strictness relaxed only at the boundary.

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("Failed to resolve starting path from {path}: {path_error}")]
    StartingPath { path_error: PathError, path: String },
    #[error("Failed to resolve package path: {0}")]
    PackagePath(#[from] PathError),
    #[error(transparent)]
    Inference(#[from] inference::Error),
    #[error("Failed to resolve package manager from {path}: {error}")]
    PackageManager {
        error: String,
        path: AbsoluteSystemPathBuf,
    },
    #[error("Package graph error: {0}")]
    PackageGraph(#[from] turborepo_repository::package_graph::Error),
    #[error("package.json error: {0}")]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error("turbo.json error: {0}")]
    TurboJson(#[from] turborepo_turbo_json::Error),
}

impl From<Error> for napi::Error<Status> {
    fn from(value: Error) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

impl Workspace {
    pub(crate) async fn find_internal(path: Option<String>) -> Result<Self, Error> {
        let reference_dir = match path {
            Some(path) => {
                AbsoluteSystemPathBuf::from_cwd(&path).map_err(|path_error| Error::StartingPath {
                    path: path.clone(),
                    path_error,
                })
            }
            None => AbsoluteSystemPathBuf::cwd().map_err(|path_error| Error::StartingPath {
                path: "".to_string(),
                path_error,
            }),
        }?;
        let mut workspace_state = WorkspaceState::infer(&reference_dir)?;
        // The CLI requires a declared package manager (`packageManager` or
        // `devEngines.packageManager`), but this library analyzes repositories
        // it doesn't control, so fall back to lockfile-based detection when the
        // declaration is missing or unusable — mirroring the CLI's
        // `--dangerously-allow-missing-package-manager` behavior. If detection
        // also fails, surface the original declaration error below.
        if workspace_state.package_manager.is_err()
            && let Ok(detected) =
                package_manager::PackageManager::detect_package_manager(&workspace_state.root)
        {
            // `mode` was derived while the package manager was unresolved,
            // so workspace globs could not be read; recompute it with the
            // detected package manager.
            workspace_state.mode = if detected.get_workspace_globs(&workspace_state.root).is_ok() {
                WorkspaceType::MultiPackage
            } else {
                WorkspaceType::SinglePackage
            };
            workspace_state.package_manager = Ok(detected);
        }
        let workspace_state = workspace_state;
        let is_multi_package = workspace_state.mode == WorkspaceType::MultiPackage;
        let package_manager =
            workspace_state
                .package_manager
                .as_ref()
                .map_err(|error| Error::PackageManager {
                    error: error.to_string(),
                    path: workspace_state.root.clone(),
                })?;

        let package_manager_name = package_manager.name();

        let workspace_root = &workspace_state.root;
        let initial_turbo_json = match load_from_path(
            &TurboJsonReader::new(workspace_root.clone()),
            TurboJsonPath::Dir(workspace_root),
            true,
        ) {
            Ok(turbo_json) => turbo_json,
            Err(turborepo_turbo_json::Error::NoTurboJSON) => TurboJson::default(),
            Err(error) => return Err(error.into()),
        };
        let future_flags = initial_turbo_json
            .path()
            .map(|path| workspace_root.join_component(path.as_ref()))
            .map(|path| RawTurboJson::read(workspace_root, &path, true))
            .transpose()?
            .flatten()
            .and_then(|raw| raw.future_flags.map(|flags| flags.into_inner()))
            .unwrap_or_default();
        let turbo_json = if initial_turbo_json.path().is_some() {
            load_from_path(
                &TurboJsonReader::new(workspace_root.clone()).with_future_flags(future_flags),
                TurboJsonPath::Dir(workspace_root),
                true,
            )?
        } else {
            initial_turbo_json
        };
        let root_package_json = PackageJson::load(&workspace_root.join_component("package.json"))?;
        let package_manager_version = detect_package_manager_version(&root_package_json);
        let mut package_graph_builder = PackageGraphBuilder::new(workspace_root, root_package_json)
            .with_single_package_mode(!is_multi_package)
            .with_package_manager(package_manager.clone());
        if turbo_json.future_flags.experimental_cargo_workspaces {
            package_graph_builder = package_graph_builder.with_cargo();
        }
        if turbo_json.future_flags.experimental_python_workspaces {
            package_graph_builder = package_graph_builder.with_uv();
        }
        let package_graph = package_graph_builder.build().await?;

        Ok(Self {
            absolute_path: workspace_state.root.to_string(),
            is_multi_package,
            package_manager: PackageManager {
                name: package_manager_name.to_string(),
                version: package_manager_version,
            },
            graph: package_graph,
        })
    }

    pub(crate) async fn packages_internal(&self) -> Result<Vec<Package>, Error> {
        packages_from_graph(&self.graph)
    }
}

/// Best-effort extraction of the declared package manager version from the
/// root `package.json`. Prefers the `packageManager` field (e.g.
/// `pnpm@9.12.3`), falling back to `devEngines.packageManager.version`. Returns
/// `None` when neither is present or when the version points at a URL rather
/// than a concrete version. This is metadata only, so any failure to parse is
/// swallowed rather than surfaced as an error.
fn detect_package_manager_version(package_json: &PackageJson) -> Option<String> {
    if let Some(field) = &package_json.package_manager
        && let Ok((_, version)) =
            package_manager::PackageManager::parse_package_manager_string(field)
        && !version.starts_with("http")
    {
        return Some(version.to_string());
    }

    let dev_engines = package_json.dev_engines.as_ref()?;
    let version = dev_engines
        .as_object()?
        .get("packageManager")?
        .as_object()?
        .get("version")?
        .as_str()?;
    Some(version.to_string())
}

fn packages_from_graph(graph: &PackageGraph) -> Result<Vec<Package>, Error> {
    let mut packages = graph
        .package_task_contexts()
        .filter(|context| graph.is_real_package(context.package()))
        .map(|context| {
            let path = graph.repo_root().resolve(context.directory());
            Package::new(
                context.package().as_str().to_owned(),
                graph.repo_root(),
                &path,
            )
            .map_err(Error::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    packages.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(packages)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use turborepo_errors::Spanned;
    use turborepo_repository::toolchain::{
        DiscoverPackagesFuture, DiscoveredPackage, DiscoveredPackages, RepositoryContributor,
        ToolchainId, WorkspaceRoot,
    };

    use super::*;

    struct CustomPackageJsonContributor {
        root: AbsoluteSystemPathBuf,
    }

    impl RepositoryContributor for CustomPackageJsonContributor {
        fn id(&self) -> ToolchainId {
            ToolchainId::new("custom")
        }

        fn discover_packages(&self) -> DiscoverPackagesFuture<'_> {
            Box::pin(async move {
                Ok(DiscoveredPackages::new(
                    vec![
                        DiscoveredPackage::package(
                            Some("custom-package".to_string()),
                            PackageJson::default(),
                            self.root.join_components(&["custom", "package.json"]),
                        ),
                        DiscoveredPackage::package(
                            Some("native-package".to_string()),
                            PackageJson::default(),
                            self.root.join_components(&["native", "Cargo.toml"]),
                        ),
                        DiscoveredPackage::aggregate(
                            "custom-aggregate".to_string(),
                            PackageJson::default(),
                            self.root.join_components(&["aggregate", "package.json"]),
                        ),
                    ],
                    vec![WorkspaceRoot::new("custom", self.root.clone())],
                ))
            })
        }
    }

    #[tokio::test]
    async fn package_listing_uses_retained_graph_generation() {
        let root = AbsoluteSystemPathBuf::new(std::env::temp_dir().to_string_lossy()).unwrap();
        let package_jsons = HashMap::from([
            (
                root.join_components(&["z", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("z".into())),
                    ..Default::default()
                },
            ),
            (
                root.join_components(&["a", "package.json"]),
                PackageJson {
                    name: Some(Spanned::new("a".into())),
                    ..Default::default()
                },
            ),
        ]);
        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_manager(package_manager::PackageManager::Npm)
            .with_package_jsons(Some(package_jsons))
            .with_allow_no_package_manager(true)
            .build()
            .await
            .unwrap();

        let packages = packages_from_graph(&graph).unwrap();
        assert_eq!(
            packages
                .iter()
                .map(|package| package.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
    }

    #[tokio::test]
    async fn package_listing_uses_manifest_capability_not_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::new(temp.path().to_string_lossy().to_string()).unwrap();
        let graph = PackageGraphBuilder::new(&root, PackageJson::default())
            .with_package_manager(package_manager::PackageManager::Npm)
            .with_package_jsons(Some(HashMap::new()))
            .with_allow_no_package_manager(true)
            .with_contributor(Arc::new(CustomPackageJsonContributor {
                root: root.clone(),
            }))
            .build()
            .await
            .unwrap();

        let packages = packages_from_graph(&graph).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "custom-package");
        assert_eq!(
            graph.package_toolchain(&"custom-package".into()),
            Some(&ToolchainId::new("custom"))
        );
    }
}
