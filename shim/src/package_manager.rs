use crate::paths::{AbsolutePath, GlobWalker};
use anyhow::{anyhow, Result};
use glob::Pattern;
use serde::Deserialize;
use std::env::current_exe;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::DirEntry;

#[derive(Debug, Deserialize)]
struct PnpmWorkspaces {
    pub packages: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    pub workspaces: Vec<PathBuf>,
}

enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
}

impl PackageManager {
    fn get_workspace_globs(&self, root_path: &AbsolutePath) -> Result<Vec<PathBuf>> {
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

    fn get_workspace_ignores(&self, root_path: &AbsolutePath) -> Result<Vec<Pattern>> {
        match self {
            PackageManager::Berry => {
                // Matches upstream values:
                // Key code: https://github.com/yarnpkg/berry/blob/8e0c4b897b0881878a1f901230ea49b7c8113fbe/packages/yarnpkg-core/sources/Workspace.ts#L64-L70
                Ok(vec![
                    Pattern::new("**/node_modules")?,
                    Pattern::new("**/.git")?,
                    Pattern::new("**/.yarn")?,
                ])
            }
            PackageManager::Npm => {
                // Matches upstream values:
                // function: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L73
                // key code: https://github.com/npm/map-workspaces/blob/a46503543982cb35f51cc2d6253d4dcc6bca9b32/lib/index.js#L90-L96
                // call site: https://github.com/npm/cli/blob/7a858277171813b37d46a032e49db44c8624f78f/lib/workspaces/get-workspaces.js#L14
                Ok(vec![Pattern::new("**/node_modules/**")?])
            }
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                // Matches upstream values:
                // function: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L27
                // key code: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-packages/src/index.ts#L30
                // call site: https://github.com/pnpm/pnpm/blob/d99daa902442e0c8ab945143ebaf5cdc691a91eb/packages/find-workspace-packages/src/index.ts#L32-L39
                Ok(vec![
                    Pattern::new("**/node_modules/**")?,
                    Pattern::new("**/bower_components/**")?,
                ])
            }
            PackageManager::Yarn => {
                // function: https://github.com/yarnpkg/yarn/blob/3119382885ea373d3c13d6a846de743eca8c914b/src/config.js#L799

                // Yarn is unique in ignore patterns handling.
                // The only time it does globbing is for package.json or yarn.json and it scopes the search to each workspace.
                // For example: `apps/*/node_modules/**/+(package.json|yarn.json)`
                // The `extglob` `+(package.json|yarn.json)` (from micromatch) after node_modules/** is redundant.

                let globs = self.get_workspace_globs(root_path)?;

                globs
                    .into_iter()
                    .map(|path| {
                        let mut path = PathBuf::from(path);
                        path.push("/node_modules/**");
                        Pattern::new(
                            path.to_str()
                                .ok_or_else(|| anyhow!("Path is invalid unicode"))?,
                        )
                        .map_err(|e| anyhow!("Error creating pattern: {}", e))
                    })
                    .collect::<Result<Vec<Pattern>>>()
            }
        }
    }

    fn get_workspaces(&self, root_path: &AbsolutePath) -> Result<Vec<DirEntry>> {
        let globs = self.get_workspace_globs(root_path)?;
        let just_jsons = globs
            .into_iter()
            .map(|mut path| {
                path.push("package.json");
                let path_string = path
                    .to_str()
                    .ok_or_else(|| anyhow!("Path is invalid unicode"))?;

                Ok(Pattern::new(path_string)?)
            })
            .collect::<Result<Vec<_>>>()?;

        let ignores = self.get_workspace_ignores(root_path)?;

        println!("JUST JSONS: {:?}", just_jsons);
        println!("IGNORES: {:?}", ignores);

        let glob_walker = GlobWalker::new(root_path, just_jsons, ignores);

        glob_walker.collect()
    }
}

#[test]
fn test_get_workspace_globs() {
    let package_manager = PackageManager::Npm;
    let globs = package_manager
        .get_workspace_globs(&Path::new("../examples/basic"))
        .unwrap();

    assert_eq!(
        globs,
        vec![PathBuf::from("apps/*"), PathBuf::from("packages/*")]
    );
}

#[test]
fn test_get_workspace_ignores() {
    let package_manager = PackageManager::Npm;
    let globs = package_manager
        .get_workspace_ignores(&Path::new("../examples/basic"))
        .unwrap();

    assert_eq!(globs, vec![Pattern::new("**/node_modules/**").unwrap()]);
}

#[test]
fn test_get_workspaces() {
    let mut home_path = current_exe().unwrap();
    home_path.pop();
    home_path.push("../../../../examples/basic/apps/docs/package.json");
    println!("{:?}", home_path);
    let pattern = Pattern::new("apps/*/package.json").unwrap();
    println!("{}", pattern.matches_path(&home_path));
    // let package_manager = PackageManager::Npm;
    // let mut home_path = current_exe().unwrap();
    // home_path.pop();
    // home_path.push("../../../../examples/basic");
    // let workspaces = package_manager.get_workspaces(&home_path).unwrap();
    //
    // println!("Workspaces: {:?}", workspaces);
}
