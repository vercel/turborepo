use std::result::Result;

use napi::Status;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};
use turborepo_repository::{
    inference::{self, RepoMode as WorkspaceType, RepoState as WorkspaceState},
    package_manager,
};

use crate::{Package, Workspace};

/// This module is used to isolate code with defined errors
/// from code in lib.rs that needs to have errors coerced to strings /
/// napi::Error for return to javascript.
/// Dividing the source code up this way allows us to be stricter here, and have
/// the strictness relaxed only at the boundary.

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("Failed to resolve starting path from {path}: {path_error}")]
    StartingPath { path_error: PathError, path: String },
    #[error(transparent)]
    Inference(#[from] inference::Error),
    #[error("Failed to resolve package manager from {path}: {error}")]
    PackageManager {
        error: String,
        path: AbsoluteSystemPathBuf,
    },
    #[error("Failed to discover packages from root {workspace_root}: {error}")]
    PackageJsons {
        error: package_manager::Error,
        workspace_root: AbsoluteSystemPathBuf,
    },
    #[error("Package directory {0} has no parent")]
    MissingParent(AbsoluteSystemPathBuf),
}

impl From<Error> for napi::Error<Status> {
    fn from(value: Error) -> Self {
        napi::Error::from_reason(value.to_string())
    }
}

impl Workspace {
    pub(crate) async fn find_internal(path: Option<String>) -> Result<Self, Error> {
        let reference_dir = path
            .map(|path| {
                AbsoluteSystemPathBuf::from_cwd(&path).map_err(|path_error| Error::StartingPath {
                    path: path.clone(),
                    path_error,
                })
            })
            .unwrap_or_else(|| {
                AbsoluteSystemPathBuf::cwd().map_err(|path_error| Error::StartingPath {
                    path: "".to_string(),
                    path_error,
                })
            })?;
        let workspace_state = WorkspaceState::infer(&reference_dir)?;
        let is_multi_package = workspace_state.mode == WorkspaceType::MultiPackage;
        Ok(Self {
            absolute_path: workspace_state.root.to_string(),
            workspace_state,
            is_multi_package,
        })
    }

    pub(crate) async fn packages_internal(&self) -> Result<Vec<Package>, Error> {
        // Note: awkward error handling because we memoize the error from package
        // manager discovery. That probably isn't the best design. We should
        // address it when we decide how we want to handle possibly finding a
        // repo root but not finding a package manager.
        let package_manager = self
            .workspace_state
            .package_manager
            .as_ref()
            .map_err(|error| Error::PackageManager {
                error: error.to_string(),
                path: self.workspace_state.root.clone(),
            })?;
        let package_manager = *package_manager;
        let workspace_root = self.workspace_state.root.clone();
        let package_json_paths =
            tokio::task::spawn(async move { package_manager.get_package_jsons(&workspace_root) })
                .await
                .expect("package enumeration should not crash")
                .map_err(|error| Error::PackageJsons {
                    error,
                    workspace_root: self.workspace_state.root.clone(),
                })?;
        let packages = package_json_paths
            .map(|path| {
                path.parent()
                    .map(|package_path| Package::new(&self.workspace_state.root, package_path))
                    .ok_or_else(|| Error::MissingParent(path.to_owned()))
            })
            .collect::<Result<Vec<Package>, Error>>()?;
        Ok(packages)
    }
}
