use serde::Serialize;
use tracing::trace;
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::{
    cli::Args, commands::CommandBase, package_json::PackageJson, package_manager::PackageManager,
};

#[derive(Debug, Serialize)]
pub struct ExecutionState<'a> {
    pub api_client_config: APIClientConfig<'a>,
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

impl<'a> TryFrom<&'a CommandBase> for ExecutionState<'a> {
    type Error = anyhow::Error;

    fn try_from(base: &'a CommandBase) -> Result<Self, Self::Error> {
        let root_package_json = PackageJson::load(&AbsoluteSystemPathBuf::new(
            base.repo_root
                .join_relative(RelativeSystemPathBuf::new("package.json")?),
        )?)
        .ok();

        let package_manager =
            PackageManager::get_package_manager(base, root_package_json.as_ref())?;
        trace!("Found {} as package manager", package_manager);

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
            package_manager,
            cli_args: base.args(),
        })
    }
}
