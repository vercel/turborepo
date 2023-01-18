use std::{
    env,
    env::current_dir,
    path::{Path, PathBuf},
};

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

use crate::Args;

pub fn default_user_config_path() -> Result<PathBuf> {
    config_dir()
        .map(|p| p.join("turborepo").join("config.json"))
        .context("default config path not found")
}

#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|p| p.join("turborepo"))
}

const DEFAULT_API_URL: &str = "https://vercel.com/api";
const DEFAULT_LOGIN_URL: &str = "https://vercel.com";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepoConfig {
    #[serde(rename = "apiurl")]
    pub api_url: String,
    #[serde(rename = "loginurl")]
    pub login_url: String,
    #[serde(rename = "teamslug")]
    pub team_slug: Option<String>,
}

impl RepoConfig {
    pub fn new(clap_args: &Args) -> Result<Self> {
        let repo_root = match &clap_args.cwd {
            Some(cwd) => cwd.clone(),
            None => current_dir()?,
        };
        let config_path = repo_root.join(".turbo").join("config.json");
        let config: RepoConfig = Config::builder()
            .set_override_option("teamslug", env::var("TURBO_TEAM").ok())?
            .set_override_option("apiurl", env::var("TURBO_API").ok())?
            .set_override_option("loginurl", env::var("TURBO_LOGIN").ok())?
            .set_override_option("apiurl", clap_args.api.as_deref())?
            .set_override_option("loginurl", clap_args.login.as_deref())?
            .set_override_option("teamslug", clap_args.team.as_deref())?
            .set_default("apiurl", DEFAULT_API_URL)?
            .set_default("loginurl", DEFAULT_LOGIN_URL)?
            .add_source(
                config::File::with_name(config_path.to_string_lossy().as_ref())
                    .format(config::FileFormat::Json)
                    .required(false),
            )
            .build()?
            .try_deserialize()?;

        Ok(config)
    }
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
    use std::{env, fs, io::Write};

    use tempfile::{NamedTempFile, TempDir};

    use super::*;

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

    fn test_env_var_trumps_disk() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        writeln!(&mut config_file, "{{\"token\": \"foo\"}}")?;
        env::set_var("TURBO_TOKEN", "bar");
        let config = UserConfig::load(config_file.path(), None)?;
        assert_eq!(config.token(), Some("bar"));
        Ok(())
    }

    fn test_create_repo_config_no_overrides() {
        let repo_root = TempDir::new().unwrap();

        // Confirm that defaults are used when no values are provided.
        let config = RepoConfig::new(repo_root.path(), None, None, None).unwrap();
        assert_eq!(config.api_url, DEFAULT_API_URL);
        assert_eq!(config.login_url, DEFAULT_LOGIN_URL);
        assert_eq!(config.team_slug, None);
    }
    fn test_create_repo_config_with_overrides() {
        let repo_root = TempDir::new().unwrap();
        // Confirm that when values are provided, they should be used.
        let config = RepoConfig::new(
            repo_root.path(),
            Some("https://api.example.com".to_string()),
            Some("https://login.example.com".to_string()),
            Some("team".to_string()),
        )
        .unwrap();
        assert_eq!(config.api_url, "https://api.example.com");
        assert_eq!(config.login_url, "https://login.example.com");
        assert_eq!(config.team_slug, Some("team".to_string()));
    }

    fn test_create_repo_config_with_config_file() {
        let repo_root = TempDir::new().unwrap();
        // Confirm that the repo config file is used when present.
        let turbo_dir_path = repo_root.path().join(".turbo");
        fs::create_dir_all(&turbo_dir_path).unwrap();
        let config_file_path = turbo_dir_path.join("config.json");
        fs::write(
            config_file_path,
            r#"{
              "apiurl": "https://api.example4.com",
              "loginurl": "https://login.example4.com",
              "teamslug": "turbo-team"
             }"#,
        )
        .unwrap();

        let config = RepoConfig::new(repo_root.path(), None, None, None).unwrap();
        assert_eq!(config.api_url, "https://api.example4.com");
        assert_eq!(config.login_url, "https://login.example4.com");
        assert_eq!(config.team_slug, Some("turbo-team".to_string()));
    }

    fn test_create_repo_config_with_env_var() {
        // Confirm that environment variables are used when no values are provided.
        env::set_var("TURBO_API", "https://api.example2.com");
        env::set_var("TURBO_LOGIN", "https://login.example2.com");
        env::set_var("TURBO_TEAM", "turborepo");

        let config = RepoConfig::new(repo_root.path(), None, None, None).unwrap();
        assert_eq!(config.api_url, "https://api.example2.com");
        assert_eq!(config.login_url, "https://login.example2.com");
        assert_eq!(config.team_slug, Some("turborepo".to_string()));

        // Confirm that manual overrides take precedence over env variables.
        let config = RepoConfig::new(
            repo_root.path(),
            Some("https://api.example3.com".to_string()),
            Some("https://login.example3.com".to_string()),
            Some("turbo-tooling".to_string()),
        )
        .unwrap();
        assert_eq!(config.api_url, "https://api.example3.com");
        assert_eq!(config.login_url, "https://login.example3.com");
        assert_eq!(config.team_slug, Some("turbo-tooling".to_string()));

        env::remove_var("TURBO_API");
        env::remove_var("TURBO_LOGIN");
        env::remove_var("TURBO_TEAM");
    }

    // NOTE: This is one large test because tests are run in parallel and we do not
    // want interleaved state with environment variables.
    #[test]
    fn test_config() {
        // Remove variables to avoid accidental test failures;
        env::remove_var("TURBO_TEAM");
        env::remove_var("TURBO_API");
        env::remove_var("TURBO_LOGIN");

        test_handles_non_existent_path().unwrap();
        test_disk_value_preserved().unwrap();
        test_env_var_trumps_disk().unwrap();
        test_create_repo_config_no_overrides();
        test_create_repo_config_with_overrides();
        test_create_repo_config_with_config_file();
        test_create_repo_config_with_env_var();
    }
}
