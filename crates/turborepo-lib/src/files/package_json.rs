use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageJson {
    pub version: Option<String>,
    pub workspaces: Option<Workspaces>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
pub enum Workspaces {
    TopLevel(Vec<String>),

    // This only works in old npm.
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

impl Default for PackageJson {
    fn default() -> Self {
        Self {
            version: None,
            workspaces: Some(Workspaces::TopLevel(vec![])),
        }
    }
}

pub fn read(path: PathBuf) -> Result<PackageJson> {
    let package_json_string = fs::read_to_string(path)?;
    let package_json = serde_json::from_str(&package_json_string)?;

    Ok(package_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nested_workspace_globs() -> Result<()> {
        let top_level: PackageJson = serde_json::from_str("{ \"workspaces\": [\"packages/**\"]}")?;
        assert_eq!(top_level.workspaces.unwrap().as_ref(), vec!["packages/**"]);
        let nested: PackageJson =
            serde_json::from_str("{ \"workspaces\": {\"packages\": [\"packages/**\"]}}")?;
        assert_eq!(nested.workspaces.unwrap().as_ref(), vec!["packages/**"]);
        Ok(())
    }
}
