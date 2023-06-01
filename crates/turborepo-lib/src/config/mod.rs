mod client;
mod env;
mod repo;
mod turbo;
mod user;

use std::path::PathBuf;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
pub use client::{ClientConfig, ClientConfigLoader};
#[cfg(not(windows))]
use dirs_next::config_dir;
// Go's xdg implementation uses FOLDERID_LocalAppData for config home
// https://github.com/adrg/xdg/blob/master/paths_windows.go#L28
// Rust xdg implementations uses FOLDERID_RoamingAppData for config home
// We use cache_dir so we can find the config dir that the Go code uses
#[cfg(windows)]
use dirs_next::data_local_dir as config_dir;
pub use env::MappedEnvironment;
pub use repo::{get_repo_config_path, RepoConfig, RepoConfigLoader};
use serde::Serialize;
pub use turbo::{SpacesJson, TurboJson};
pub use user::{UserConfig, UserConfigLoader};

pub fn default_user_config_path() -> Result<Utf8PathBuf> {
    Ok(Utf8PathBuf::try_from(
        config_dir()
            .map(|p| p.join("turborepo").join("config.json"))
            .context("default config path not found")?,
    )?)
}

#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|p| p.join("turborepo"))
}

fn write_to_disk<T>(path: &Utf8Path, config: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent_dir) = path.parent() {
        std::fs::create_dir_all(parent_dir)?;
    }
    let config_file = std::fs::File::create(path)?;
    serde_json::to_writer_pretty(&config_file, &config)?;
    config_file.sync_all()?;
    Ok(())
}
