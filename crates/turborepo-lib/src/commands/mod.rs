use std::time::Duration;

use camino::Utf8PathBuf;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};
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
    turbo_json::RawRootTurboJson,
    Args,
};

pub(crate) mod bin;
pub(crate) mod boundaries;
pub(crate) mod clone;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod devtools;
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
        // First, read turbo.json config values if present
        let turbo_json_config = Self::read_turbo_json_config(repo_root, args)?;

        let builder = TurborepoConfigBuilder::new(repo_root)
            // Provide the turbo.json configuration (lowest priority, merged first)
            .with_turbo_json_config(turbo_json_config)
            // CLI arguments (highest priority, applied as overrides)
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
            );

        builder.build().map_err(ConfigError::from)
    }

    /// Reads configuration values from turbo.json file.
    /// Returns default ConfigurationOptions if turbo.json doesn't exist or
    /// can't be parsed.
    fn read_turbo_json_config(
        repo_root: &AbsoluteSystemPath,
        args: &Args,
    ) -> Result<ConfigurationOptions, ConfigError> {
        // Determine the turbo.json path
        let turbo_json_path = if let Some(ref custom_path) = args.root_turbo_json {
            AbsoluteSystemPathBuf::from_cwd(custom_path.clone())?
        } else {
            resolve_turbo_config_path(repo_root)?
        };

        // Read and parse turbo.json if it exists
        let contents = match turbo_json_path.read_existing_to_string() {
            Ok(Some(contents)) => contents,
            Ok(None) => return Ok(ConfigurationOptions::default()),
            Err(_) => return Ok(ConfigurationOptions::default()),
        };

        let root_relative_path = repo_root.anchor(&turbo_json_path).map_or_else(
            |_| turbo_json_path.as_str().to_owned(),
            |relative| relative.to_string(),
        );

        // Parse the turbo.json - if parsing fails, return empty defaults
        // The actual error will be reported later when turbo.json is
        // fully loaded for task configuration
        let raw_turbo_json = match RawRootTurboJson::parse(&contents, &root_relative_path) {
            Ok(json) => json,
            Err(_) => return Ok(ConfigurationOptions::default()),
        };

        // Extract cache_dir if present and valid
        let cache_dir = raw_turbo_json.cache_dir.as_ref().and_then(|spanned| {
            let cache_dir_str = spanned.value.as_str();
            // Validate it's a relative path
            RelativeUnixPath::new(cache_dir_str).ok()?;
            let cache_dir_system = RelativeUnixPath::new(cache_dir_str)
                .ok()?
                .to_anchored_system_path_buf();
            Some(Utf8PathBuf::from(cache_dir_system.to_string()))
        });

        // Extract remote cache options
        let remote_cache = raw_turbo_json.remote_cache.as_ref();

        // Build ConfigurationOptions from turbo.json values
        // Note: token is intentionally not read from turbo.json for security reasons
        Ok(ConfigurationOptions {
            ui: raw_turbo_json.ui.as_ref().map(|s| s.value),
            cache_dir,
            allow_no_package_manager: raw_turbo_json
                .allow_no_package_manager
                .as_ref()
                .map(|s| s.value),
            daemon: raw_turbo_json.daemon.as_ref().map(|s| s.value),
            env_mode: raw_turbo_json.env_mode.as_ref().map(|s| s.value),
            concurrency: raw_turbo_json.concurrency.as_ref().map(|s| s.value.clone()),
            api_url: remote_cache.and_then(|rc| rc.api_url.as_ref().map(|s| s.value.clone())),
            login_url: remote_cache.and_then(|rc| rc.login_url.as_ref().map(|s| s.value.clone())),
            team_slug: remote_cache.and_then(|rc| rc.team_slug.as_ref().map(|s| s.value.clone())),
            team_id: remote_cache.and_then(|rc| rc.team_id.as_ref().map(|s| s.value.clone())),
            signature: remote_cache.and_then(|rc| rc.signature.as_ref().map(|s| s.value)),
            preflight: remote_cache.and_then(|rc| rc.preflight.as_ref().map(|s| s.value)),
            timeout: remote_cache.and_then(|rc| rc.timeout.as_ref().map(|s| s.value)),
            upload_timeout: remote_cache.and_then(|rc| rc.upload_timeout.as_ref().map(|s| s.value)),
            enabled: remote_cache.and_then(|rc| rc.enabled.as_ref().map(|s| s.value)),
            ..ConfigurationOptions::default()
        })
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
        resolve_turbo_config_path(&self.repo_root).map_err(ConfigError::from)
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
