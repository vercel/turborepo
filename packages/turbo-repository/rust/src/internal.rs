use std::collections::{BTreeMap, HashMap};

use napi::Status;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};
use turborepo_lockfiles::Lockfile;
use turborepo_repository::{
    inference::{self, RepoMode as WorkspaceType, RepoState as WorkspaceState},
    package_graph::{PackageGraph, PackageGraphBuilder, lockfile_closure},
    package_json::{DependencyKind, PackageJson},
    package_manager,
};
use turborepo_turbo_json::{
    RawTurboJson, TurboJson, TurboJsonPath, TurboJsonReader, load_from_path,
};

use crate::{
    LockfileError, LockfileErrorKind, LockfilePackage, LockfilePackages, LockfilePackagesMetadata,
    Package, PackageManager, Workspace, package_source_name, split_identity,
};

enum DirectLockfileRead {
    Loaded(Box<dyn Lockfile>),
    Missing(String),
    Unreadable {
        kind: LockfileErrorKind,
        message: String,
    },
}

/// Resolves the workspace lockfile directly from the root `package.json`,
/// bypassing the package graph. Used for single-package repositories (whose
/// core graph intentionally skips lockfile resolution) and for multi-package
/// repositories opened with `skipPackageGraph`.
pub(crate) struct DirectLockfile {
    workspace_root: AbsoluteSystemPathBuf,
    package_manager: package_manager::PackageManager,
    root_package_json: PackageJson,
    lockfile: DirectLockfileRead,
}

impl DirectLockfile {
    fn new(
        workspace_root: &turbopath::AbsoluteSystemPath,
        package_manager: &package_manager::PackageManager,
        package_json: &PackageJson,
    ) -> Self {
        let lockfile = match package_manager.read_lockfile(workspace_root, package_json) {
            Ok(lockfile) => DirectLockfileRead::Loaded(lockfile),
            Err(package_manager::Error::LockfileMissing(_))
                if matches!(
                    package_manager.lockfile_manager(),
                    package_manager::PackageManager::Bun
                ) && workspace_root.join_component("bun.lockb").exists() =>
            {
                DirectLockfileRead::Unreadable {
                    kind: LockfileErrorKind::UnsupportedBunLockfile,
                    message: "Only found bun.lockb, please run `bun install --save-text-lockfile`"
                        .to_string(),
                }
            }
            Err(package_manager::Error::LockfileMissing(path)) => {
                DirectLockfileRead::Missing(format!("Lockfile not found at {path}"))
            }
            Err(error) => DirectLockfileRead::Unreadable {
                kind: classify_package_manager_error(&error),
                message: error.to_string(),
            },
        };
        Self {
            workspace_root: workspace_root.to_owned(),
            package_manager: package_manager.clone(),
            root_package_json: package_json.clone(),
            lockfile,
        }
    }

    pub(crate) fn error_kind(&self) -> Option<LockfileErrorKind> {
        match &self.lockfile {
            DirectLockfileRead::Loaded(_) => None,
            DirectLockfileRead::Missing(_) => Some(LockfileErrorKind::NoLockfile),
            DirectLockfileRead::Unreadable { kind, .. } => Some(*kind),
        }
    }

    pub(crate) fn error_message(&self) -> Option<String> {
        match &self.lockfile {
            DirectLockfileRead::Loaded(_) => None,
            DirectLockfileRead::Missing(message)
            | DirectLockfileRead::Unreadable { message, .. } => Some(message.clone()),
        }
    }

    /// The transitive closure of the root `package.json` declarations only.
    /// Single-package repositories have no other manifests.
    fn root_closure(
        &self,
        lockfile: &dyn Lockfile,
    ) -> Result<Vec<turborepo_lockfiles::Package>, String> {
        let mut dependencies = BTreeMap::new();
        for (name, specifier, kind) in self.root_package_json.dependencies_with_kind() {
            if !matches!(kind, DependencyKind::Peer { .. }) {
                dependencies
                    .entry(name.clone())
                    .or_insert_with(|| specifier.clone());
            }
        }
        let mut closures = turborepo_lockfiles::all_transitive_closures_sorted(
            lockfile,
            HashMap::from([(String::new(), dependencies)]),
            false,
        )
        .map_err(|error| error.to_string())?;
        Ok(closures
            .remove("")
            .unwrap_or_default()
            .into_iter()
            .map(|package| (*package).clone())
            .collect())
    }

