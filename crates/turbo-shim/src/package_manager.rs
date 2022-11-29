use std::{
    fmt::{Display, Formatter},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use semver::Version;
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

impl Display for PackageManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PackageManager::Berry => "yarn berry",
                PackageManager::Npm => "npm",
                PackageManager::Pnpm => "pnpm",
                PackageManager::Pnpm6 => "pnpm v6",
                PackageManager::Yarn => "yarn",
            }
        )
    }
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
                let workspace_yaml =
                    fs::read_to_string(root_path.join("../../../pnpm-workspace.yaml"))?;
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
                let package_json_text =
                    fs::read_to_string(root_path.join("../../../package.json"))?;
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

    pub fn get_local_turbo_version(&self, repo_root: &Path) -> Option<Version> {
        match self {
            PackageManager::Npm => {
                let package_lock_path = repo_root.join("package-lock.json");
                let package_lock_text = fs::read_to_string(package_lock_path).ok()?;
                let package_lock: serde_json::Value =
                    serde_json::from_str(&package_lock_text).ok()?;

                package_lock
                    .get("packages")?
                    .get("node_modules/turbo")?
                    .get("version")?
                    .as_str()?
                    .parse()
                    .ok()
            }
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let pnpm_lock_path = repo_root.join("../../../pnpm-lock.yaml");
                let pnpm_lock_text = fs::read_to_string(pnpm_lock_path).ok()?;
                let pnpm_lock: serde_yaml::Value = serde_yaml::from_str(&pnpm_lock_text).ok()?;

                let mut package_entries = pnpm_lock.get("packages")?.as_mapping()?.into_iter();

                // Find first key that starts with `/turbo/ and return the version
                let version_str = package_entries.find_map(|(key, _)| {
                    key.as_str().and_then(|key| key.strip_prefix("/turbo/"))
                })?;

                version_str.parse().ok()
            }
            PackageManager::Berry | PackageManager::Yarn => {
                let yarn_lock_path = repo_root.join("yarn.lock");
                let yarn_lock_text = fs::read_to_string(yarn_lock_path).ok()?;
                let yarn_lock_entries = yarn_lock_parser::parse_str(&yarn_lock_text).ok()?;

                yarn_lock_entries
                    .into_iter()
                    .find(|entry| entry.name == "turbo")?
                    .version
                    .parse()
                    .ok()
            }
        }
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
