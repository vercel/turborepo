//! Configuration options and builder for Turborepo
//!
//! This module contains the main configuration types:
//! - `ConfigurationOptions`: The resolved configuration options
//! - `TurborepoConfigBuilder`: Builder for constructing configuration from
//!   multiple sources

use std::{collections::HashMap, ffi::OsString};

use camino::{Utf8Path, Utf8PathBuf};
use derive_setters::Setters;
use merge::Merge;
use serde::Deserialize;
use struct_iterable::Iterable;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_cache::CacheConfig;

use crate::{
    Error,
    env::EnvVars,
    file::{AuthFile, ConfigFile},
    override_env::OverrideEnvVars,
};

/// Configuration file name (JSON format)
pub const CONFIG_FILE: &str = "turbo.json";

/// Configuration file name (JSON with comments format)
pub const CONFIG_FILE_JSONC: &str = "turbo.jsonc";

// =============================================================================
// Local Type Definitions
// =============================================================================

// TODO: These types are duplicated from turborepo-lib. When turbo_json is
// extracted to turborepo-config, consolidate these definitions.

/// UI mode for Turborepo output
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    Deserialize,
    clap::ValueEnum,
    biome_deserialize_macros::Deserializable,
)]
#[serde(rename_all = "camelCase")]
pub enum UIMode {
    /// Use the terminal user interface (default)
    #[default]
    Tui,
    /// Use the standard output stream
    Stream,
    /// Use the web user interface (experimental)
    Web,
}

impl std::fmt::Display for UIMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UIMode::Tui => write!(f, "tui"),
            UIMode::Stream => write!(f, "stream"),
            UIMode::Web => write!(f, "web"),
        }
    }
}

impl UIMode {
    /// Returns true if this mode uses the terminal UI
    pub fn use_tui(&self) -> bool {
        matches!(self, Self::Tui)
    }

    /// Returns true if the UI mode has a sender,
    /// i.e. web or tui but not stream
    pub fn has_sender(&self) -> bool {
        matches!(self, Self::Tui | Self::Web)
    }
}

/// Environment variable handling mode
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    Deserialize,
    clap::ValueEnum,
    biome_deserialize_macros::Deserializable,
)]
#[serde(rename_all = "lowercase")]
pub enum EnvMode {
    /// Loose mode - all env vars are passed through
    Loose,
    /// Strict mode - only declared env vars are included (default)
    #[default]
    Strict,
}

impl std::fmt::Display for EnvMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvMode::Loose => write!(f, "loose"),
            EnvMode::Strict => write!(f, "strict"),
        }
    }
}

/// Log ordering mode
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum LogOrder {
    /// Automatic ordering based on context (default)
    #[default]
    Auto,
    /// Stream logs as they arrive
    Stream,
    /// Group logs by task
    Grouped,
}

impl std::fmt::Display for LogOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogOrder::Auto => write!(f, "auto"),
            LogOrder::Stream => write!(f, "stream"),
            LogOrder::Grouped => write!(f, "grouped"),
        }
    }
}

impl LogOrder {
    /// Check if the log order is compatible with TUI mode.
    /// If the user requested a specific order to the logs, then this isn't
    /// compatible with the TUI and means we cannot use it.
    pub fn compatible_with_tui(&self) -> bool {
        matches!(self, LogOrder::Auto)
    }
}

/// Future flags for experimental features
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct FutureFlags {
    // Fields will be added when turbo_json is extracted
}

// =============================================================================
// Configuration Constants
// =============================================================================

pub(crate) const DEFAULT_API_URL: &str = "https://vercel.com/api";
pub(crate) const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
const DEFAULT_TIMEOUT: u64 = 30;
const DEFAULT_UPLOAD_TIMEOUT: u64 = 60;
pub(crate) const DEFAULT_TUI_SCROLLBACK_LENGTH: u64 = 2048;

// =============================================================================
// ConfigurationOptions
// =============================================================================

