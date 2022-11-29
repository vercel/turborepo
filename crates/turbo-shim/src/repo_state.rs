use core::{
    option::{
        Option,
        Option::{None, Some},
    },
    result::Result::{Err, Ok},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use semver::Version;
use serde::Serialize;

use crate::{package_manager::PackageManager, TURBO_JSON};

pub static MINIMUM_SUPPORTED_LOCAL_TURBO: &str = "1.7.0";

#[derive(Debug, Clone, Serialize)]
pub struct RepoState {
    pub root: PathBuf,
    pub mode: RepoMode,
}

#[derive(Debug, Clone, Serialize)]
pub enum RepoMode {
    SinglePackage,
    MultiPackage,
}

impl RepoState {
    /// Infers `RepoState` from current directory.
    ///
    /// # Arguments
    ///
    /// * `current_dir`: Current working directory
    ///
    /// returns: Result<RepoState, Error>
    pub fn infer(current_dir: &Path) -> Result<Self> {
        // First we look for a `turbo.json`. This iterator returns the first ancestor
        // that contains a `turbo.json` file.
        let root_path = current_dir
            .ancestors()
            .find(|p| fs::metadata(p.join(TURBO_JSON)).is_ok());

        // If that directory exists, then we figure out if there are workspaces defined
        // in it NOTE: This may change with multiple `turbo.json` files
        if let Some(root_path) = root_path {
            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace = pnpm.get_workspace_globs(root_path).is_ok()
                || npm.get_workspace_globs(root_path).is_ok();

            let mode = if is_workspace {
                RepoMode::MultiPackage
            } else {
                RepoMode::SinglePackage
            };

            return Ok(Self {
                root: root_path.to_path_buf(),
                mode,
            });
        }

        // What we look for next is a directory that contains a `package.json`.
        let potential_roots = current_dir
            .ancestors()
            .filter(|path| fs::metadata(path.join("../../../package.json")).is_ok());

        let mut first_package_json_dir = None;
        // We loop through these directories and see if there are workspaces defined in
        // them, either in the `package.json` or `pnm-workspaces.yml`
        for dir in potential_roots {
            if first_package_json_dir.is_none() {
                first_package_json_dir = Some(dir)
            }

            let pnpm = PackageManager::Pnpm;
            let npm = PackageManager::Npm;
            let is_workspace =
                pnpm.get_workspace_globs(dir).is_ok() || npm.get_workspace_globs(dir).is_ok();

            if is_workspace {
                return Ok(Self {
                    root: dir.to_path_buf(),
                    mode: RepoMode::MultiPackage,
                });
            }
        }

        // Finally, if we don't detect any workspaces, go to the first `package.json`
        // and use that in single package mode.
        let root = first_package_json_dir
            .ok_or_else(|| {
                anyhow!(
                    "Unable to find `{}` or `package.json` in current path",
                    TURBO_JSON
                )
            })?
            .to_path_buf();

        Ok(Self {
            root,
            mode: RepoMode::SinglePackage,
        })
    }

    pub fn infer_local_turbo_version(&self) -> Result<Option<Version>> {
        let package_managers = vec![
            PackageManager::Npm,
            PackageManager::Pnpm,
            PackageManager::Yarn,
        ];

        let mut found_version = None;
        let mut found_package_manager = None;
        for package_manager in package_managers {
            let version = package_manager.get_local_turbo_version(&self.root);
            match (&found_version, version) {
                (Some(found_version), Some(version)) => {
                    if found_version != &version {
                        return Err(anyhow!(
                            "Multiple versions of turbo found in the repo from different package \
                             managers: {} from {} and {} from {}",
                            found_version,
                            found_package_manager.unwrap(),
                            version,
                            package_manager
                        ));
                    }
                }
                (None, Some(version)) => {
                    found_version = Some(version);
                    found_package_manager = Some(package_manager);
                }
                _ => {}
            }
        }

        Ok(found_version)
    }
}
