use std::collections::HashMap;

use config::{Config, Environment};
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use super::write_to_disk;
use crate::config::Error;

// Inner struct that matches the config file schema
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Default)]
struct UserConfigValue {
    token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserConfig {
    // The configuration that comes from the disk
    // We keep this as a separate value to avoid saving values that come from
    // environment variables or command line flags.
    disk_config: UserConfigValue,
    config: UserConfigValue,
    path: AbsoluteSystemPathBuf,
}

/// Configuration options for loading a UserConfig object
#[derive(Debug, Clone)]
pub struct UserConfigLoader {
    path: AbsoluteSystemPathBuf,
    token: Option<String>,
    environment: Option<HashMap<String, String>>,
}

impl UserConfig {
    #[allow(dead_code)]
    pub fn token(&self) -> Option<&str> {
        self.config.token.as_deref()
    }

    /// Returns where a token will be written to.
    pub fn path(&self) -> &AbsoluteSystemPath {
        &self.path
    }

    /// Set token and sync the changes to disk
    pub fn set_token(&mut self, token: Option<String>) -> Result<(), Error> {
        self.disk_config.token = token.clone();
        self.config.token = token;
        self.write_to_disk()
    }

    fn write_to_disk(&self) -> Result<(), Error> {
        write_to_disk(&self.path, &self.disk_config)
    }
}

impl UserConfigLoader {
    /// Creates a loader that will load the config file at the given path
    pub fn new(path: impl Into<AbsoluteSystemPathBuf>) -> Self {
        Self {
            path: path.into(),
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
    pub fn load(self) -> Result<UserConfig, Error> {
        let Self {
            path,
            token,
            environment,
        } = self;
        // We load just the disk config to make sure we don't write a config
        // value that comes from a flag or environment variable.
        let raw_disk_config = Config::builder()
            .add_source(
                config::File::with_name(path.as_str())
                    .format(config::FileFormat::Json)
                    .required(false),
            )
            .build()?;

        let config = Config::builder()
            .add_source(raw_disk_config.clone())
            .add_source(Environment::with_prefix("TURBO").source(environment.clone()))
            .add_source(Environment::with_prefix("VERCEL_ARTIFACTS").source(environment))
            .set_override_option("token", token)?
            .build()?
            .try_deserialize()?;

        let disk_config: UserConfigValue = raw_disk_config.try_deserialize()?;

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

    use anyhow::Result;
    use tempfile::{NamedTempFile, TempDir};

    use super::*;

    #[test]
    fn test_handles_non_existent_path() -> Result<()> {
        let config_dir = TempDir::new()?;
        let mut config_path = config_dir.path().to_path_buf();
        config_path.push("turbo");
        config_path.push("config.json");
        let loader = UserConfigLoader::new(AbsoluteSystemPathBuf::try_from(config_path).unwrap());
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
            UserConfigLoader::new(AbsoluteSystemPathBuf::try_from(config_file.path()).unwrap())
                .with_token(Some("bar".into()));
        let config = loader.load()?;
        assert_eq!(config.token(), Some("bar"));
        config.write_to_disk()?;
        let new_config =
            UserConfigLoader::new(AbsoluteSystemPathBuf::try_from(config_file.path()).unwrap())
                .load()?;
        assert_eq!(new_config.token(), Some("foo"));
        Ok(())
    }

    static TOKEN_ENV_VARS: [&str; 2] = ["TURBO_TOKEN", "VERCEL_ARTIFACTS_TOKEN"];

    #[test]
    fn test_env_var_trumps_disk() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;

        for (idx, env_var) in TOKEN_ENV_VARS.into_iter().enumerate() {
            let env_var_value = format!("bar{}", idx);

            let env = {
                let mut map = HashMap::new();
                map.insert(env_var.into(), env_var_value.clone());
                map
            };
            let config =
                UserConfigLoader::new(AbsoluteSystemPathBuf::try_from(config_file.path()).unwrap())
                    .with_environment(Some(env))
                    .load()?;

            assert_eq!(config.token(), Some(env_var_value.as_str()));
        }

        Ok(())
    }
}
