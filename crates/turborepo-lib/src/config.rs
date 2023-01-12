use std::path::{Path, PathBuf};

use anyhow::Result;
use config::{Config, Environment};
// Go's xdg implementation uses FOLDERID_LocalAppData for config home
// https://github.com/adrg/xdg/blob/master/paths_windows.go#L28
// Rust xdg implementations uses FOLDERID_RoamingAppData for config home
// We use cache_dir so we can find the config dir that the Go code uses
#[cfg(windows)]
use dirs_next::cache_dir as config_dir;
#[cfg(not(windows))]
use dirs_next::config_dir;
use serde::{Deserialize, Serialize};

pub fn default_user_config_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("turborepo").join("config.json"))
}

pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|p| p.join("turborepo"))
}

// we probably need a wrapper struct that holds the file config so we don't
// write env/cli values to file

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
struct UserConfigInner {
    token: Option<String>,
}

pub struct UserConfig {
    disk_config: UserConfigInner,
    config: UserConfigInner,
    path: PathBuf,
}

impl UserConfig {
    pub fn load(path: &Path, token: Option<&str>) -> Result<Self> {
        let raw_disk_config = Config::builder()
            .add_source(
                config::File::with_name(path.to_string_lossy().as_ref())
                    .format(config::FileFormat::Json),
            )
            .build()?;

        let config = Config::builder()
            .add_source(raw_disk_config.clone())
            .add_source(Environment::with_prefix("turbo"))
            .set_override_option("token", token)?
            .build()?
            .try_deserialize()?;

        let disk_config: UserConfigInner = raw_disk_config.try_deserialize()?;

        Ok(Self {
            disk_config,
            config,
            path: path.to_path_buf(),
        })
    }

    pub fn token(&self) -> Option<&str> {
        self.config.token.as_deref()
    }

    #[allow(dead_code)]
    pub fn set_token(&mut self, token: Option<String>) -> Result<()> {
        self.disk_config.token = token.clone();
        self.config.token = token;
        self.write_to_disk()
    }

    fn write_to_disk(&self) -> Result<()> {
        // TODO make sure that containing dir is created
        let config_file = std::fs::File::create(&self.path)?;
        serde_json::to_writer_pretty(&config_file, &self.disk_config)?;
        config_file.sync_all()?;
        Ok(())
    }
}
