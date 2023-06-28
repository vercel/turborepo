use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub package_manager: Option<String>,
    pub dependencies: Option<BTreeMap<String, String>>,
    pub dev_dependencies: Option<BTreeMap<String, String>>,
    pub optional_dependencies: Option<BTreeMap<String, String>>,
    pub peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "turbo")]
    pub legacy_turbo_config: Option<serde_json::Value>,
    #[serde(default)]
    pub scripts: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to read package.json: {0}")]
    Io(#[from] std::io::Error),
    #[error("unable to parse package.json: {0}")]
    Json(#[from] serde_json::Error),
}

impl PackageJson {
    pub fn load(path: &AbsoluteSystemPath) -> Result<PackageJson, Error> {
        let contents = std::fs::read_to_string(path)?;
        let package_json: PackageJson = serde_json::from_str(&contents)?;
        Ok(package_json)
    }

    // Utility method for easy construction of package.json during testing
    #[cfg(test)]
    pub fn from_value(value: serde_json::Value) -> Result<PackageJson, Error> {
        let package_json: PackageJson = serde_json::from_value(value)?;
        Ok(package_json)
    }

    pub fn all_dependencies(&self) -> impl Iterator<Item = (&String, &String)> + '_ {
        self.dependencies
            .iter()
            .flatten()
            .chain(self.dev_dependencies.iter().flatten())
            .chain(self.optional_dependencies.iter().flatten())
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;

    use crate::package_json::PackageJson;

    #[test]
    fn test_legacy_turbo_config() -> Result<()> {
        let contents = r#"{"turbo": {}}"#;
        let package_json = serde_json::from_str::<PackageJson>(contents)?;

        assert!(package_json.legacy_turbo_config.is_some());

        let contents = r#"{"turbo": { "globalDependencies": [".env"] } }"#;
        let package_json = serde_json::from_str::<PackageJson>(contents)?;

        assert!(package_json.legacy_turbo_config.is_some());

        Ok(())
    }
}
