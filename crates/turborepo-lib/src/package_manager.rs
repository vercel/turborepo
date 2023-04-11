use std::{
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Result};
use itertools::Itertools;
use node_semver::{Range, Version};
use regex::Regex;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::{commands::CommandBase, package_json::PackageJson, ui::UNDERLINE};

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

#[derive(Debug, Serialize)]
pub enum PackageManager {
    Berry,
    Npm,
    Pnpm,
    Pnpm6,
    Yarn,
}

impl fmt::Display for PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageManager::Berry => write!(f, "berry"),
            PackageManager::Npm => write!(f, "npm"),
            PackageManager::Pnpm => write!(f, "pnpm"),
            PackageManager::Pnpm6 => write!(f, "pnpm6"),
            PackageManager::Yarn => write!(f, "yarn"),
        }
    }
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

        Ok(*includes && !*excludes)
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

    pub fn get_package_manager(base: &mut CommandBase, pkg: &PackageJson) -> Result<Self> {
        if let Ok(Some(package_manager)) = Self::read_package_manager(pkg) {
            return Ok(package_manager);
        }

        Self::detect_package_manager(base)
    }

    fn detect_berry_or_yarn(version: &Version) -> Result<Self> {
        let berry_constraint: Range = ">=2.0.0-0".parse()?;
        if berry_constraint.satisfies(version) {
            Ok(PackageManager::Berry)
        } else {
            Ok(PackageManager::Yarn)
        }
    }

    fn detect_pnpm6_or_pnpm(version: &Version) -> Result<Self> {
        let pnpm6_constraint: Range = "<7.0.0".parse()?;
        if pnpm6_constraint.satisfies(version) {
            Ok(PackageManager::Pnpm6)
        } else {
            Ok(PackageManager::Pnpm)
        }
    }

    // Attempts to read the package manager from the package.json
    fn read_package_manager(pkg: &PackageJson) -> Result<Option<Self>> {
        let Some(package_manager) = &pkg.package_manager else {
            return Ok(None)
        };

        let (manager, version) = Self::parse_package_manager_string(package_manager)?;
        let version = version.parse()?;
        let manager = match manager {
            "npm" => Some(PackageManager::Npm),
            "yarn" => Some(Self::detect_berry_or_yarn(&version)?),
            "pnpm" => Some(Self::detect_pnpm6_or_pnpm(&version)?),
            _ => None,
        };

        Ok(manager)
    }

    fn detect_package_manager(base: &mut CommandBase) -> Result<PackageManager> {
        let mut detected_package_managers = vec![];
        let project_directory = AbsoluteSystemPathBuf::new(&base.repo_root)?;
        let npm_lockfile = project_directory
            .join_relative(RelativeSystemPathBuf::new("package-lock.json").unwrap());
        if npm_lockfile.exists() {
            detected_package_managers.push(PackageManager::Npm);
        }

        let pnpm_lockfile =
            project_directory.join_relative(RelativeSystemPathBuf::new("pnpm-lock.yaml").unwrap());
        if pnpm_lockfile.exists() {
            detected_package_managers.push(PackageManager::Pnpm);
        }

        let yarn_lockfile =
            project_directory.join_relative(RelativeSystemPathBuf::new("yarn.lock").unwrap());
        if yarn_lockfile.exists() {
            let output = Command::new("yarn").arg("--version").output()?;
            let version: Version = String::from_utf8(output.stdout)?.parse()?;
            detected_package_managers.push(Self::detect_berry_or_yarn(&version)?);
        }

        match detected_package_managers.len() {
            0 => {
                let url = base.ui.apply(
                    UNDERLINE.apply_to("https://nodejs.org/api/packages.html#packagemanager"),
                );
                Err(anyhow!(
                    "We did not find a package manager specified in your root package.json. \
                     Please set the \"packageManager\" property in your root package.json ({url}) \
                     or run `npx @turbo/codemod add-package-manager` in the root of your monorepo."
                ))
            }
            1 => Ok(detected_package_managers.pop().unwrap()),
            _ => Err(anyhow!(
                "We detected multiple package managers in your repository: {}. Please remove one \
                 of them.",
                detected_package_managers.into_iter().join(", ")
            )),
        }
    }

    fn parse_package_manager_string(manager: &str) -> Result<(&str, &str)> {
        let package_manager_pattern =
            Regex::new(r"(?P<manager>npm|pnpm|yarn)@(?P<version>\d+\.\d+\.\d+(-.+)?)")?;
        if let Some(captures) = package_manager_pattern.captures(manager) {
            let manager = captures.name("manager").unwrap().as_str();
            let version = captures.name("version").unwrap().as_str();
            Ok((manager, version))
        } else {
            Err(anyhow!(
                "We could not parse packageManager field in package.json, expected: {}, received: \
                 {}",
                package_manager_pattern,
                manager
            ))
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