// We intentionally don't derive Serialize so that different parts
// of the code that want to display the config can tune how they
// want to display and what fields they want to include.
#[derive(Deserialize, Default, Debug, PartialEq, Eq, Clone, Iterable, Merge, Setters)]
#[serde(rename_all = "camelCase")]
// Generate setters for the builder type that set these values on its override_config field
#[setters(
    prefix = "with_",
    generate_delegates(ty = "TurborepoConfigBuilder", field = "override_config")
)]
pub struct ConfigurationOptions {
    #[serde(alias = "apiurl")]
    #[serde(alias = "ApiUrl")]
    #[serde(alias = "APIURL")]
    pub api_url: Option<String>,
    #[serde(alias = "loginurl")]
    #[serde(alias = "LoginUrl")]
    #[serde(alias = "LOGINURL")]
    pub login_url: Option<String>,
    #[serde(alias = "teamslug")]
    #[serde(alias = "TeamSlug")]
    #[serde(alias = "TEAMSLUG")]
    /// corresponds to env var TURBO_TEAM
    pub team_slug: Option<String>,
    #[serde(alias = "teamid")]
    #[serde(alias = "TeamId")]
    #[serde(alias = "TEAMID")]
    /// corresponds to env var TURBO_TEAMID
    pub team_id: Option<String>,
    /// corresponds to env var TURBO_TOKEN
    pub token: Option<String>,
    pub signature: Option<bool>,
    pub preflight: Option<bool>,
    pub timeout: Option<u64>,
    pub upload_timeout: Option<u64>,
    pub enabled: Option<bool>,
    #[serde(rename = "ui")]
    pub ui: Option<UIMode>,
    #[serde(rename = "dangerouslyDisablePackageManagerCheck")]
    pub allow_no_package_manager: Option<bool>,
    pub daemon: Option<bool>,
    #[serde(rename = "envMode")]
    pub env_mode: Option<EnvMode>,
    pub scm_base: Option<String>,
    pub scm_head: Option<String>,
    #[serde(rename = "cacheDir")]
    pub cache_dir: Option<Utf8PathBuf>,
    // This is skipped as we never want this to be stored in a file
    #[serde(skip)]
    pub root_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    pub force: Option<bool>,
    pub log_order: Option<LogOrder>,
    #[serde(skip)]
    pub cache: Option<CacheConfig>,
    pub remote_only: Option<bool>,
    pub remote_cache_read_only: Option<bool>,
    pub run_summary: Option<bool>,
    pub allow_no_turbo_json: Option<bool>,
    pub tui_scrollback_length: Option<u64>,
    pub concurrency: Option<String>,
    pub no_update_notifier: Option<bool>,
    pub sso_login_callback_port: Option<u16>,
    #[serde(skip)]
    pub future_flags: Option<FutureFlags>,
}

// Getters
impl ConfigurationOptions {
    pub fn api_url(&self) -> &str {
        non_empty_str(self.api_url.as_deref()).unwrap_or(DEFAULT_API_URL)
    }

    pub fn login_url(&self) -> &str {
        non_empty_str(self.login_url.as_deref()).unwrap_or(DEFAULT_LOGIN_URL)
    }

    pub fn team_slug(&self) -> Option<&str> {
        self.team_slug
            .as_deref()
            .and_then(|slug| (!slug.is_empty()).then_some(slug))
    }

    pub fn team_id(&self) -> Option<&str> {
        non_empty_str(self.team_id.as_deref())
    }

    pub fn token(&self) -> Option<&str> {
        non_empty_str(self.token.as_deref())
    }

    pub fn signature(&self) -> bool {
        self.signature.unwrap_or_default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn preflight(&self) -> bool {
        self.preflight.unwrap_or_default()
    }

    /// Note: 0 implies no timeout
    pub fn timeout(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_TIMEOUT)
    }

