use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageJson {
    name: String,
    version: String,
    pub scripts: BTreeMap<String, String>,
    dependencies: BTreeMap<String, String>,
    dev_dependencies: BTreeMap<String, String>,
    optional_dependencies: BTreeMap<String, String>,
    peer_dependencies: BTreeMap<String, String>,
    pub package_manager: Option<String>,
    os: Vec<String>,
    workspaces: Vec<String>,
    private: bool,
    package_json_path: Option<AnchoredSystemPathBuf>,
    dir: Option<AnchoredSystemPathBuf>,
    internal_deps: Vec<String>,
    unresolved_external_deps: BTreeMap<String, String>,
    #[serde(alias = "turbo")]
    pub(crate) legacy_turbo_config: Option<()>,
    #[serde(flatten)]
    raw_json: HashMap<String, serde_json::Value>,
}

impl PackageJson {
    pub fn load(path: &AbsoluteSystemPathBuf) -> Result<PackageJson> {
        let contents = std::fs::read_to_string(path)?;
        let package_json: PackageJson = serde_json::from_str(&contents)?;
        Ok(package_json)
    }
}
