use std::time::Duration;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_auth::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};
use turborepo_dirs::config_dir;
use turborepo_ui::ColorConfig;

use crate::{
    cli,
    config::{
        resolve_configuration_from_args, resolve_turbo_config_path, ConfigurationOptions,
        Error as ConfigError,
    },
    opts::Opts,
    Args,
};

pub(crate) mod bin;
pub(crate) mod boundaries;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod devtools;
pub(crate) mod docs;
pub(crate) mod generate;
pub(crate) mod get_mfe_port;
pub(crate) mod info;
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
    pub opts: Opts,
    version: &'static str,
}

impl CommandBase {
    pub fn new(
        args: Args,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        color_config: ColorConfig,
    ) -> Result<Self, cli::Error> {
        let config = Self::load_config(&repo_root, &args)?;
        let opts = Opts::new(&repo_root, &args, config)?;

        Ok(Self {
            repo_root,
            color_config,
            opts,
            version,
        })
    }

    pub fn from_opts(
        opts: Opts,
        repo_root: AbsoluteSystemPathBuf,
        version: &'static str,
        color_config: ColorConfig,
    ) -> Self {
        Self {
            repo_root,
            color_config,
            version,
            opts,
        }
    }

    pub fn load_config(
        repo_root: &AbsoluteSystemPath,
        args: &Args,
    ) -> Result<ConfigurationOptions, ConfigError> {
        resolve_configuration_from_args(repo_root, args)
    }

    pub fn opts(&self) -> &Opts {
        &self.opts
    }

    // Getting all of the paths.
    fn global_config_path(&self) -> Result<AbsoluteSystemPathBuf, ConfigError> {
        let config_dir = config_dir()?.ok_or(ConfigError::NoGlobalConfigPath)?;

        Ok(config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]))
    }
    fn local_config_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_components(&[".turbo", "config.json"])
    }
    fn root_package_json_path(&self) -> AbsoluteSystemPathBuf {
        self.repo_root.join_component("package.json")
    }
    fn root_turbo_json_path(&self) -> Result<AbsoluteSystemPathBuf, ConfigError> {
        resolve_turbo_config_path(&self.repo_root)
    }

    pub fn api_auth(&self) -> Result<Option<APIAuth>, ConfigError> {
        let team_id = self.opts.api_client_opts.team_id.as_ref();
        let team_slug = self.opts.api_client_opts.team_slug.as_ref();

        let Some(token) = &self.opts.api_client_opts.token else {
            return Ok(None);
        };

        Ok(Some(APIAuth {
            team_id: team_id.map(|s| s.to_string()),
            token: token.clone(),
            team_slug: team_slug.map(|s| s.to_string()),
        }))
    }

    pub fn api_client(&self) -> Result<APIClient, ConfigError> {
        let timeout = self.opts.api_client_opts.timeout;
        let upload_timeout = self.opts.api_client_opts.upload_timeout;

        APIClient::new(
            &self.opts.api_client_opts.api_url,
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
            self.opts.api_client_opts.preflight,
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
