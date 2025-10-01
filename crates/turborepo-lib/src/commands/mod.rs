use std::time::Duration;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_api_client::{APIAuth, APIClient};
use turborepo_auth::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};
use turborepo_dirs::config_dir;
use turborepo_ui::ColorConfig;

use crate::{
    cli,
    config::{
        resolve_turbo_config_path, ConfigurationOptions, Error as ConfigError,
        TurborepoConfigBuilder,
    },
    opts::Opts,
    Args,
};

pub(crate) mod bin;
pub(crate) mod boundaries;
pub(crate) mod clone;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod generate;
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
        TurborepoConfigBuilder::new(repo_root)
            // The below should be deprecated and removed.
            .with_api_url(args.api.clone())
            .with_login_url(args.login.clone())
            .with_team_slug(args.team.clone())
            .with_token(args.token.clone())
            .with_timeout(args.remote_cache_timeout)
            .with_preflight(args.preflight.then_some(true))
            .with_ui(args.ui)
            .with_allow_no_package_manager(
                args.dangerously_disable_package_manager_check
                    .then_some(true),
            )
            .with_daemon(args.run_args().and_then(|args| args.daemon()))
            .with_env_mode(
                args.execution_args()
                    .and_then(|execution_args| execution_args.env_mode),
            )
            .with_cache_dir(
                args.execution_args()
                    .and_then(|execution_args| execution_args.cache_dir.clone()),
            )
            .with_root_turbo_json_path(
                args.root_turbo_json
                    .clone()
                    .map(AbsoluteSystemPathBuf::from_cwd)
                    .transpose()?,
            )
            .with_force(
                args.run_args()
                    .and_then(|args| args.force.map(|value| value.unwrap_or(true))),
            )
            .with_log_order(args.execution_args().and_then(|args| args.log_order))
            .with_remote_only(args.run_args().and_then(|args| args.remote_only()))
            .with_remote_cache_read_only(
                args.run_args()
                    .and_then(|args| args.remote_cache_read_only()),
            )
            .with_cache(
                args.run_args()
                    .and_then(|args| args.cache.as_deref())
                    .map(|cache| cache.parse())
                    .transpose()?,
            )
            .with_run_summary(args.run_args().and_then(|args| args.summarize()))
            .with_allow_no_turbo_json(args.allow_no_turbo_json.then_some(true))
            .with_concurrency(
                args.execution_args()
                    .and_then(|args| args.concurrency.clone()),
            )
            .build()
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
            token: token.to_string(),
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
