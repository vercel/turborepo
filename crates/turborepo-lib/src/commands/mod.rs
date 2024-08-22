use std::{cell::OnceCell, time::Duration};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_auth::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};
use turborepo_dirs::config_dir;
use turborepo_ui::ColorConfig;

use crate::{
    cli::Command,
    config::{ConfigurationOptions, Error as ConfigError, TurborepoConfigBuilder},
    turbo_json::UIMode,
    Args,
};

pub(crate) mod bin;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod generate;
pub(crate) mod link;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod ls;
pub(crate) mod prune;
pub(crate) mod query;
pub(crate) mod run;
pub(crate) mod scan;
pub(crate) mod telemetry;
pub(crate) mod unlink;

#[derive(Debug, Clone)]
pub struct CommandBase {
    pub repo_root: AbsoluteSystemPathBuf,
    pub color_config: ColorConfig,
    #[cfg(test)]
    pub global_config_path: Option<AbsoluteSystemPathBuf>,
    config: OnceCell<ConfigurationOptions>,
    args: Args,
    version: &'static str,
}

impl CommandBase {
    pub fn new(
        args: Args,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        color_config: ColorConfig,
    ) -> Self {
        Self {
            repo_root,
            color_config,
            args,
            #[cfg(test)]
            global_config_path: None,
            config: OnceCell::new(),
            version,
        }
    }

    #[cfg(test)]
    pub fn with_global_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_config_path = Some(path);
        self
    }

    fn config_init(&self) -> Result<ConfigurationOptions, ConfigError> {
        TurborepoConfigBuilder::new(self)
            // The below should be deprecated and removed.
            .with_api_url(self.args.api.clone())
            .with_login_url(self.args.login.clone())
            .with_team_slug(self.args.team.clone())
            .with_token(self.args.token.clone())
            .with_timeout(self.args.remote_cache_timeout)
            .with_preflight(self.args.preflight.then_some(true))
            .with_ui(self.args.ui.or_else(|| {
                self.args.execution_args.as_ref().and_then(|args| {
                    if !args.log_order.compatible_with_tui() {
                        Some(UIMode::Stream)
                    } else {
                        // If the argument is compatible with the TUI this does not mean we should
                        // override other configs
                        None
                    }
                })
            }))
            .with_allow_no_package_manager(
                self.args
                    .dangerously_disable_package_manager_check
                    .then_some(true),
            )
            .with_daemon(self.args.run_args.as_ref().and_then(|args| args.daemon()))
            .with_env_mode(
                self.args
                    .command
                    .as_ref()
                    .and_then(|c| match c {
                        Command::Run { execution_args, .. } => execution_args.env_mode,
                        _ => None,
                    })
                    .or_else(|| {
                        self.args
                            .execution_args
                            .as_ref()
                            .and_then(|args| args.env_mode)
                    }),
            )
            .with_cache_dir(
                self.args
                    .command
                    .as_ref()
                    .and_then(|c| match c {
                        Command::Run { execution_args, .. } => execution_args.cache_dir.clone(),
                        _ => None,
                    })
                    .or_else(|| {
                        self.args
                            .execution_args
                            .as_ref()
                            .and_then(|args| args.cache_dir.clone())
                    }),
            )
            .build()
    }

    pub fn config(&self) -> Result<&ConfigurationOptions, ConfigError> {
        self.config.get_or_try_init(|| self.config_init())
    }

    // Getting all of the paths.
    fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, ConfigError> {
        #[cfg(test)]
        if let Some(global_config_path) = self.global_config_path.clone() {
            return Ok(global_config_path);
        }

        let config_dir = config_dir()?.ok_or(ConfigError::NoGlobalConfigPath)?;

        Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    fn root_turbo_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("turbo.json")
    }

    pub fn api_auth(&self) -> Result<Option<APIAuth>, ConfigError> {
        let config = self.config()?;
        let team_id = config.team_id();
        let team_slug = config.team_slug();

        let Some(token) = config.token() else {
            return Ok(None);
        };

        Ok(Some(APIAuth {
            team_id: team_id.map(|s| s.to_string()),
            token: token.to_string(),
            team_slug: team_slug.map(|s| s.to_string()),
        }))
    }

    pub fn args(&self) -> &Args {
        &self.args
    }

    pub fn args_mut(&mut self) -> &mut Args {
        &mut self.args
    }

    pub fn api_client(&self) -> Result<APIClient, ConfigError> {
        let config = self.config()?;
        let api_url = config.api_url();
        let timeout = config.timeout();
        let upload_timeout = config.upload_timeout();

        APIClient::new(
            api_url,
            if timeout > 0 {
                Some(Duration::from_secs(timeout))
            } else {
                None
            },
            if upload_timeout > 0 {
                Some(Duration::from_secs(upload_timeout))
            } else {
                None
            },
            self.version,
            config.preflight(),
        )
        .map_err(ConfigError::ApiClient)
    }

    /// Current working directory for the turbo command
    pub fn cwd(&self) -> &AbsoluteSystemPath {
        // Earlier in execution
        // self.cli_args.cwd = Some(repo_root.as_path())
        // happens.
        // We directly use repo_root to avoid converting back to absolute system path
        &self.repo_root
    }

    pub fn version(&self) -> &'static str {
        self.version
    }
}
