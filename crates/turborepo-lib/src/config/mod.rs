mod env;

use std::{collections::HashMap, path::PathBuf};

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

/// Configuration options for loading a UserConfig object
#[derive(Debug, Clone)]
pub struct UserConfigLoader {
    path: PathBuf,
    token: Option<String>,
    environment: Option<HashMap<String, String>>,
}

impl UserConfig {
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

impl UserConfigLoader {
    /// Creates a loader that will load the config file at the given path
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            token: None,
            environment: None,
        }
    }

    /// Set an override for token that the user provided via the command line
    #[allow(dead_code)]
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// Use the given environment map instead of querying the processes
    /// environment
    #[allow(dead_code)]
    pub fn with_environment(mut self, environment: Option<HashMap<String, String>>) -> Self {
        self.environment = environment;
        self
    }

    /// Loads the user config using settings of the loader
    pub fn load(self) -> Result<UserConfig> {
        let Self {
            path,
            token,
            environment,
        } = self;
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
            .add_source(Environment::with_prefix("turbo").source(environment))
            .set_override_option("token", token)?
            .build()?
            .try_deserialize()?;

        let disk_config: UserConfigInner = raw_disk_config.try_deserialize()?;

        Ok(UserConfig {
            disk_config,
            config,
            path,
        })
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use tempfile::{NamedTempFile, TempDir};

    use super::*;

    #[test]
    fn test_handles_non_existent_path() -> Result<()> {
        let config_dir = TempDir::new()?;
        let mut config_path = config_dir.path().to_path_buf();
        config_path.push("turbo");
        config_path.push("config.json");
        let loader = UserConfigLoader::new(config_path.clone());
        let mut config = loader.clone().load()?;
        assert_eq!(config.token(), None);
        config.set_token(Some("foo".to_string()))?;
        let new_config = loader.load()?;
        assert_eq!(new_config.token(), Some("foo"));
        Ok(())
    }

    #[test]
    fn test_disk_value_preserved() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;
        let loader =
            UserConfigLoader::new(config_file.path().to_path_buf()).with_token(Some("bar".into()));
        let config = loader.load()?;
        assert_eq!(config.token(), Some("bar"));
        config.write_to_disk()?;
        let new_config = UserConfigLoader::new(config_file.path().to_path_buf()).load()?;
        assert_eq!(new_config.token(), Some("foo"));
        Ok(())
    }

    #[test]
    fn test_env_var_trumps_disk() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;
        let env = {
            let mut map = HashMap::new();
            map.insert("TURBO_TOKEN".into(), "bar".into());
            map
        };
        let config = UserConfigLoader::new(config_file.path().to_path_buf())
            .with_environment(Some(env))
            .load()?;
        assert_eq!(config.token(), Some("bar"));
        Ok(())
    }
}
