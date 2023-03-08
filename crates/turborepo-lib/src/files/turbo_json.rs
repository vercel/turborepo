use std::{fs, path::PathBuf};

use anyhow::Result;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct TurboJson {
    pub turbo_version: String,
}

impl TurboJson {
    pub fn check_version(&self, version: &str) -> Result<bool> {
        let version = Version::parse(version)?;

        let version_request = VersionReq::parse(&self.turbo_version)?;
        Ok(version_request.matches(&version))
    }
}

impl Default for TurboJson {
    fn default() -> Self {
        Self {
            turbo_version: "*".to_string(),
        }
    }
}

pub fn read(path: &PathBuf) -> Result<TurboJson> {
    let turbo_json_string = fs::read_to_string(path)?;
    let turbo_json = serde_json::from_str(&turbo_json_string)?;

    Ok(turbo_json)
}
