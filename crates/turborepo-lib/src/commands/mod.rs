use std::path::PathBuf;

use anyhow::Result;

use crate::{
    config::{
        default_user_config_path, get_repo_config_path, RepoConfig, RepoConfigLoader, UserConfig,
        UserConfigLoader,
    },
    ui::UI,
    Args,
};

pub(crate) mod bin;
pub(crate) mod logout;

pub struct CommandBase {
    pub repo_root: PathBuf,
    pub ui: UI,
    args: Args,
}

impl CommandBase {
    pub fn new(args: Args, repo_root: PathBuf) -> Result<Self> {
        Ok(Self {
            repo_root,
            ui: args.ui(),
            args,
        })
    }

    #[allow(dead_code)]
    pub fn repo_config(&self) -> Result<RepoConfig> {
        let repo_config_path = get_repo_config_path(&self.repo_root);

        RepoConfigLoader::new(repo_config_path)
            .with_api(self.args.api.clone())
            .with_login(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .load()
    }

    pub fn user_config(&self) -> Result<UserConfig> {
        UserConfigLoader::new(default_user_config_path()?)
            .with_token(self.args.token.clone())
            .load()
    }
}
