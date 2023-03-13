use std::{fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PnpmWorkspace {
    pub packages: Option<Vec<String>>,
}

pub fn read(path: PathBuf) -> Result<PnpmWorkspace> {
    let pnpm_workspace_string = fs::read_to_string(path)?;
    let pnpm_workspace = serde_yaml::from_str(&pnpm_workspace_string)?;

    Ok(pnpm_workspace)
}
