use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PnpmWorkspaces {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    pub workspaces: Vec<String>,
}

pub enum PackageManager {
    #[allow(dead_code)]
    Berry,
    Npm,
    Pnpm,
    #[allow(dead_code)]
    Pnpm6,
    #[allow(dead_code)]
    Yarn,
}

#[derive(Debug)]
pub struct Globs {
    #[allow(dead_code)]
    inclusions: Vec<PathBuf>,
    #[allow(dead_code)]
    exclusions: Vec<PathBuf>,
}

impl PackageManager {
    /// Returns a list of globs for the package workspace.
    /// NOTE: We return a `Vec<PathBuf>` instead of a `GlobSet` because we
    /// may need to iterate through these globs and a `GlobSet` doesn't allow
    /// that.
    ///
    /// # Arguments
    ///
    /// * `root_path`:
    ///
    /// returns: Result<Globs, Error>
    ///
    /// # Examples
    ///
    /// ```
    /// ```
    pub fn get_workspace_globs(&self, root_path: &Path) -> Result<Globs> {
        let globs = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let workspace_yaml = fs::read_to_string(root_path.join("pnpm-workspace.yaml"))?;
                let workspaces: PnpmWorkspaces = serde_yaml::from_str(&workspace_yaml)?;
                if workspaces.packages.is_empty() {
                    return Err(anyhow!(
                        "pnpm-workspace.yaml: no packages found. Turborepo requires pnpm \
                         workspaces and thus packages to be defined in the root \
                         pnpm-workspace.yaml"
                    ));
                } else {
                    workspaces.packages
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json_text = fs::read_to_string(root_path.join("package.json"))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)?;

                if package_json.workspaces.is_empty() {
                    return Err(anyhow!(
                        "package.json: no packages found. Turborepo requires packages to be \
                         defined in the root package.json"
                    ));
                } else {
                    package_json.workspaces
                }
            }
        };

        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        for glob in globs {
            if let Some(exclusion) = glob.strip_prefix('!') {
                exclusions.push(PathBuf::from(exclusion.to_string()));
            } else {
                inclusions.push(PathBuf::from(glob));
            }
        }

        Ok(Globs {
            inclusions,
            exclusions,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_get_workspace_globs() {
        let package_manager = PackageManager::Npm;
        let globs = package_manager
            .get_workspace_globs(Path::new("../examples/basic"))
            .unwrap();

        assert_eq!(
            globs.inclusions,
            vec![PathBuf::from("apps/*"), PathBuf::from("packages/*")]
        );
    }
}