    /// Note: 0 implies no timeout
    pub fn upload_timeout(&self) -> u64 {
        self.upload_timeout.unwrap_or(DEFAULT_UPLOAD_TIMEOUT)
    }

    pub fn tui_scrollback_length(&self) -> u64 {
        self.tui_scrollback_length
            .unwrap_or(DEFAULT_TUI_SCROLLBACK_LENGTH)
    }

    pub fn ui(&self) -> UIMode {
        // If we aren't hooked up to a TTY, then do not use TUI
        if !atty::is(atty::Stream::Stdout) {
            return UIMode::Stream;
        }

        self.log_order()
            .compatible_with_tui()
            .then_some(self.ui)
            .flatten()
            .unwrap_or(UIMode::Stream)
    }

    pub fn scm_base(&self) -> Option<&str> {
        non_empty_str(self.scm_base.as_deref())
    }

    pub fn scm_head(&self) -> Option<&str> {
        non_empty_str(self.scm_head.as_deref())
    }

    pub fn allow_no_package_manager(&self) -> bool {
        self.allow_no_package_manager.unwrap_or_default()
    }

    pub fn daemon(&self) -> Option<bool> {
        // hardcode to off in CI
        if turborepo_ci::is_ci() {
            if Some(true) == self.daemon {
                debug!("Ignoring daemon setting and disabling the daemon because we're in CI");
            }

            return Some(false);
        }

        self.daemon
    }

    pub fn env_mode(&self) -> EnvMode {
        self.env_mode.unwrap_or_default()
    }

    pub fn cache_dir(&self) -> &Utf8Path {
        self.cache_dir.as_deref().unwrap_or_else(|| {
            Utf8Path::new(if cfg!(windows) {
                ".turbo\\cache"
            } else {
                ".turbo/cache"
            })
        })
    }

    pub fn cache(&self) -> Option<CacheConfig> {
        self.cache
    }

    pub fn force(&self) -> bool {
        self.force.unwrap_or_default()
    }

    pub fn log_order(&self) -> LogOrder {
        self.log_order.unwrap_or_default()
    }

    pub fn remote_only(&self) -> bool {
        self.remote_only.unwrap_or_default()
    }

    pub fn remote_cache_read_only(&self) -> bool {
        self.remote_cache_read_only.unwrap_or_default()
    }

    pub fn run_summary(&self) -> bool {
        self.run_summary.unwrap_or_default()
    }

    pub fn root_turbo_json_path(
        &self,
        repo_root: &AbsoluteSystemPath,
    ) -> Result<AbsoluteSystemPathBuf, Error> {
        if let Some(path) = &self.root_turbo_json_path {
            return Ok(path.clone());
        }

        resolve_turbo_config_path(repo_root)
    }

    pub fn allow_no_turbo_json(&self) -> bool {
        self.allow_no_turbo_json.unwrap_or_default()
    }

    pub fn no_update_notifier(&self) -> bool {
        self.no_update_notifier.unwrap_or_default()
    }

    pub fn sso_login_callback_port(&self) -> Option<u16> {
        self.sso_login_callback_port
    }

    pub fn future_flags(&self) -> FutureFlags {
        self.future_flags.clone().unwrap_or_default()
    }

    /// Returns the concurrency setting as a raw string (e.g., "10" or "50%")
    pub fn concurrency(&self) -> Option<&str> {
        self.concurrency.as_deref()
    }

    /// Returns the raw enabled flag from remote cache config
    /// Use this when you need to check if the user explicitly disabled remote
    /// cache
    pub fn enabled_raw(&self) -> Option<bool> {
        self.enabled
    }
}

