use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PnpmWorkspace {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJsonWorkspaces {
    workspaces: Workspaces,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
enum Workspaces {
    TopLevel(Vec<String>),
    Nested { packages: Vec<String> },
}

impl AsRef<[String]> for Workspaces {
    fn as_ref(&self) -> &[String] {
        match self {
            Workspaces::TopLevel(packages) => packages.as_slice(),
            Workspaces::Nested { packages } => packages.as_slice(),
        }
    }
}

impl From<Workspaces> for Vec<String> {
    fn from(value: Workspaces) -> Self {
        match value {
            Workspaces::TopLevel(packages) => packages,
            Workspaces::Nested { packages } => packages,
        }
    }
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
    pub inclusions: Vec<String>,
    pub exclusions: Vec<String>,
}

impl Globs {
    pub fn test(&self, root: PathBuf, target: PathBuf) -> Result<bool> {
        let search_value = target
            .strip_prefix(root)?
            .to_str()
            .ok_or_else(|| anyhow!("The relative path is not UTF8."))?;

        let includes = &self
            .inclusions
            .iter()
            .any(|inclusion| glob_match::glob_match(inclusion, search_value));

        let excludes = &self
            .exclusions
            .iter()
            .any(|exclusion| glob_match::glob_match(exclusion, search_value));

        Ok(*includes && !excludes)
    }
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
    /// returns: Result<Option<Globs>, Error>
    ///
    /// # Examples
    ///
    /// ```
    /// ```
    pub fn get_workspace_globs(&self, root_path: &Path) -> Result<Option<Globs>> {
        let globs = match self {
            PackageManager::Pnpm | PackageManager::Pnpm6 => {
                let workspace_yaml = fs::read_to_string(root_path.join("pnpm-workspace.yaml"))?;
                let pnpm_workspace: PnpmWorkspace = serde_yaml::from_str(&workspace_yaml)?;
                if pnpm_workspace.packages.is_empty() {
                    return Ok(None);
                } else {
                    pnpm_workspace.packages
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json_text = fs::read_to_string(root_path.join("package.json"))?;
                let package_json: PackageJsonWorkspaces = serde_json::from_str(&package_json_text)?;

                if package_json.workspaces.as_ref().is_empty() {
                    return Ok(None);
                } else {
                    package_json.workspaces.into()
                }
            }
        };

        let mut inclusions = Vec::new();
        let mut exclusions = Vec::new();

        for glob in globs {
            if let Some(exclusion) = glob.strip_prefix('!') {
                exclusions.push(exclusion.to_string());
            } else {
                inclusions.push(glob);
            }
        }

        Ok(Some(Globs {
            inclusions,
            exclusions,
        }))
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
            .get_workspace_globs(Path::new("../../examples/with-yarn"))
            .unwrap()
            .unwrap();

        assert_eq!(globs.inclusions, vec!["apps/*", "packages/*"]);
    }

    #[test]
    fn test_globs_test() {
        struct TestCase {
            globs: Globs,
            root: PathBuf,
            target: PathBuf,
            output: Result<bool>,
        }

        let tests = [TestCase {
            globs: Globs {
                inclusions: vec!["d/**".to_string()],
                exclusions: vec![],
            },
            root: PathBuf::from("/a/b/c"),
            target: PathBuf::from("/a/b/c/d/e/f"),
            output: Ok(true),
        }];

        for test in tests {
            match test.globs.test(test.root, test.target) {
                Ok(value) => assert_eq!(value, test.output.unwrap()),
                Err(value) => assert_eq!(value.to_string(), test.output.unwrap_err().to_string()),
            };
        }
    }

    #[test]
    fn test_nested_workspace_globs() -> Result<()> {
        let top_level: PackageJsonWorkspaces =
            serde_json::from_str("{ \"workspaces\": [\"packages/**\"]}")?;
        assert_eq!(top_level.workspaces.as_ref(), vec!["packages/**"]);
        let nested: PackageJsonWorkspaces =
            serde_json::from_str("{ \"workspaces\": {\"packages\": [\"packages/**\"]}}")?;
        assert_eq!(nested.workspaces.as_ref(), vec!["packages/**"]);
        Ok(())
    }
}
