use serde::Serialize;
use tracing::trace;
use turborepo_repository::{package_json::PackageJson, package_manager::PackageManager};

use crate::{
    cli::Args, commands::CommandBase, config::ConfigurationOptions, run, run::Run,
    task_hash::TaskHashTrackerState,
};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub config: &'a ConfigurationOptions,
    global_hash: Option<String>,
    task_hash_tracker: Option<TaskHashTrackerState>,
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
    type Error = run::Error;
    fn try_from(base: &'a CommandBase) -> Result<Self, Self::Error> {
        let run = Run::new(base);

        let global_hash;
        let task_hash_tracker;
        #[cfg(debug_assertions)]
        {
            let result = run.get_hashes()?;
            global_hash = Some(result.0);
            task_hash_tracker = Some(result.1);
        }
        #[cfg(not(debug_assertions))]
        {
            global_hash = None;
            task_hash_tracker = None;
        }

        let root_package_json =
            PackageJson::load(&base.repo_root.join_component("package.json")).ok();

        let package_manager =
            PackageManager::get_package_manager(&base.repo_root, root_package_json.as_ref())?;
        trace!("Found {} as package manager", package_manager);

        let config = base.config()?;

        let api_client_config = APIClientConfig {
            token: config.token(),
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.timeout(),
        };

        let spaces_api_client_config = SpacesAPIClientConfig {
            token: config.token(),
            team_id: config.team_id(),
            team_slug: config.team_slug(),
            api_url: config.api_url(),
            use_preflight: config.preflight(),
            timeout: config.timeout(),
        };

        Ok(ExecutionState {
            config,
            global_hash,
            task_hash_tracker,
            api_client_config,
            spaces_api_client_config,
            package_manager,
            cli_args: base.args(),
        })
    }
}