// Maps Some("") to None to emulate how Go handles empty strings
fn non_empty_str(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

// =============================================================================
// ResolvedConfigurationOptions Trait
// =============================================================================

/// Trait for configuration sources that can provide configuration options.
pub(crate) trait ResolvedConfigurationOptions {
    fn get_configuration_options(
        &self,
        existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error>;
}

// Used for global config and local config.
impl<'a> ResolvedConfigurationOptions for &'a ConfigurationOptions {
    fn get_configuration_options(
        &self,
        _existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error> {
        Ok((*self).clone())
    }
}

fn get_lowercased_env_vars() -> HashMap<OsString, OsString> {
    std::env::vars_os()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

// =============================================================================
// TurborepoConfigBuilder
// =============================================================================

/// Builder for constructing Turborepo configuration.
///
/// Merges configuration from multiple sources in priority order (highest to
/// lowest):
/// 1. Builder pattern overrides (via `with_*` methods)
/// 2. Environment variables (`TURBO_*`)
/// 3. Override environment variables (`VERCEL_ARTIFACTS_*`, `CI`, `NO_COLOR`)
/// 4. Local config file (`<REPO_ROOT>/.turbo/config.json`)
/// 5. Global auth file (Vercel/Turbo token storage)
/// 6. Global config file (`~/.turbo/config.json`)
/// 7. turbo.json configuration (provided via `with_turbo_json_config`)
///
/// # Example
/// ```ignore
/// let config = TurborepoConfigBuilder::new(&repo_root)
///     .with_turbo_json_config(turbo_json_config)
///     .with_api_url(Some("https://custom.api.com".to_string()))
///     .build()?;
/// ```
#[derive(Default)]
pub struct TurborepoConfigBuilder {
    pub(crate) repo_root: AbsoluteSystemPathBuf,
    pub(crate) override_config: ConfigurationOptions,
    pub(crate) global_config_path: Option<AbsoluteSystemPathBuf>,
    /// Configuration options extracted from turbo.json
    /// This must be provided by the caller since turbo.json parsing
    /// requires types from turborepo-lib (circular dependency otherwise)
    turbo_json_config: Option<ConfigurationOptions>,
    #[allow(dead_code)]
    environment: Option<HashMap<OsString, OsString>>,
}

impl TurborepoConfigBuilder {
    pub fn new(repo_root: &AbsoluteSystemPath) -> Self {
        Self {
            repo_root: repo_root.to_owned(),
            override_config: Default::default(),
            global_config_path: None,
            turbo_json_config: None,
            environment: None,
        }
    }

    pub fn with_global_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_config_path = Some(path);
        self
    }

    /// Set the configuration options extracted from turbo.json.
    ///
    /// This is required because turbo.json parsing lives in `turborepo-lib`
    /// due to circular dependency constraints. The caller is responsible for
    /// parsing turbo.json and extracting the relevant configuration options.
    ///
    /// # Example
    /// ```ignore
    /// // In turborepo-lib:
    /// let raw_turbo_json = RawRootTurboJson::parse(&contents, &path)?;
    /// let turbo_json_config = ConfigurationOptions::from(&raw_turbo_json);
    /// let config = TurborepoConfigBuilder::new(&repo_root)
    ///     .with_turbo_json_config(turbo_json_config)
    ///     .build()?;
    /// ```
    pub fn with_turbo_json_config(mut self, config: ConfigurationOptions) -> Self {
        self.turbo_json_config = Some(config);
        self
    }

    fn get_environment(&self) -> HashMap<OsString, OsString> {
        self.environment
            .clone()
            .unwrap_or_else(get_lowercased_env_vars)
    }

    /// Build the configuration by merging all configuration sources.
    ///
    /// Priority, from least significant to most significant:
    /// - shared configuration (turbo.json) - provided via
    ///   `with_turbo_json_config`
    /// - global configuration (~/.turbo/config.json)
    /// - local configuration (<REPO_ROOT>/.turbo/config.json)
    /// - environment variables
    /// - CLI arguments
    /// - builder pattern overrides.
    pub fn build(&self) -> Result<ConfigurationOptions, Error> {
        let global_config = ConfigFile::global_config(self.global_config_path.clone())?;
        let global_auth = AuthFile::global_auth(self.global_config_path.clone())?;
        let local_config = ConfigFile::local_config(&self.repo_root);
        let env_vars = self.get_environment();
        let env_var_config = EnvVars::new(&env_vars)?;
        let override_env_var_config = OverrideEnvVars::new(&env_vars)?;

        // Use the turbo.json config if provided, otherwise use empty defaults
        let turbo_json_config = self
            .turbo_json_config
            .clone()
            .unwrap_or_else(ConfigurationOptions::default);

        // These are ordered from highest to lowest priority
        let sources: [Box<dyn ResolvedConfigurationOptions>; 7] = [
            Box::new(&self.override_config),
            Box::new(env_var_config),
            Box::new(override_env_var_config),
            Box::new(local_config),
            Box::new(global_auth),
            Box::new(global_config),
            Box::new(&turbo_json_config),
        ];

        let config = sources.into_iter().try_fold(
            ConfigurationOptions::default(),
            |mut acc, current_source| {
                let current_source_config = current_source.get_configuration_options(&acc)?;
                acc.merge(current_source_config);
                Ok(acc)
            },
        );

        // We explicitly do a let and return to help the Rust compiler see that there
        // are no references still held by the folding.
        #[allow(clippy::let_and_return)]
        config
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Given a directory path, determines which turbo.json configuration file to
/// use. Returns an error if both turbo.json and turbo.jsonc exist in the same
/// directory. Returns the path to the config file to use, defaulting to
/// turbo.json if neither exists.
pub fn resolve_turbo_config_path(
    dir_path: &AbsoluteSystemPath,
) -> Result<AbsoluteSystemPathBuf, Error> {
    let turbo_json_path = dir_path.join_component(CONFIG_FILE);
    let turbo_jsonc_path = dir_path.join_component(CONFIG_FILE_JSONC);

    let turbo_json_exists = turbo_json_path.try_exists()?;
    let turbo_jsonc_exists = turbo_jsonc_path.try_exists()?;

    match (turbo_json_exists, turbo_jsonc_exists) {
        (true, true) => Err(Error::MultipleTurboConfigs {
            directory: dir_path.to_string(),
        }),
        (true, false) => Ok(turbo_json_path),
        (false, true) => Ok(turbo_jsonc_path),
        // Default to turbo.json if neither exists
        (false, false) => Ok(turbo_json_path),
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPath;

    use super::*;

    #[test]
    fn test_defaults() {
        let defaults: ConfigurationOptions = Default::default();
        assert_eq!(defaults.api_url(), DEFAULT_API_URL);
        assert_eq!(defaults.login_url(), DEFAULT_LOGIN_URL);
        assert_eq!(defaults.team_slug(), None);
        assert_eq!(defaults.team_id(), None);
        assert_eq!(defaults.token(), None);
        assert!(!defaults.signature());
        assert!(defaults.enabled());
        assert!(!defaults.preflight());
        assert_eq!(defaults.timeout(), DEFAULT_TIMEOUT);
        assert!(!defaults.allow_no_package_manager());
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\fake\\repo"
        } else {
            "/fake/repo"
        })
        .unwrap();
        assert_eq!(
            defaults.root_turbo_json_path(repo_root).unwrap(),
            repo_root.join_component("turbo.json")
        )
    }

    #[test]
    fn test_log_order_tui_compatibility() {
        // Only Auto is compatible with TUI - if the user requested a specific
        // order to the logs, then this isn't compatible with the TUI
        assert!(LogOrder::Auto.compatible_with_tui());
        assert!(!LogOrder::Grouped.compatible_with_tui());
        assert!(!LogOrder::Stream.compatible_with_tui());
    }

    #[test]
    fn test_non_empty_str() {
        assert_eq!(non_empty_str(None), None);
        assert_eq!(non_empty_str(Some("")), None);
        assert_eq!(non_empty_str(Some("value")), Some("value"));
    }
}
