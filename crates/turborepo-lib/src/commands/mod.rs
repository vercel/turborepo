use std::{borrow::Borrow, cell::OnceCell};

use anyhow::Result;
use sha2::{Digest, Sha256};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::APIClient;
use turborepo_ui::UI;

use crate::{
    config::{
        default_user_config_path, get_repo_config_path, ClientConfig, ClientConfigLoader,
        Error as ConfigError, RepoConfig, RepoConfigLoader, UserConfig, UserConfigLoader,
    },
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
pub(crate) mod unlink;

#[derive(Debug)]
pub struct CommandBase {
    pub repo_root: AbsoluteSystemPathBuf,
    pub ui: UI,
    user_config: OnceCell<UserConfig>,
    repo_config: OnceCell<RepoConfig>,
    client_config: OnceCell<ClientConfig>,
    args: Args,
    version: &'static str,
}

impl CommandBase {
    pub fn new(
        args: Args,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        ui: UI,
    ) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui,
            args,
            repo_config: OnceCell::new(),
            user_config: OnceCell::new(),
            client_config: OnceCell::new(),
            version,
        })
    }

    fn repo_config_init(&self) -> Result<RepoConfig, ConfigError> {
        let repo_config_path = get_repo_config_path(self.repo_root.borrow());

        RepoConfigLoader::new(repo_config_path)
            .with_api(self.args.api.clone())
            .with_login(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .load()
    }

    pub fn repo_config(&self) -> Result<&RepoConfig, ConfigError> {
        self.repo_config.get_or_try_init(|| self.repo_config_init())
    }

    pub fn repo_config_mut(&mut self) -> Result<&mut RepoConfig, ConfigError> {
        // Approximates `get_mut_or_try_init`
        self.repo_config()?;
        Ok(self.repo_config.get_mut().unwrap())
    }

    fn user_config_init(&self) -> Result<UserConfig, ConfigError> {
        UserConfigLoader::new(default_user_config_path()?)
            .with_token(self.args.token.clone())
            .load()
    }

    pub fn user_config(&self) -> Result<&UserConfig, ConfigError> {
        self.user_config.get_or_try_init(|| self.user_config_init())
    }

    pub fn user_config_mut(&mut self) -> Result<&mut UserConfig, ConfigError> {
        // Approximates `get_mut_or_try_init`
        self.user_config()?;
        Ok(self.user_config.get_mut().unwrap())
    }

    fn client_config_init(&self) -> Result<ClientConfig, ConfigError> {
        ClientConfigLoader::new()
            .with_remote_cache_timeout(self.args.remote_cache_timeout)
            .load()
    }

    pub fn client_config(&self) -> Result<&ClientConfig, ConfigError> {
        self.client_config
            .get_or_try_init(|| self.client_config_init())
    }

    pub fn client_config_mut(&mut self) -> Result<&mut ClientConfig, ConfigError> {
        // Approximates `get_mut_or_try_init`
        self.client_config()?;
        Ok(self.client_config.get_mut().unwrap())
    }

    pub fn args(&self) -> &Args {
        &self.args
    }

    pub fn api_client(&self) -> Result<APIClient> {
        let repo_config = self.repo_config()?;
        let client_config = self.client_config()?;
        let args = self.args();

        let api_url = repo_config.api_url();
        let timeout = client_config.remote_cache_timeout();
        Ok(APIClient::new(
            api_url,
            timeout,
            self.version,
            args.preflight,
        )?)
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
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true)).unwrap();

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
        let command_base = CommandBase::new(args, repo_root, get_version(), UI::new(true)).unwrap();

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }
}
