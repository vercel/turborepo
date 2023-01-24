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
    pub repo_config: RepoConfig,
    pub user_config: UserConfig,
}

impl CommandBase {
    pub fn new(args: Args, repo_root: PathBuf) -> Result<Self> {
        let repo_config_path = get_repo_config_path(&repo_root);

        Ok(Self {
            repo_root,
            ui: args.ui(),
            repo_config: RepoConfigLoader::new(repo_config_path)
                .with_api(args.api)
                .with_login(args.login)
                .with_team_slug(args.team)
                .load()?,
            user_config: UserConfigLoader::new(default_user_config_path()?)
                .with_token(args.token)
                .load()?,
        })
    }
}
