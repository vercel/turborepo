use serde::Serialize;
use tracing::trace;

use crate::{
    cli::Args, commands::CommandBase, package_json::PackageJson, package_manager::PackageManager,
};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub api_client_config: APIClientConfig<'a>,
    pub spaces_api_client_config: SpacesAPIClientConfig<'a>,
    package_manager: PackageManager,
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

#[derive(Debug, Serialize, Default)]
pub struct SpacesAPIClientConfig<'a> {
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
        let root_package_json =
            PackageJson::load(&base.repo_root.join_component("package.json")).ok();

        let package_manager =
            PackageManager::get_package_manager(&base.repo_root, root_package_json.as_ref())?;
        trace!("Found {} as package manager", package_manager);

        let config = base.turbo_config()?;

        let api_client_config = APIClientConfig {
            token: config.token(),
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.remote_cache_timeout(),
        };

        let spaces_api_client_config = SpacesAPIClientConfig {
            token: config.token(),
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.remote_cache_timeout(),
        };

        Ok(ExecutionState {
            api_client_config,
            spaces_api_client_config,
            package_manager,
            cli_args: base.args(),
        })
    }
}
