use std::path::PathBuf;

use anyhow::Result;
use tokio::sync::OnceCell;

use crate::{
    client::APIClient,
    config::{
        default_user_config_path, get_repo_config_path, RepoConfig, RepoConfigLoader, UserConfig,
        UserConfigLoader,
    },
    ui::UI,
    Args,
};

pub(crate) mod bin;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;

pub struct CommandBase {
    pub repo_root: PathBuf,
    pub ui: UI,
    user_config: OnceCell<UserConfig>,
    repo_config: OnceCell<RepoConfig>,
    args: Args,
}

impl CommandBase {
    pub fn new(args: Args, repo_root: PathBuf) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui: args.ui(),
            args,
            repo_config: OnceCell::new(),
            user_config: OnceCell::new(),
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

    fn create_user_config(&self) -> Result<()> {
        let user_config = UserConfigLoader::new(default_user_config_path()?)
            .with_token(self.args.token.clone())
            .load()?;
        self.user_config.set(user_config)?;

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

    pub fn api_client(&mut self) -> Result<Option<APIClient>> {
        let repo_config = self.repo_config()?;
        let api_url = repo_config.api_url();
        let user_config = self.user_config()?;
        if let Some(token) = user_config.token() {
            Ok(Some(APIClient::new(token, api_url)?))
        } else {
            Ok(None)
        }
    }
}
