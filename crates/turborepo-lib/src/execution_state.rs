use serde::Serialize;

use crate::{cli::Args, commands::CommandBase};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub api_client_config: APIClientConfig<'a>,
    pub cli_args: &'a Args,
}

#[derive(Debug, Serialize, Default)]
pub struct APIClientConfig<'a> {
    // Comes from user config, i.e. $XDG_CONFIG_HOME/turborepo/config.json
    pub token: Option<&'a str>,
    // Comes from repo config, i.e. ./.turbo/config.json
    pub team_id: Option<&'a str>,
    pub team_slug: Option<&'a str>,
    pub api_url: &'a str,
    pub use_preflight: bool,
    pub timeout: u64,
}

impl<'a> TryFrom<&'a CommandBase> for ExecutionState<'a> {
    type Error = anyhow::Error;

    fn try_from(base: &'a CommandBase) -> Result<Self, Self::Error> {
        let repo_config = base.repo_config()?;
        let user_config = base.user_config()?;
        let client_config = base.client_config()?;
        let args = base.args();

        let api_client_config = APIClientConfig {
            token: user_config.token(),
            team_id: repo_config.team_id(),
            team_slug: repo_config.team_slug(),
            api_url: repo_config.api_url(),
            use_preflight: args.preflight,
            timeout: client_config.remote_cache_timeout(),
        };

        Ok(ExecutionState {
            api_client_config,
            cli_args: base.args(),
        })
    }
}
