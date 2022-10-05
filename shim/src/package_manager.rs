use crate::paths::GlobWalker;
use anyhow::{anyhow, Result};
use globset::{Glob, GlobSetBuilder};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct PnpmWorkspaces {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    pub workspaces: Vec<String>,
}

pub enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
}

#[derive(Debug)]
pub struct Globs {
    inclusions: Vec<PathBuf>,
    exclusions: Vec<PathBuf>,
}

impl PackageManager {
    /// Returns a list of globs for the package workspace.
    /// NOTE: We return a `Vec<PathBuf>` instead of a `GlobSet` because we
    /// may need to iterate through these globs and a `GlobSet` doesn't allow that.
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
    ///
    /// ```
    pub fn get_workspace_globs(&self, root_path: &Path) -> Result<Globs> {
        let globs = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let workspace_yaml = fs::read_to_string(root_path.join("pnpm-workspace.yaml"))?;
                let workspaces: PnpmWorkspaces = serde_yaml::from_str(&workspace_yaml)?;
                if workspaces.packages.is_empty() {
                    return Err(anyhow!("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml"));
                } else {
                    workspaces.packages
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json_text = fs::read_to_string(root_path.join("package.json"))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)?;

                if package_json.workspaces.is_empty() {
                    return Err(anyhow!("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml"));
                } else {
                    package_json.workspaces
                }
            }
        };

        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        for glob in globs {
            if glob.starts_with("!") {
                exclusions.push(PathBuf::from(glob[1..].to_string()));
            } else {
                inclusions.push(PathBuf::from(glob));
            }
        }

        Ok(Globs {
            inclusions,
            exclusions,
        })
    }

    /// Returns a `GlobSet` that matches the paths that should be ignored.
    ///
    /// # Arguments
    ///
    /// * `root_path`:
    ///
    /// returns: Result<<unknown>, Error>
    ///
    fn get_workspace_ignores(&self, root_path: &Path) -> Result<GlobSetBuilder> {
        match self {
            PackageManager::Berry => {
                // Matches upstream values:
                // Key code: https://github.com/yarnpkg/berry/blob/8e0c4b897b0881878a1f901230ea49b7c8113fbe/packages/yarnpkg-core/sources/Workspace.ts#L64-L70
                let mut builder = GlobSetBuilder::new();
                builder.add(Glob::new("**/node_modules")?);
                builder.add(Glob::new("**/.git")?);
                builder.add(Glob::new("**/.yarn")?);

                Ok(builder)
            }
            PackageManager::Npm => {
                // Matches upstream values:
                // function: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L73
                // key code: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L90-L96
                // call site: https://github.com/npm/cli/blob/7a858277171813b37d46a032e49db44c8624f78f/lib/workspaces/get-workspaces.js#L14

                let mut builder = GlobSetBuilder::new();
                builder.add(Glob::new("**/node_modules/**")?);

                Ok(builder)
            }
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                // Matches upstream values:
                // function: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L27
                // key code: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L30
                // call site: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-workspace-packages/src/index.ts#L32-L39
                let mut builder = GlobSetBuilder::new();
                builder.add(Glob::new("**/node_modules/**")?);
                builder.add(Glob::new("**/bower_components/**")?);

                Ok(builder)
            }
            PackageManager::Yarn => {
                // function: https://github.com/yarnpkg/yarn/blob/3119382885ea373d3c13d6a846de743eca8c914b/src/config.js#L799

                // Yarn is unique in ignore patterns handling.
                // The only time it does globbing is for package.json or yarn.json and it scopes the search to each workspace.
                // For example: `apps/*/node_modules/**/+(package.json|yarn.json)`
                // The `extglob` `+(package.json|yarn.json)` (from micromatch) after node_modules/** is redundant.

                let globs = self.get_workspace_globs(root_path)?;

                let mut builder = GlobSetBuilder::new();
                for mut glob_path in globs.inclusions {
                    glob_path.push("/node_modules/**");

                    builder.add(Glob::new(
                        glob_path
                            .to_str()
                            .ok_or_else(|| anyhow!("Path is invalid unicode"))?,
                    )?);
                }

                Ok(builder)
            }
        }
    }

    /// Returns a list of paths of package.json files for the current repository.
    ///
    /// # Arguments
    ///
    /// * `root_path`: The root path of the repository
    ///
    /// returns: Result<Vec<DirEntry, Global>, Error>
    ///
    fn get_workspaces(&self, root_path: &Path) -> Result<Vec<PathBuf>> {
        let workspace_globs = self.get_workspace_globs(root_path)?;

        let mut workspace_globs_builder = GlobSetBuilder::new();

        for mut path in workspace_globs.inclusions {
            path.push("package.json");
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow!("Path is invalid unicode"))?;

            // We need to push on the root for the globbing to work properly
            let root_str = root_path
                .to_str()
                .ok_or_else(|| anyhow!("Path is invalid unicode"))?;

            workspace_globs_builder.add(Glob::new(&format!("{}/{}", root_str, path_str))?);
        }
        let workspace_globs_set = workspace_globs_builder.build()?;

        let mut ignores_builder = self.get_workspace_ignores(root_path)?;
        for mut path in workspace_globs.exclusions {
            path.push("package.json");
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow!("Path is invalid unicode"))?;

            // We need to push on the root for the globbing to work properly
            let root_str = root_path
                .to_str()
                .ok_or_else(|| anyhow!("Path is invalid unicode"))?;
            ignores_builder.add(Glob::new(&format!("{}/{}", root_str, path_str))?);
        }
        let ignores_set = ignores_builder.build()?;

        let glob_walker = GlobWalker::new(root_path, workspace_globs_set, ignores_set);

        glob_walker
            .map(|dir_entry| dir_entry.map(|e| e.into_path()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn test_get_workspace_globs() {
        let package_manager = PackageManager::Npm;
        let globs = package_manager
            .get_workspace_globs(&Path::new("../examples/basic"))
            .unwrap();

        assert_eq!(
            globs.inclusions,
            vec![PathBuf::from("apps/*"), PathBuf::from("packages/*")]
        );
    }

    #[test]
    fn test_get_workspace_ignores() {
        let package_manager = PackageManager::Npm;
        let globs = package_manager
            .get_workspace_ignores(&Path::new("../examples/basic"))
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(globs.is_match("node_modules/foo"), true);
        assert_eq!(globs.is_match("bar.js"), false);
    }

    #[test]
    fn test_get_workspaces() {
        let package_manager = PackageManager::Npm;
        let home_path = Path::new("../examples/basic");
        let workspaces = package_manager.get_workspaces(&home_path).unwrap();

        // This is not ideal, but we can't compare with an expected set of paths because
        // the paths are absolute and therefore depend on who's running the test.
        for dir_entry in workspaces {
            assert_eq!(dir_entry.file_name().unwrap(), OsStr::new("package.json"))
        }
    }
}
