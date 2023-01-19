use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use config::{Config, Environment};
#[cfg(not(windows))]
use dirs_next::config_dir;
// Go's xdg implementation uses FOLDERID_LocalAppData for config home
// https://github.com/adrg/xdg/blob/master/paths_windows.go#L28
// Rust xdg implementations uses FOLDERID_RoamingAppData for config home
// We use cache_dir so we can find the config dir that the Go code uses
#[cfg(windows)]
use dirs_next::data_local_dir as config_dir;
use serde::{Deserialize, Serialize};

pub fn default_user_config_path() -> Result<PathBuf> {
    config_dir()
        .map(|p| p.join("turborepo").join("config.json"))
        .context("default config path not found")
}

#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|p| p.join("turborepo"))
}

// Inner struct that matches the config file schema
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Default)]
struct UserConfigInner {
    token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserConfig {
    // The configuration that comes from the disk
    // We keep this as a separate value to avoid saving values that come from
    // environment variables or command line flags.
    disk_config: UserConfigInner,
    config: UserConfigInner,
    path: PathBuf,
}

impl UserConfig {
    /// Loads the user config from the given path, with token as an optional
    /// override that the user might provide via the command line.
    pub fn load(path: &Path, token: Option<&str>) -> Result<Self> {
        // We load just the disk config to make sure we don't write a config
        // value that comes from a flag or environment variable.
        let raw_disk_config = Config::builder()
            .add_source(
                config::File::with_name(path.to_string_lossy().as_ref())
                    .format(config::FileFormat::Json)
                    .required(false),
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

    #[allow(dead_code)]
    pub fn token(&self) -> Option<&str> {
        self.config.token.as_deref()
    }

    /// Set token and sync the changes to disk
    pub fn set_token(&mut self, token: Option<String>) -> Result<()> {
        self.disk_config.token = token.clone();
        self.config.token = token;
        self.write_to_disk()
    }

    fn write_to_disk(&self) -> Result<()> {
        if let Some(parent_dir) = self.path.parent() {
            std::fs::create_dir_all(parent_dir)?;
        }
        let config_file = std::fs::File::create(&self.path)?;
        serde_json::to_writer_pretty(&config_file, &self.disk_config)?;
        config_file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::{env, io::Write};

    use tempfile::{NamedTempFile, TempDir};

    use super::*;

    #[test]
    fn test_handles_non_existent_path() -> Result<()> {
        let config_dir = TempDir::new()?;
        let mut config_path = config_dir.path().to_path_buf();
        config_path.push("turbo");
        config_path.push("config.json");
        let mut config = UserConfig::load(&config_path, None)?;
        assert_eq!(config.token(), None);
        config.set_token(Some("foo".to_string()))?;
        let new_config = UserConfig::load(&config_path, None)?;
        assert_eq!(new_config.token(), Some("foo"));
        Ok(())
    }

    #[test]
    fn test_disk_value_preserved() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;
        let config = UserConfig::load(config_file.path(), Some("bar"))?;
        assert_eq!(config.token(), Some("bar"));
        config.write_to_disk()?;
        let new_config = UserConfig::load(config_file.path(), None)?;
        assert_eq!(new_config.token(), Some("foo"));
        Ok(())
    }

    #[test]
    fn test_env_var_trumps_disk() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;
        env::set_var("TURBO_TOKEN", "bar");
        let config = UserConfig::load(config_file.path(), None)?;
        assert_eq!(config.token(), Some("bar"));
        Ok(())
    }
}
