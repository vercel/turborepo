use serde::Serialize;
use tracing::trace;
use turborepo_auth::Provider;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use crate::{cli::Args, commands::CommandBase, config::ConfigurationOptions, run};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub config: &'a ConfigurationOptions,
    pub api_client_config: APIClientConfig<'a>,
    pub spaces_api_client_config: SpacesAPIClientConfig<'a>,
    package_manager: PackageManager,
    pub cli_args: &'a Args,
}

#[derive(Debug, Serialize, Default)]
pub struct APIClientConfig<'a> {
    // Comes from user config, i.e. $XDG_CONFIG_HOME/turborepo/config.json
    pub token: Option<String>,
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
    pub token: Option<String>,
    // Comes from repo config, i.e. ./.turbo/config.json
    pub team_id: Option<&'a str>,
    pub team_slug: Option<&'a str>,
    pub api_url: &'a str,
    pub use_preflight: bool,
    pub timeout: u64,
}

impl<'a> TryFrom<&'a CommandBase> for ExecutionState<'a> {
    type Error = run::Error;
    fn try_from(base: &'a CommandBase) -> Result<Self, Self::Error> {
        let root_package_json =
            PackageJson::load(&base.repo_root.join_component("package.json")).ok();

        let package_manager =
            PackageManager::get_package_manager(&base.repo_root, root_package_json.as_ref())?;
        trace!("Found {} as package manager", package_manager);

        let config = base.config()?;
        let token: Option<String> = if let Some(token) = config.token() {
            Some(token.to_string())
        } else {
            let auth_file_path = base.global_auth_path()?;
            let config_file_path = base.global_config_path()?;
            let auth_provider = Provider::new(turborepo_auth::Source::Turborepo(
                auth_file_path,
                config_file_path,
                config.api_url().to_string(),
            ))?;

            auth_provider
                .get_token(config.api_url())
                .map(|t| t.token.to_owned())
        };

        let api_client_config = APIClientConfig {
            token: token.clone(),
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.timeout(),
        };

        let spaces_api_client_config = SpacesAPIClientConfig {
            token,
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.timeout(),
        };

        Ok(ExecutionState {
            config,
            api_client_config,
            spaces_api_client_config,
            package_manager,
            cli_args: base.args(),
        })
    }
}
