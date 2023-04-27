use anyhow::Result;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::APIClient;

use crate::{
    config::{
        default_user_config_path, get_repo_config_path, ClientConfig, ClientConfigLoader,
        RepoConfig, RepoConfigLoader, UserConfig, UserConfigLoader,
    },
    ui::UI,
    Args,
};

pub(crate) mod bin;
pub(crate) mod daemon;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod unlink;

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
    ) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui: args.ui(),
            args,
            repo_config: OnceCell::new(),
            user_config: OnceCell::new(),
            client_config: OnceCell::new(),
            version,
        })
    }

    fn create_repo_config(&self) -> Result<()> {
        let repo_config_path = get_repo_config_path(&self.repo_root);

        let repo_config = RepoConfigLoader::new(repo_config_path)
            .with_api(self.args.api.clone())
            .with_login(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .load()?;

        self.repo_config.set(repo_config)?;

        Ok(())
    }

    // NOTE: This deletes the repo config file. It does *not* remove the
    // `RepoConfig` struct from `CommandBase`. This is fine because we
    // currently do not have any commands that delete the repo config file
    // and then attempt to read from it.
    pub fn delete_repo_config_file(&mut self) -> Result<()> {
        let repo_config_path = get_repo_config_path(&self.repo_root);
        if repo_config_path.exists() {
            std::fs::remove_file(repo_config_path)?;
        }
        Ok(())
    }

    fn create_user_config(&self) -> Result<()> {
        let user_config = UserConfigLoader::new(default_user_config_path()?)
            .with_token(self.args.token.clone())
            .load()?;
        self.user_config.set(user_config)?;

        Ok(())
    }

    fn create_client_config(&self) -> Result<()> {
        let client_config = ClientConfigLoader::new()
            .with_remote_cache_timeout(self.args.remote_cache_timeout)
            .load()?;
        self.client_config.set(client_config)?;

        Ok(())
    }

    pub fn repo_config_mut(&mut self) -> Result<&mut RepoConfig> {
        if self.repo_config.get().is_none() {
            self.create_repo_config()?;
        }

        Ok(self.repo_config.get_mut().unwrap())
    }

    pub fn repo_config(&self) -> Result<&RepoConfig> {
        if self.repo_config.get().is_none() {
            self.create_repo_config()?;
        }

        Ok(self.repo_config.get().unwrap())
    }

    pub fn user_config_mut(&mut self) -> Result<&mut UserConfig> {
        if self.user_config.get().is_none() {
            self.create_user_config()?;
        }

        Ok(self.user_config.get_mut().unwrap())
    }

    pub fn user_config(&self) -> Result<&UserConfig> {
        if self.user_config.get().is_none() {
            self.create_user_config()?;
        }

        Ok(self.user_config.get().unwrap())
    }

    pub fn client_config(&self) -> Result<&ClientConfig> {
        if self.client_config.get().is_none() {
            self.create_client_config()?;
        }

        Ok(self.client_config.get().unwrap())
    }

    pub fn args(&self) -> &Args {
        &self.args
    }

    pub fn api_client(&mut self) -> Result<APIClient> {
        let repo_config = self.repo_config()?;
        let client_config = self.client_config()?;

        let api_url = repo_config.api_url();
        let timeout = client_config.remote_cache_timeout();
        APIClient::new(api_url, timeout, self.version)
    }

    pub fn daemon_file_root(&self) -> turbopath::AbsoluteSystemPathBuf {
        turbopath::AbsoluteSystemPathBuf::new(std::env::temp_dir())
            .expect("temp dir is valid")
            .join_relative(
                turbopath::RelativeSystemPathBuf::new("turbod").expect("turbod is valid"),
            )
            .join_relative(
                turbopath::RelativeSystemPathBuf::new(self.repo_hash()).expect("hash is valid"),
            )
    }

    fn repo_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.repo_root.to_str().unwrap().as_bytes());
        hex::encode(&hasher.finalize()[..8])
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::get_version;

    #[cfg(not(target_os = "windows"))]
    #[test_case("/tmp/turborepo", "6e0cfa616f75a61c"; "basic example")]
    fn test_repo_hash(path: &str, expected_hash: &str) {
        use super::CommandBase;
        use crate::Args;

        let args = Args::default();
        let repo_root = AbsoluteSystemPathBuf::new(path).unwrap();
        let command_base = CommandBase::new(args, repo_root, get_version()).unwrap();

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
        let command_base = CommandBase::new(args, repo_root, get_version()).unwrap();

        let hash = command_base.repo_hash();

        assert_eq!(hash, expected_hash);
        assert_eq!(hash.len(), 16);
    }
}