    pub(crate) fn packages(
        &self,
        is_multi_package: bool,
        lockfile_path: &str,
        lockfile_format: &str,
        package_manager: &PackageManager,
    ) -> LockfilePackages {
        let metadata = |lockfile_version| LockfilePackagesMetadata {
            lockfile_path: lockfile_path.to_string(),
            lockfile_format: lockfile_format.to_string(),
            lockfile_version,
            package_manager: package_manager.name.clone(),
            package_manager_version: package_manager.version.clone(),
        };
        let lockfile = match &self.lockfile {
            DirectLockfileRead::Loaded(lockfile) => lockfile.as_ref(),
            DirectLockfileRead::Missing(message) => {
                return LockfilePackages::new(
                    Vec::new(),
                    vec![LockfileError {
                        kind: LockfileErrorKind::NoLockfile,
                        message: message.clone(),
                    }],
                    metadata(None),
                );
            }
            DirectLockfileRead::Unreadable { kind, message } => {
                return LockfilePackages::new(
                    Vec::new(),
                    vec![LockfileError {
                        kind: *kind,
                        message: message.clone(),
                    }],
                    metadata(None),
                );
            }
        };

        let closure = if is_multi_package {
            lockfile_closure::external_packages(
                &self.workspace_root,
                &self.package_manager,
                &self.root_package_json,
                lockfile,
            )
            .map_err(|error| error.to_string())
        } else {
            self.root_closure(lockfile)
        };
        let closure = match closure {
            Ok(closure) => closure,
            Err(message) => {
                return LockfilePackages::new(
                    Vec::new(),
                    vec![LockfileError {
                        kind: LockfileErrorKind::ResolutionFailed,
                        message,
                    }],
                    metadata(lockfile.format_version()),
                );
            }
        };

        let mut packages = Vec::new();
        let mut errors = Vec::new();
        for package in &closure {
            let display_name = lockfile
                .human_name(package)
                .unwrap_or_else(|| package.key.clone());
            match split_identity(&display_name) {
                Some((name, version)) => packages.push(LockfilePackage {
                    name,
                    version,
                    source: package_source_name(lockfile.package_source(package)),
                }),
                None => errors.push(LockfileError {
                    kind: LockfileErrorKind::UnparseableEntry,
                    message: format!(
                        "could not parse name and version from lockfile entry '{display_name}'"
                    ),
                }),
            }
        }
        packages
            .sort_by(|left, right| (&left.name, &left.version).cmp(&(&right.name, &right.version)));
        packages.dedup_by(|left, right| left.name == right.name && left.version == right.version);
        LockfilePackages::new(packages, errors, metadata(lockfile.format_version()))
    }
}

fn classify_package_manager_error(error: &package_manager::Error) -> LockfileErrorKind {
    match error {
        package_manager::Error::Lockfile(turborepo_lockfiles::Error::UnsupportedNpmVersion) => {
            LockfileErrorKind::UnsupportedNpmLockfileVersion
        }
        package_manager::Error::BunBinaryLockfile => LockfileErrorKind::UnsupportedBunLockfile,
        _ => LockfileErrorKind::LockfileUnreadable,
    }
}

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
    #[error(
        "package graph is unavailable because the workspace was opened with skipPackageGraph; \
         only lockfilePackages() is supported"
    )]
    PackageGraphSkipped,
}

impl From<Error> for napi::Error<Status> {
    fn from(value: Error) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

impl Workspace {
    pub(crate) async fn find_internal(
        path: Option<String>,
        skip_package_graph: bool,
    ) -> Result<Self, Error> {
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
        let lockfile_manager = package_manager.lockfile_manager();
        let lockfile_path = if matches!(lockfile_manager, package_manager::PackageManager::Bun)
            && !workspace_root.join_component("bun.lock").exists()
            && workspace_root.join_component("bun.lockb").exists()
        {
            workspace_root.join_component("bun.lockb")
        } else {
            package_manager.lockfile_path(workspace_root)
        }
        .to_string();
        let lockfile_format = match lockfile_manager {
            package_manager::PackageManager::Npm => "npm",
            package_manager::PackageManager::Pnpm
            | package_manager::PackageManager::Pnpm6
            | package_manager::PackageManager::Pnpm9 => "pnpm",
            package_manager::PackageManager::Yarn => "yarn",
            package_manager::PackageManager::Berry => "yarn",
            package_manager::PackageManager::Bun => "bun",
            package_manager::PackageManager::Nub { .. }
            | package_manager::PackageManager::Aube { .. } => {
                unreachable!("lockfile_manager returns a concrete package manager")
            }
        }
        .to_string();
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
        let lockfile = DirectLockfile::new(workspace_root, package_manager, &root_package_json);
        let package_graph = if skip_package_graph {
            None
        } else {
            let mut package_graph_builder =
                PackageGraphBuilder::new(workspace_root, root_package_json)
                    .with_single_package_mode(!is_multi_package)
                    .with_package_manager(package_manager.clone());
            if turbo_json.future_flags.experimental_cargo_workspaces {
                package_graph_builder = package_graph_builder.with_cargo();
            }
            if turbo_json.future_flags.experimental_python_workspaces {
                package_graph_builder = package_graph_builder.with_uv();
            }
            if turbo_json.future_flags.experimental_go_workspaces {
                package_graph_builder = package_graph_builder.with_go();
            }
            Some(package_graph_builder.build().await?)
        };

        Ok(Self {
            absolute_path: workspace_state.root.to_string(),
            is_multi_package,
            package_manager: PackageManager {
                name: package_manager_name.to_string(),
                version: package_manager_version,
            },
            graph: package_graph,
            lockfile,
            lockfile_path,
            lockfile_format,
        })
    }

    /// The package graph, or an error when the workspace was opened with
    /// `skipPackageGraph`.
    pub(crate) fn graph(&self) -> Result<&PackageGraph, Error> {
        self.graph.as_ref().ok_or(Error::PackageGraphSkipped)
    }

    pub(crate) async fn packages_internal(&self) -> Result<Vec<Package>, Error> {
        packages_from_graph(self.graph()?)
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
