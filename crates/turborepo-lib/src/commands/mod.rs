use std::path::PathBuf;

use anyhow::Result;

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
    user_config: Option<UserConfig>,
    repo_config: Option<RepoConfig>,
    args: Args,
}

impl CommandBase {
    pub fn new(args: Args, repo_root: PathBuf) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui: args.ui(),
            args,
            repo_config: None,
            user_config: None,
        })
    }

    pub fn repo_config(&mut self) -> Result<RepoConfig> {
        if let Some(repo_config) = &self.repo_config {
            return Ok(repo_config.clone());
        } else {
            let repo_config_path = get_repo_config_path(&self.repo_root);

            let repo_config = RepoConfigLoader::new(repo_config_path)
                .with_api(self.args.api.clone())
                .with_login(self.args.login.clone())
                .with_team_slug(self.args.team.clone())
                .load()?;
            self.repo_config = Some(repo_config.clone());

            Ok(repo_config)
        }
    }

    pub fn user_config(&mut self) -> Result<UserConfig> {
        if let Some(user_config) = &self.user_config {
            return Ok(user_config.clone());
        } else {
            let user_config = UserConfigLoader::new(default_user_config_path()?)
                .with_token(self.args.token.clone())
                .load()?;
            self.user_config = Some(user_config.clone());
            Ok(user_config)
        }
    }

    pub fn api_client(&mut self) -> Result<Option<APIClient>> {
        let repo_config = self.repo_config()?;
        let user_config = self.user_config()?;
        if let Some(token) = user_config.token() {
            Ok(Some(APIClient::new(token, repo_config.api_url())?))
        } else {
            Ok(None)
        }
    }
}
