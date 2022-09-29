use crate::paths::AbsolutePath;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize)]
struct PnpmWorkspaces {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    pub workspaces: Vec<String>,
}

enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
}

impl PackageManager {
    fn get_workspace_globs(&self, root_path: &AbsolutePath) -> Result<Vec<String>> {
        match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let workspace_yaml = fs::read_to_string(root_path.join("pnpm-workspace.yaml"))?;
                let workspaces: PnpmWorkspaces = serde_yaml::from_str(&workspace_yaml)?;
                if workspaces.packages.is_empty() {
                    Err(anyhow!("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml"))
                } else {
                    Ok(workspaces.packages)
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json_text = fs::read_to_string(root_path.join("package.json"))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)?;

                if package_json.workspaces.is_empty() {
                    Err(anyhow!("pnpm-workspace.yaml: no packages found. Turborepo requires pnpm workspaces and thus packages to be defined in the root pnpm-workspace.yaml"))
                } else {
                    Ok(package_json.workspaces)
                }
            }
        }
    }

    fn get_workspace_ignores(&self, root_path: &AbsolutePath) -> Result<Vec<String>> {
        match self {
            PackageManager::Berry => {
                // Matches upstream values:
                // Key code: https://github.com/yarnpkg/berry/blob/8e0c4b897b0881878a1f901230ea49b7c8113fbe/packages/yarnpkg-core/sources/Workspace.ts#L64-L70
                Ok(vec![
                    "**/node_modules".to_string(),
                    "**/.git".to_string(),
                    "**/.yarn".to_string(),
                ])
            }
            PackageManager::Npm => {
                // Matches upstream values:
                // function: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L73
                // key code: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L90-L96
                // call site: https://github.com/npm/cli/blob/7a858277171813b37d46a032e49db44c8624f78f/lib/workspaces/get-workspaces.js#L14
                Ok(vec!["**/node_modules/**".to_string()])
            }
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                // Matches upstream values:
                // function: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L27
                // key code: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L30
                // call site: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-workspace-packages/src/index.ts#L32-L39
                Ok(vec![
                    "**/node_modules/**".to_string(),
                    "**/bower_components/**".to_string(),
                ])
            }
            PackageManager::Yarn => {
                // function: https://github.com/yarnpkg/yarn/blob/3119382885ea373d3c13d6a846de743eca8c914b/src/config.js#L799

                // Yarn is unique in ignore patterns handling.
                // The only time it does globbing is for package.json or yarn.json and it scopes the search to each workspace.
                // For example: `apps/*/node_modules/**/+(package.json|yarn.json)`
                // The `extglob` `+(package.json|yarn.json)` (from micromatch) after node_modules/** is redundant.

                let globs = self.get_workspace_globs(root_path)?;

                Ok(globs
                    .into_iter()
                    .map(|path| {
                        format!(
                            "{}/node_modules/**",
                            path.strip_suffix("/").unwrap_or_else(|| &path)
                        )
                    })
                    .collect())
            }
        }
    }
}
