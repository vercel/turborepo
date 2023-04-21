use serde::Serialize;

use crate::{cli::Args, commands::CommandBase};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub remote_config: RemoteConfig<'a>,
    pub cli_args: &'a Args,
}

#[derive(Debug, Serialize, Default)]
pub struct RemoteConfig<'a> {
    pub token: Option<&'a str>,
    pub team_id: Option<&'a str>,
    pub team_slug: Option<&'a str>,
    pub api_url: &'a str,
}

impl<'a> TryFrom<&'a CommandBase> for ExecutionState<'a> {
    type Error = anyhow::Error;

    fn try_from(base: &'a CommandBase) -> Result<Self, Self::Error> {
        let repo_config = base.repo_config()?;
        let user_config = base.user_config()?;

        let remote_config = RemoteConfig {
            token: user_config.token(),
            team_id: repo_config.team_id(),
            team_slug: repo_config.team_slug(),
            api_url: repo_config.api_url(),
        };

        Ok(ExecutionState {
            remote_config,
            cli_args: base.args(),
        })
    }
}
