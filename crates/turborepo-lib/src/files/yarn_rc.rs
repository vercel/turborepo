use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct YarnRc {
    pub pnp_unplugged_folder: PathBuf,
}

impl Default for YarnRc {
    fn default() -> Self {
        Self {
            pnp_unplugged_folder: [".yarn", "unplugged"].iter().collect(),
        }
    }
}

pub fn read(path: &PathBuf) -> Result<YarnRc> {
    let yarn_rc_string = fs::read_to_string(path)?;
    let yarn_rc = serde_yaml::from_str(&yarn_rc_string)?;

    Ok(yarn_rc)
}
