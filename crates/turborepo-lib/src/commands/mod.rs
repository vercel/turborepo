use std::cell::OnceCell;

use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_auth::{
    TURBOREPO_AUTH_FILE_NAME, TURBOREPO_CONFIG_DIR, TURBOREPO_LEGACY_AUTH_FILE_NAME,
};
use turborepo_dirs::config_dir;
use turborepo_ui::UI;

use crate::{
    config::{ConfigurationOptions, Error as ConfigError, TurborepoConfigBuilder},
    Args,
};

pub(crate) mod bin;
pub(crate) mod daemon;
pub(crate) mod generate;
pub(crate) mod info;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod prune;
pub(crate) mod run;
pub(crate) mod telemetry;
pub(crate) mod unlink;

#[derive(Debug)]
pub struct CommandBase {
    pub repo_root: AbsoluteSystemPathBuf,
    pub ui: UI,
    #[cfg(test)]
    pub global_config_path: Option<AbsoluteSystemPathBuf>,
    #[cfg(test)]
    pub global_auth_path: Option<AbsoluteSystemPathBuf>,
    config: OnceCell<ConfigurationOptions>,
    args: Args,
    version: &'static str,
}

impl CommandBase {
    pub fn new(
        args: Args,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        ui: UI,
    ) -> Self {
        Self {
            repo_root,
            ui,
            args,
            #[cfg(test)]
            global_config_path: None,
            #[cfg(test)]
            global_auth_path: None,
            config: OnceCell::new(),
            version,
        }
    }

    #[cfg(test)]
    pub fn with_global_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_config_path = Some(path);
        self
    }
    #[cfg(test)]
    pub fn with_global_auth_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_auth_path = Some(path);
        self
    }

    fn config_init(&self) -> Result<ConfigurationOptions, ConfigError> {
        TurborepoConfigBuilder::new(self)
            .with_api_url(self.args.api.clone())
            .with_login_url(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .with_token(self.args.token.clone())
            .with_timeout(self.args.remote_cache_timeout)
            .build()
    }

    pub fn config(&self) -> Result<&ConfigurationOptions, ConfigError> {
        self.config.get_or_try_init(|| self.config_init())
    }

    // Getting all of the paths.
    pub fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, ConfigError> {
        #[cfg(test)]
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }

        let config_dir = config_dir().ok_or(ConfigError::NoGlobalConfigPath)?;
        let global_config_path = config_dir
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_LEGACY_AUTH_FILE_NAME);
        AbsoluteSystemPathBuf::try_from(global_config_path).map_err(ConfigError::PathError)
    }
    /// Returns the path to the global auth file (auth.json).
    pub fn global_auth_path(&self) -> Result<AbsoluteSystemPathBuf, ConfigError> {
        #[cfg(test)]
        if let Some(global_auth_path) = &self.global_auth_path {
            return Ok(global_auth_path.clone());
        }

        let config_dir = config_dir().ok_or(ConfigError::NoGlobalAuthFilePath)?;
        let global_auth_path = config_dir
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME);
        AbsoluteSystemPathBuf::try_from(global_auth_path).map_err(ConfigError::PathError)
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    fn root_turbo_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("turbo.json")
    }

    pub fn api_auth(&self) -> Result<Option<APIAuth>, ConfigError> {
        let config = self.config()?;
        let team_id = config.team_id();
        let team_slug = config.team_slug();

        // Check to see if token was passed in. If so, use that.
        if let Some(token) = self.args.token.clone() {
            return Ok(Some(APIAuth {
                team_id: team_id.map(|s| s.to_string()),
                token: token.to_string(),
                team_slug: team_slug.map(|s| s.to_string()),
            }));
        }

        let auth_file_path = self.global_auth_path()?;
        let config_file_path = self.global_config_path()?;
        let client = self.api_client()?;
        let auth = turborepo_auth::read_or_create_auth_file(
            &auth_file_path,
            &config_file_path,
            client.base_url(),
        )?;

        let auth_token = auth.get_token(client.base_url());
        if let Some(auth_token) = auth_token {
            Ok(Some(APIAuth {
                team_id: team_id.map(|s| s.to_string()),
                token: auth_token.token,
                team_slug: team_slug.map(|s| s.to_string()),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn args(&self) -> &Args {
        &self.args
    }

    pub fn api_client(&self) -> Result<APIClient, ConfigError> {
        let config = self.config()?;
        let args = self.args();

        let api_url = config.api_url();
        let timeout = config.timeout();

        APIClient::new(api_url, timeout, self.version, args.preflight)
            .map_err(ConfigError::ApiClient)
    }

    pub fn daemon_file_root(&self) -> AbsoluteSystemPathBuf {
        AbsoluteSystemPathBuf::new(std::env::temp_dir().to_str().expect("UTF-8 path"))
            .expect("temp dir is valid")
            .join_component("turbod")
            .join_component(self.repo_hash().as_str())
    }

    fn repo_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.repo_root.as_bytes());
        hex::encode(&hasher.finalize()[..8])
    }

    /// Current working directory for the turbo command
    pub fn cwd(&self) -> &AbsoluteSystemPath {
        // Earlier in execution
        // self.cli_args.cwd = Some(repo_root.as_path())
        // happens.
        // We directly use repo_root to avoid converting back to absolute system path
        &self.repo_root
    }

    pub fn version(&self) -> &'static str {
        self.version
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::UI;

    use crate::get_version;

    #[cfg(not(target_os = "windows"))]
    #[test_case("/tmp/turborepo", "6e0cfa616f75a61c"; "basic example")]
    fn test_repo_hash(path: &str, expected_hash: &str) {
        use super::CommandBase;
        use crate::Args;

        let args = Args::default();
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true));

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }

    #[cfg(target_os = "windows")]
    #[test_case("C:\\\\tmp\\turborepo", "0103736e6883e35f"; "basic example")]
    fn test_repo_hash_win(path: &str, expected_hash: &str) {
        use super::CommandBase;
        use crate::Args;

        let args = Args::default();
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true));

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }
}
