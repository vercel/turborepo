use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::files::{package_json, pnpm_workspace};

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
                let pnpm_workspace = pnpm_workspace::read(&root_path.join("pnpm-workspace.yaml"))?;

                match pnpm_workspace.packages {
                    Some(packages) => packages,
                    None => return Ok(None),
                }
            }
            PackageManager::Berry | PackageManager::Npm | PackageManager::Yarn => {
                let package_json = package_json::read(&root_path.join("package.json"))?;

                match package_json.workspaces {
                    Some(workspaces) => workspaces.into(),
                    None => return Ok(None),
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
}
