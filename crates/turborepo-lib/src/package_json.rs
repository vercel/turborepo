use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackageJson {
    pub package_manager: Option<String>,
}

impl PackageJson {
    pub fn load(path: &AbsoluteSystemPath) -> Result<PackageJson> {
        let contents = std::fs::read_to_string(path)?;
        let package_json: PackageJson = serde_json::from_str(&contents)?;
        Ok(package_json)
    }
}
