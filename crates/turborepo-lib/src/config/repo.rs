use std::{collections::HashMap, env};

use anyhow::Result;
use config::Config;
use serde::{Deserialize, Serialize};
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use super::{write_to_disk, MappedEnvironment};

const DEFAULT_API_URL: &str = "https://vercel.com/api";
const DEFAULT_LOGIN_URL: &str = "https://vercel.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoConfig {
    disk_config: RepoConfigValue,
    config: RepoConfigValue,
    path: AbsoluteSystemPathBuf,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Default)]
struct RepoConfigValue {
    #[serde(rename = "apiurl")]
    api_url: Option<String>,
    #[serde(rename = "loginurl")]
    login_url: Option<String>,
    #[serde(rename = "teamslug")]
    team_slug: Option<String>,
    #[serde(rename = "teamid")]
    team_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoConfigLoader {
    path: AbsoluteSystemPathBuf,
    api: Option<String>,
    login: Option<String>,
    team_slug: Option<String>,
    environment: Option<HashMap<String, String>>,
}

impl RepoConfig {
    #[allow(dead_code)]
    pub fn api_url(&self) -> &str {
        self.config.api_url.as_deref().unwrap_or(DEFAULT_API_URL)
    }

    #[allow(dead_code)]
    pub fn login_url(&self) -> &str {
        self.config
            .login_url
            .as_deref()
            .unwrap_or(DEFAULT_LOGIN_URL)
    }

    #[allow(dead_code)]
    pub fn team_slug(&self) -> Option<&str> {
        self.config.team_slug.as_deref()
    }

    #[allow(dead_code)]
    pub fn team_id(&self) -> Option<&str> {
        self.config.team_id.as_deref()
    }

    /// Sets the team id and clears the team slug, since it may have been from
    /// an old team
    #[allow(dead_code)]
    pub fn set_team_id(&mut self, team_id: Option<String>) -> Result<()> {
        self.disk_config.team_slug = None;
        self.config.team_slug = None;
        self.disk_config.team_id = team_id.clone();
        self.config.team_id = team_id;
        self.write_to_disk()
    }

    fn write_to_disk(&self) -> Result<()> {
        write_to_disk(&self.path.as_path(), &self.disk_config)
    }
}

pub fn get_repo_config_path(repo_root: &AbsoluteSystemPathBuf) -> AbsoluteSystemPathBuf {
    let config = RelativeSystemPathBuf::new(".turbo/config.json").expect("is relative");
    repo_root.join_relative(config)
}

impl RepoConfigLoader {
    #[allow(dead_code)]
    pub fn new(path: AbsoluteSystemPathBuf) -> Self {
        Self {
            path,
            api: None,
            login: None,
            team_slug: None,
            environment: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_api(mut self, api: Option<String>) -> Self {
        self.api = api;
        self
    }

    #[allow(dead_code)]
    pub fn with_login(mut self, login: Option<String>) -> Self {
        self.login = login;
        self
    }

    #[allow(dead_code)]
    pub fn with_team_slug(mut self, team_slug: Option<String>) -> Self {
        self.team_slug = team_slug;
        self
    }

    #[allow(dead_code)]
    pub fn with_environment(mut self, environment: Option<HashMap<String, String>>) -> Self {
        self.environment = environment;
        self
    }

    #[allow(dead_code)]
    pub fn load(self) -> Result<RepoConfig> {
        let Self {
            path,
            api,
            login,
            team_slug,
            environment,
        } = self;
        let raw_disk_config = Config::builder()
            .add_source(
                config::File::with_name(path.to_string_lossy().as_ref())
                    .format(config::FileFormat::Json)
                    .required(false),
            )
            .build()?;

        let has_team_slug_override = team_slug.is_some();

        let mut config: RepoConfigValue = Config::builder()
            .add_source(raw_disk_config.clone())
            .add_source(
                MappedEnvironment::with_prefix("turbo")
                    .source(environment)
                    .replace("api", "apiurl")
                    .replace("login", "loginurl")
                    .replace("team", "teamslug"),
            )
            .set_override_option("apiurl", api)?
            .set_override_option("loginurl", login)?
            .set_override_option("teamslug", team_slug)?
            // set teamid to none if teamslug present
            .build()?
            .try_deserialize()?;

        let disk_config: RepoConfigValue = raw_disk_config.try_deserialize()?;

        // If teamid was passed via command line flag we ignore team slug as it
        // might not match.
        if has_team_slug_override {
            config.team_id = None;
        }

        // We don't set this above because it's specific to team_id
        if let Ok(vercel_artifacts_owner) = env::var("VERCEL_ARTIFACTS_OWNER") {
            config.team_id = Some(vercel_artifacts_owner);
        }

        Ok(RepoConfig {
            disk_config,
            config,
            path,
        })
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn test_repo_config_when_missing() -> Result<()> {
        let path = if cfg!(windows) {
            "C:\\missing"
        } else {
            "/missing"
        };

        let config = RepoConfigLoader::new(AbsoluteSystemPathBuf::new(path).unwrap()).load();
        assert!(config.is_ok());

        Ok(())
    }

    #[test]
    fn test_repo_config_with_team_and_api_flags() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        let config_path = AbsoluteSystemPathBuf::new(config_file.path())?;
        writeln!(&mut config_file, "{{\"teamId\": \"123\"}}")?;

        let config = RepoConfigLoader::new(config_path)
            .with_team_slug(Some("my-team-slug".into()))
            .with_api(Some("http://my-login-url".into()))
            .load()?;

        assert_eq!(config.team_id(), None);
        assert_eq!(config.team_slug(), Some("my-team-slug"));
        assert_eq!(config.api_url(), "http://my-login-url");

        Ok(())
    }

    #[test]
    fn test_repo_config_includes_defaults() {
        let path = if cfg!(windows) {
            "C:\\missing"
        } else {
            "/missing"
        };

        let config = RepoConfigLoader::new(AbsoluteSystemPathBuf::new(path).unwrap())
            .load()
            .unwrap();
        assert_eq!(config.api_url(), DEFAULT_API_URL);
        assert_eq!(config.login_url(), DEFAULT_LOGIN_URL);
        assert_eq!(config.team_slug(), None);
        assert_eq!(config.team_id(), None);
    }

    #[test]
    fn test_team_override_clears_id() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        let config_path = AbsoluteSystemPathBuf::new(config_file.path())?;
        writeln!(&mut config_file, "{{\"teamId\": \"123\"}}")?;
        let loader = RepoConfigLoader::new(config_path).with_team_slug(Some("foo".into()));

        let config = loader.load()?;
        assert_eq!(config.team_slug(), Some("foo"));
        assert_eq!(config.team_id(), None);

        Ok(())
    }

    #[test]
    fn test_set_team_clears_id() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        let config_path = AbsoluteSystemPathBuf::new(config_file.path())?;
        // We will never pragmatically write the "teamslug" field as camelCase,
        // but viper is case insensitive and we want to keep this functionality.
        writeln!(&mut config_file, "{{\"teamSlug\": \"my-team\"}}")?;
        let loader = RepoConfigLoader::new(config_path);

        let mut config = loader.clone().load()?;
        config.set_team_id(Some("my-team-id".into()))?;

        let new_config = loader.load()?;
        assert_eq!(new_config.team_slug(), None);
        assert_eq!(new_config.team_id(), Some("my-team-id"));

        Ok(())
    }

    #[test]
    fn test_repo_env_variable() -> Result<()> {
        let mut config_file = NamedTempFile::new()?;
        let config_path = AbsoluteSystemPathBuf::new(config_file.path())?;
        writeln!(&mut config_file, "{{\"teamslug\": \"other-team\"}}")?;
        let login_url = "http://my-login-url";
        let api_url = "http://my-api";
        let team_id = "123";
        let team_slug = "my-team";
        let config = RepoConfigLoader::new(config_path)
            .with_environment({
                let mut env = HashMap::new();
                env.insert("TURBO_API".into(), api_url.into());
                env.insert("TURBO_LOGIN".into(), login_url.into());
                env.insert("TURBO_TEAM".into(), team_slug.into());
                env.insert("TURBO_TEAMID".into(), team_id.into());
                Some(env)
            })
            .load()?;

        assert_eq!(config.login_url(), login_url);
        assert_eq!(config.api_url(), api_url);
        assert_eq!(config.team_id(), Some(team_id));
        assert_eq!(config.team_slug(), Some(team_slug));
        Ok(())
    }
}
