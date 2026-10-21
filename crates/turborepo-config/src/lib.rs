//! turborepo-config: Configuration loading and merging for Turborepo
//!
//! This crate handles loading configuration from multiple sources:
//! - turbo.json files
//! - Global config files (~/.turbo/config.json)
//! - Local config files (.turbo/config.json)
//! - Environment variables
//! - CLI arguments
//!
//! Configuration is merged with a priority order where higher-priority sources
//! override lower-priority ones on a per-field basis. Scalar fields use
//! first-writer-wins (`overwrite_none`). Nested config objects like OTEL
//! options are deep-merged so that a higher-priority source overriding a
//! single field does not shadow unrelated fields from lower-priority sources.
//!
//! Security-sensitive fields (`headers`, `use_remote_cache_token`) are coupled
//! to `endpoint`: once an endpoint is set by a source, credentials from
//! lower-priority sources are discarded. See `ExperimentalOtelOptions::merge`.

// Structured config errors are returned throughout configuration resolution;
// Clippy reports 29 sites. Keep the shared diagnostic error type unchanged.
#![allow(clippy::result_large_err)]

mod env;
mod experimental_otel;
mod file;
mod override_env;
mod turbo_json;

use std::{
    collections::HashMap,
    ffi::OsString,
    io::{self, IsTerminal},
};

use camino::{Utf8Path, Utf8PathBuf};
use env::EnvVars;
pub use experimental_otel::{
    ExperimentalOtelMetricsOptions, ExperimentalOtelOptions, ExperimentalOtelProtocol,
    ExperimentalOtelRunAttributesOptions, ExperimentalOtelTaskAttributesOptions,
};
pub use file::ConfigurationFileInputs;
use file::{AuthFile, ConfigFile};
use miette::Diagnostic;
use override_env::OverrideEnvVars;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;
use turbo_json::TurboJsonReader;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_cache::CacheConfig;
use turborepo_repository::{
    discovery::{MultipleTurboConfigsError, select_turbo_config_path},
    package_graph::PackageName,
};
use turborepo_scm::WorktreeInfo;
use turborepo_turbo_json::FutureFlags;
/// Configuration file names, shared with the loader.
pub use turborepo_types::{CONFIG_FILE, CONFIG_FILE_JSONC};
use turborepo_types::{ConfigurationSource, Credential, EnvMode, LogOrder, UIMode};

// The first Some value wins as config sources are merged highest-priority
// first.
fn merge_option<T>(left: &mut Option<T>, right: Option<T>) {
    if left.is_none() {
        *left = right;
    }
}

trait Merge {
    fn merge(&mut self, other: Self);
}

fn merge_nested<T: Merge>(left: &mut Option<T>, right: Option<T>) {
    if let Some(right) = right {
        if let Some(left) = left {
            left.merge(right);
        } else {
            *left = Some(right);
        }
    }
}

// Destructure every field so adding a new configuration field requires an
// explicit merge strategy rather than silently dropping it.
macro_rules! merge_fields {
    ($left:ident, $right:ident; $($field:ident),+; nested = $nested:ident) => {
        let Self { $($field,)+ $nested } = $right;
        $(merge_option(&mut $left.$field, $field);)+
        merge_nested(&mut $left.$nested, $nested);
    };
}

/// Wrapper for observability-related config. Deep-merges `otel` so that
/// a partial `ExperimentalOtelOptions` from one source (e.g. a single env var)
/// does not shadow the entire block from a lower-priority source.
#[derive(Deserialize, Serialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalObservabilityOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otel: Option<ExperimentalOtelOptions>,
}

impl Merge for ExperimentalObservabilityOptions {
    fn merge(&mut self, other: Self) {
        merge_nested(&mut self.otel, other.otel);
    }
}

/// Configuration for structured log file output.
///
/// - `LogFileConfig::Enabled` — write to default location
///   (`.turbo/logs/<epoch_millis>.json`)
/// - `LogFileConfig::Path(p)` — write to a custom file path
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum LogFileConfig {
    Enabled,
    Path(String),
}

impl<'de> Deserialize<'de> for LogFileConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        match value {
            serde_json::Value::Bool(true) => Ok(LogFileConfig::Enabled),
            serde_json::Value::Bool(false) => Err(serde::de::Error::custom(
                "use null/absent instead of false to disable log file",
            )),
            serde_json::Value::String(path) => Ok(LogFileConfig::Path(path)),
            _ => Err(serde::de::Error::custom(
                "expected true or a file path string",
            )),
        }
    }
}

// Re-export default constants for tests and external use
pub const DEFAULT_API_URL: &str = "https://vercel.com/api";
pub const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
pub const DEFAULT_TIMEOUT: u64 = 30;
pub const DEFAULT_UPLOAD_TIMEOUT: u64 = 60;
pub const DEFAULT_TUI_SCROLLBACK_LENGTH: u64 = 2048;

/// Result of resolving the cache directory, including worktree sharing status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheDirResult {
    /// The resolved cache directory path.
    pub path: Utf8PathBuf,
    /// True if using shared cache from main worktree.
    /// This is used for messaging to inform users when worktree cache sharing
    /// is active.
    pub is_shared_worktree: bool,
    /// The git repository root, if detected during worktree resolution.
    /// Captured here so `SCM::new` can skip its own `git rev-parse` call.
    pub git_root: Option<AbsoluteSystemPathBuf>,
}

/// Configuration errors for turborepo.
///
/// This enum contains errors related to configuration loading and validation.
/// Turbo.json specific errors are in `turborepo_turbo_json::Error` and can be
/// converted via the `TurboJsonError` variant.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    // ============================================================
    // Authentication and global config errors
    // ============================================================
    #[error("Authentication error: {0}")]
    Auth(#[from] turborepo_auth::Error),
    #[error("Global config path not found.")]
    NoGlobalConfigPath,
    #[error("Global authentication file path not found.")]
    NoGlobalAuthFilePath,
    #[error("Global config directory not found.")]
    NoGlobalConfigDir,

    // ============================================================
    // File and IO errors
    // ============================================================
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Camino(#[from] camino::FromPathBufError),
    #[error("Encountered an I/O error while attempting to read {config_path}: {error}")]
    FailedToReadConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error("Encountered an I/O error while attempting to set {config_path}: {error}")]
    FailedToSetConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error(transparent)]
    Cache(#[from] turborepo_cache::config::Error),
    #[error(transparent)]
    PathError(#[from] turbopath::PathError),

    // ============================================================
    // API and network errors
    // ============================================================
    #[error("Failed to create APIClient: {0}")]
    ApiClient(#[source] turborepo_api_client::Error),

    // ============================================================
    // Environment variable parsing errors
    // ============================================================
    #[error("{0} is not UTF8.")]
    Encoding(String),
    #[error("TURBO_SIGNATURE should be either 1 or 0.")]
    InvalidSignature,
    #[error("TURBO_REMOTE_CACHE_TIMEOUT: Error parsing timeout.")]
    InvalidRemoteCacheTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_REMOTE_CACHE_UPLOAD_TIMEOUT: Error parsing timeout.")]
    InvalidUploadTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_PREFLIGHT should be either 1 or 0.")]
    InvalidPreflight,
    #[error("TURBO_LOG_ORDER should be one of: {0}")]
    InvalidLogOrder(String),
    #[error(
        "TURBO_TUI_SCROLLBACK_LENGTH: Invalid value. Use a number for how many lines to keep in \
         scrollback."
    )]
    InvalidTuiScrollbackLength(#[source] std::num::ParseIntError),
    #[error("TURBO_SSO_LOGIN_CALLBACK_PORT: Invalid value. Use a number for the callback port.")]
    InvalidSsoLoginCallbackPort(#[source] std::num::ParseIntError),
    #[error("Invalid experimentalOtel configuration: {message}")]
    InvalidExperimentalOtelConfig { message: String },

    // ============================================================
    // Turbo.json loading errors (specific to loader, not in turbo_json crate)
    // ============================================================
    #[error("Cannot load turbo.json for {0} in single package mode.")]
    InvalidTurboJsonLoad(PackageName),

    // ============================================================
    // Turbo.json errors (delegated to turborepo_turbo_json crate)
    // ============================================================
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJsonError(#[from] turborepo_turbo_json::Error),
}

impl Error {
    /// Returns true if this error indicates that no turbo.json file was found.
    pub fn is_no_turbo_json(&self) -> bool {
        matches!(
            self,
            Error::TurboJsonError(turborepo_turbo_json::Error::NoTurboJSON)
        )
    }
}

// Conversion from LoaderError to config::Error
// This is needed for TurboJsonLoader's generic update mechanism
impl From<turborepo_turbo_json::LoaderError> for Error {
    fn from(err: turborepo_turbo_json::LoaderError) -> Self {
        match err {
            turborepo_turbo_json::LoaderError::TurboJson(e) => Error::TurboJsonError(e),
            turborepo_turbo_json::LoaderError::InvalidTurboJsonLoad(pkg) => {
                Error::InvalidTurboJsonLoad(pkg)
            }
        }
    }
}

// We intentionally don't derive Serialize so that different parts
// of the code that want to display the config can tune how they
// want to display and what fields they want to include.
#[derive(Deserialize, Default, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationOptions {
    #[serde(alias = "apiurl")]
    #[serde(alias = "ApiUrl")]
    #[serde(alias = "APIURL")]
    pub api_url: Option<String>,
    #[serde(skip)]
    pub api_url_source: Option<ConfigurationSource>,
    #[serde(alias = "loginurl")]
    #[serde(alias = "LoginUrl")]
    #[serde(alias = "LOGINURL")]
    pub login_url: Option<String>,
    #[serde(skip)]
    pub login_url_source: Option<ConfigurationSource>,
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
    pub token: Option<Credential>,
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
    #[serde(rename = "cacheMaxAge")]
    pub cache_max_age: Option<String>,
    #[serde(rename = "cacheMaxSize")]
    pub cache_max_size: Option<String>,
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
    /// Whether turbo should maintain its managed guidance block in `AGENTS.md`.
    #[serde(skip)]
    pub agent_guidance: Option<bool>,
    pub sso_login_callback_port: Option<u16>,
    #[serde(skip)]
    pub future_flags: Option<FutureFlags>,
    /// Deep-merged so that setting a single OTEL env var does not shadow the
    /// entire observability block from turbo.json. See
    /// `ExperimentalOtelOptions::merge` for credential-locking semantics.
    #[serde(rename = "experimentalObservability")]
    pub experimental_observability: Option<ExperimentalObservabilityOptions>,
    /// Structured log file destination, configured via `logFile` in
    /// turbo.json or `TURBO_LOG_FILE` env var.
    #[serde(rename = "logFile")]
    pub log_file: Option<LogFileConfig>,
}

impl Merge for ConfigurationOptions {
    fn merge(&mut self, other: Self) {
        merge_fields!(self, other;
            api_url, api_url_source, login_url, login_url_source,
            team_slug, team_id, token, signature, preflight, timeout,
            upload_timeout, enabled, ui, allow_no_package_manager, daemon,
            env_mode, scm_base, scm_head, cache_dir, cache_max_age,
            cache_max_size, root_turbo_json_path, force, log_order, cache,
            remote_only, remote_cache_read_only, run_summary,
            allow_no_turbo_json, tui_scrollback_length, concurrency,
            no_update_notifier, agent_guidance, sso_login_callback_port, future_flags,
            log_file; nested = experimental_observability
        );
    }
}

#[derive(Default)]
pub struct TurborepoConfigBuilder {
    repo_root: AbsoluteSystemPathBuf,
    override_config: ConfigurationOptions,
    global_config_path: Option<AbsoluteSystemPathBuf>,
    environment: Option<HashMap<OsString, OsString>>,
}

// Keep the fluent setters on both types in sync. Each setter accepts the full
// Option<T>, including None, and the builder writes to its override layer.
macro_rules! configuration_setters {
    ($($method:ident: $field:ident: $ty:ty),* $(,)?) => {
        impl ConfigurationOptions {
            $(
                pub fn $method(mut self, value: $ty) -> Self {
                    self.$field = value;
                    self
                }
            )*
        }

        impl TurborepoConfigBuilder {
            $(
                pub fn $method(mut self, value: $ty) -> Self {
                    self.override_config.$field = value;
                    self
                }
            )*
        }
    };
}

configuration_setters! {
    with_api_url: api_url: Option<String>,
    with_api_url_source: api_url_source: Option<ConfigurationSource>,
    with_login_url: login_url: Option<String>,
    with_login_url_source: login_url_source: Option<ConfigurationSource>,
    with_team_slug: team_slug: Option<String>,
    with_team_id: team_id: Option<String>,
    with_signature: signature: Option<bool>,
    with_preflight: preflight: Option<bool>,
    with_timeout: timeout: Option<u64>,
    with_upload_timeout: upload_timeout: Option<u64>,
    with_enabled: enabled: Option<bool>,
    with_ui: ui: Option<UIMode>,
    with_allow_no_package_manager: allow_no_package_manager: Option<bool>,
    with_daemon: daemon: Option<bool>,
    with_env_mode: env_mode: Option<EnvMode>,
    with_scm_base: scm_base: Option<String>,
    with_scm_head: scm_head: Option<String>,
    with_cache_dir: cache_dir: Option<Utf8PathBuf>,
    with_cache_max_age: cache_max_age: Option<String>,
    with_cache_max_size: cache_max_size: Option<String>,
    with_root_turbo_json_path: root_turbo_json_path: Option<AbsoluteSystemPathBuf>,
    with_force: force: Option<bool>,
    with_log_order: log_order: Option<LogOrder>,
    with_cache: cache: Option<CacheConfig>,
    with_remote_only: remote_only: Option<bool>,
    with_remote_cache_read_only: remote_cache_read_only: Option<bool>,
    with_run_summary: run_summary: Option<bool>,
    with_allow_no_turbo_json: allow_no_turbo_json: Option<bool>,
    with_tui_scrollback_length: tui_scrollback_length: Option<u64>,
    with_concurrency: concurrency: Option<String>,
    with_no_update_notifier: no_update_notifier: Option<bool>,
    with_sso_login_callback_port: sso_login_callback_port: Option<u16>,
    with_future_flags: future_flags: Option<FutureFlags>,
    with_experimental_observability: experimental_observability: Option<ExperimentalObservabilityOptions>,
    with_log_file: log_file: Option<LogFileConfig>,
}

// Getters
impl ConfigurationOptions {
    fn with_url_sources(mut self, source: ConfigurationSource) -> Self {
        if self.api_url.is_some() {
            self.api_url_source = Some(source);
        }
        if self.login_url.is_some() {
            self.login_url_source = Some(source);
        }

        self
    }

    pub fn api_url(&self) -> &str {
        non_empty_str(self.api_url.as_deref()).unwrap_or(DEFAULT_API_URL)
    }

    pub fn login_url(&self) -> &str {
        non_empty_str(self.login_url.as_deref()).unwrap_or(DEFAULT_LOGIN_URL)
    }

    pub fn api_url_source(&self) -> Option<ConfigurationSource> {
        self.api_url_source
    }

    pub fn login_url_source(&self) -> Option<ConfigurationSource> {
        self.login_url_source
    }

    pub fn team_slug(&self) -> Option<&str> {
        self.team_slug
            .as_deref()
            .and_then(|slug| (!slug.is_empty()).then_some(slug))
    }

    pub fn team_id(&self) -> Option<&str> {
        non_empty_str(self.team_id.as_deref())
    }

    pub fn credential(&self) -> Option<&Credential> {
        self.token
            .as_ref()
            .filter(|token| !token.expose().is_empty())
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.map(Credential::from);
        self
    }

    pub fn token(&self) -> Option<&str> {
        self.credential().map(Credential::expose)
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
        if !std::io::stdout().is_terminal() {
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
        // The daemon is no longer used for `turbo run`. Always return None
        // (no daemon preference) regardless of configuration.
        if self.daemon.is_some() {
            tracing::warn!(
                "the `daemon` configuration option is deprecated and will be removed in version \
                 3.0. The daemon is no longer used for `turbo run`."
            );
        }
        None
    }

    pub fn env_mode(&self) -> EnvMode {
        self.env_mode.unwrap_or_default()
    }

    /// The default cache directory path (relative to repo root).
    pub const DEFAULT_CACHE_DIR: &'static str = if cfg!(windows) {
        ".turbo\\cache"
    } else {
        ".turbo/cache"
    };

    /// Returns the configured cache directory path without worktree resolution.
    /// Use `resolve_cache_dir()` for full worktree-aware resolution.
    pub fn cache_dir(&self) -> &Utf8Path {
        self.cache_dir
            .as_deref()
            .unwrap_or_else(|| Utf8Path::new(Self::DEFAULT_CACHE_DIR))
    }

    pub fn cache_max_age(&self) -> Option<&str> {
        self.cache_max_age.as_deref()
    }

    pub fn cache_max_size(&self) -> Option<&str> {
        self.cache_max_size.as_deref()
    }

    /// Resolves the cache directory, taking Git worktrees into account.
    ///
    /// If no explicit `cacheDir` is configured and we're in a linked Git
    /// worktree, this returns the cache directory from the main worktree to
    /// enable cache sharing.
    ///
    /// # Arguments
    ///
    /// * `repo_root` - The root of the current repository/worktree
    ///
    /// # Returns
    ///
    /// A `CacheDirResult` containing:
    /// - `path`: The resolved cache directory path
    /// - `is_shared_worktree`: True if using shared cache from main worktree
    pub fn resolve_cache_dir(&self, repo_root: &AbsoluteSystemPath) -> CacheDirResult {
        if let Some(explicit_cache_dir) = &self.cache_dir {
            return CacheDirResult {
                path: explicit_cache_dir.clone(),
                is_shared_worktree: false,
                git_root: None,
            };
        }

        let worktree_info = WorktreeInfo::detect(repo_root).ok();
        self.resolve_cache_dir_with_worktree_info(worktree_info.as_ref())
    }

    /// Resolve cache directory using pre-computed worktree info.
    ///
    /// This variant avoids spawning a git subprocess, which allows the caller
    /// to run worktree detection on a background thread and pass the result in.
    pub fn resolve_cache_dir_with_worktree_info(
        &self,
        worktree_info: Option<&WorktreeInfo>,
    ) -> CacheDirResult {
        if let Some(explicit_cache_dir) = &self.cache_dir {
            return CacheDirResult {
                path: explicit_cache_dir.clone(),
                is_shared_worktree: false,
                git_root: None,
            };
        }

        match worktree_info {
            Some(worktree_info) => {
                debug!(
                    "Worktree detection: current={}, main={}, is_linked={}",
                    worktree_info.worktree_root,
                    worktree_info.main_worktree_root,
                    worktree_info.is_linked_worktree()
                );
                let git_root = Some(worktree_info.git_root.clone());
                if worktree_info.is_linked_worktree() {
                    let main_cache_path = worktree_info
                        .main_worktree_root
                        .join_component(".turbo")
                        .join_component("cache");
                    let result = CacheDirResult {
                        path: Utf8PathBuf::from(main_cache_path.as_str()),
                        is_shared_worktree: true,
                        git_root,
                    };
                    debug!("Using shared worktree cache at: {}", result.path);
                    result
                } else {
                    debug!(
                        "Using local cache (main worktree): {}",
                        Self::DEFAULT_CACHE_DIR
                    );
                    CacheDirResult {
                        path: Utf8PathBuf::from(Self::DEFAULT_CACHE_DIR),
                        is_shared_worktree: false,
                        git_root,
                    }
                }
            }
            None => {
                debug!(
                    "No worktree info available, using local cache: {}",
                    Self::DEFAULT_CACHE_DIR
                );
                CacheDirResult {
                    path: Utf8PathBuf::from(Self::DEFAULT_CACHE_DIR),
                    is_shared_worktree: false,
                    git_root: None,
                }
            }
        }
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

    pub fn agent_guidance(&self) -> bool {
        self.agent_guidance.unwrap_or(false)
    }

    pub fn sso_login_callback_port(&self) -> Option<u16> {
        self.sso_login_callback_port
    }

    pub fn future_flags(&self) -> FutureFlags {
        self.future_flags.unwrap_or_default()
    }

    pub fn experimental_observability(&self) -> Option<&ExperimentalObservabilityOptions> {
        self.experimental_observability.as_ref()
    }

    pub fn log_file(&self) -> Option<&LogFileConfig> {
        self.log_file.as_ref()
    }
}

// Maps Some("") to None to emulate how Go handles empty strings
fn non_empty_str(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

pub(crate) trait ResolvedConfigurationOptions {
    fn get_configuration_options(
        &self,
        existing_config: &ConfigurationOptions,
    ) -> Result<ConfigurationOptions, Error>;
}

// Used for global config and local config.
impl ResolvedConfigurationOptions for &ConfigurationOptions {
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

impl TurborepoConfigBuilder {
    pub fn new(repo_root: &AbsoluteSystemPath) -> Self {
        Self {
            repo_root: repo_root.to_owned(),
            override_config: Default::default(),
            global_config_path: None,
            environment: None,
        }
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.override_config.token = token.map(Credential::from);
        self
    }

    pub fn with_global_config_path(mut self, path: AbsoluteSystemPathBuf) -> Self {
        self.global_config_path = Some(path);
        self
    }

    /// Applies a fully-formed override layer as the highest-precedence source.
    pub fn with_override_config(mut self, override_config: ConfigurationOptions) -> Self {
        self.override_config.merge(override_config);
        self
    }

    fn get_environment(&self) -> HashMap<OsString, OsString> {
        self.environment
            .clone()
            .unwrap_or_else(get_lowercased_env_vars)
    }

    pub fn build(&self) -> Result<ConfigurationOptions, Error> {
        self.build_with_optional_inputs(self.get_environment(), None)
    }

    /// Resolves configuration against an explicit environment snapshot and
    /// explicit global config/auth file paths.
    pub fn build_with_inputs(
        &self,
        environment: HashMap<OsString, OsString>,
        file_inputs: ConfigurationFileInputs,
    ) -> Result<ConfigurationOptions, Error> {
        self.build_with_optional_inputs(environment, Some(file_inputs))
    }

    fn build_with_optional_inputs(
        &self,
        environment: HashMap<OsString, OsString>,
        file_inputs: Option<ConfigurationFileInputs>,
    ) -> Result<ConfigurationOptions, Error> {
        let environment = environment
            .into_iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect::<HashMap<_, _>>();
        // Sources are listed highest-to-lowest priority. The fold merges each
        // source into the accumulator; `overwrite_none` keeps the first `Some`
        // it sees, so processing highest-priority first gives it precedence.
        //
        // Priority order:
        //   1. builder pattern overrides (CLI arguments)
        //   2. environment variables (TURBO_*)
        //   3. override env vars (VERCEL_ARTIFACTS_*)
        //   4. local configuration (<REPO_ROOT>/.turbo/config.json)
        //   5. global credentials selected for the final API URL
        //   6. global configuration (~/.turbo/config.json)
        //   7. shared configuration (turbo.json)
        //
        // See `test_experimental_observability_otel_precedence` for coverage.

        let turbo_json = TurboJsonReader::new(&self.repo_root);
        let global_config_file = match &file_inputs {
            Some(inputs) => ConfigFile::from_path(inputs.global_config_path.clone()),
            None => ConfigFile::global_config(self.global_config_path.clone())?,
        };
        let mut global_config =
            global_config_file.get_configuration_options(&ConfigurationOptions::default())?;
        let global_token = global_config
            .token
            .take()
            .filter(|token| !token.expose().is_empty());
        let global_auth = match &file_inputs {
            Some(inputs) => AuthFile::from_paths(
                inputs.global_auth_path.clone(),
                global_token,
                inputs.legacy_auth_path.clone(),
            ),
            None => AuthFile::global_auth(self.global_config_path.clone(), global_token)?,
        };
        let local_config = ConfigFile::local_config(&self.repo_root);
        let env_vars = environment;
        let env_var_config = EnvVars::new(&env_vars)?;
        let override_env_var_config = OverrideEnvVars::new(&env_vars)?;

        let sources: [(ConfigurationSource, Box<dyn ResolvedConfigurationOptions>); 6] = [
            (ConfigurationSource::Cli, Box::new(&self.override_config)),
            (ConfigurationSource::Environment, Box::new(env_var_config)),
            (
                ConfigurationSource::OverrideEnvironment,
                Box::new(override_env_var_config),
            ),
            (ConfigurationSource::LocalConfig, Box::new(local_config)),
            (ConfigurationSource::GlobalConfig, Box::new(&global_config)),
            (ConfigurationSource::TurboJson, Box::new(turbo_json)),
        ];

        let mut config = sources.into_iter().try_fold(
            ConfigurationOptions::default(),
            |mut acc, (source, current_source)| -> Result<_, Error> {
                let current_source_config = current_source.get_configuration_options(&acc)?;
                let current_source_config = current_source_config.with_url_sources(source);
                acc.merge(current_source_config);
                Ok(acc)
            },
        )?;
        config.merge(global_auth.get_configuration_options(&config)?);
        Ok(config)
    }
}

/// Given a directory path, determines which turbo.json configuration file to
/// use. Returns an error if both turbo.json and turbo.jsonc exist in the same
/// directory. Returns the path to the config file to use, defaulting to
/// turbo.json if neither exists.
pub fn resolve_turbo_config_path(
    dir_path: &turbopath::AbsoluteSystemPath,
) -> Result<turbopath::AbsoluteSystemPathBuf, Error> {
    let turbo_json_path = dir_path.join_component(CONFIG_FILE);
    let turbo_jsonc_path = dir_path.join_component(CONFIG_FILE_JSONC);

    select_turbo_config_path(
        dir_path,
        turbo_json_path.try_exists()?,
        turbo_jsonc_path.try_exists()?,
    )
    .map(|path| path.unwrap_or(turbo_json_path))
    .map_err(|MultipleTurboConfigsError { directory }| {
        Error::TurboJsonError(turborepo_turbo_json::Error::MultipleTurboConfigs { directory })
    })
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, ffi::OsString};

    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
    use turborepo_types::{ConfigurationSource, LogOrder};

    use crate::{
        CONFIG_FILE, CONFIG_FILE_JSONC, ConfigurationOptions, DEFAULT_API_URL, DEFAULT_LOGIN_URL,
        DEFAULT_TIMEOUT, ExperimentalObservabilityOptions, ExperimentalOtelMetricsOptions,
        ExperimentalOtelOptions, ExperimentalOtelProtocol, Merge, TurborepoConfigBuilder,
    };

    #[test]
    fn test_generated_setters() {
        let repo_root = AbsoluteSystemPath::new(if cfg!(windows) {
            "C:\\fake\\repo"
        } else {
            "/fake/repo"
        })
        .unwrap();
        let config = ConfigurationOptions::default()
            .with_api_url(Some("https://api.example".into()))
            .with_timeout(Some(42))
            .with_daemon(Some(false))
            .with_api_url(None)
            .with_log_file(Some(crate::LogFileConfig::Enabled));

        assert_eq!(config.api_url, None);
        assert_eq!(config.timeout, Some(42));
        assert_eq!(config.daemon, Some(false));
        assert_eq!(config.log_file, Some(crate::LogFileConfig::Enabled));

        let builder = TurborepoConfigBuilder::new(repo_root)
            .with_api_url(Some("https://api.example".into()))
            .with_timeout(Some(42))
            .with_daemon(Some(false))
            .with_api_url(None)
            .with_log_file(Some(crate::LogFileConfig::Enabled));
        assert_eq!(builder.override_config, config);
    }

    #[test]
    fn test_agent_guidance_merge() {
        let mut config = ConfigurationOptions::default();
        config.merge(ConfigurationOptions {
            agent_guidance: Some(false),
            ..Default::default()
        });
        assert!(!config.agent_guidance());

        config.merge(ConfigurationOptions {
            agent_guidance: Some(true),
            ..Default::default()
        });
        assert!(!config.agent_guidance());
    }

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
    fn test_env_layering() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let global_config_path = AbsoluteSystemPathBuf::try_from(
            TempDir::new().unwrap().path().join("nonexistent.json"),
        )
        .unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{}"#)
            .unwrap();

        let turbo_teamid = "team_nLlpyC6REAqxydlFKbrMDlud";
        let turbo_token = "abcdef1234567890abcdef";
        let vercel_artifacts_owner = "team_SOMEHASH";
        let vercel_artifacts_token = "correct-horse-battery-staple";

        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert("turbo_teamid".into(), turbo_teamid.into());
        env.insert("turbo_token".into(), turbo_token.into());
        env.insert(
            "vercel_artifacts_token".into(),
            vercel_artifacts_token.into(),
        );
        env.insert(
            "vercel_artifacts_owner".into(),
            vercel_artifacts_owner.into(),
        );

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config: Default::default(),
            global_config_path: Some(global_config_path),
            environment: Some(env),
        };

        let config = builder.build().unwrap();
        assert_eq!(config.team_id().unwrap(), turbo_teamid);
        assert_eq!(config.token().unwrap(), turbo_token);
    }

    #[test]
    fn test_override_config_precedes_environment_log_order() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{}"#)
            .unwrap();

        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert("turbo_log_order".into(), "grouped".into());

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions {
                log_order: Some(LogOrder::Stream),
                ..Default::default()
            },
            global_config_path: None,
            environment: Some(env),
        };

        let config = builder.build().unwrap();
        assert_eq!(config.log_order(), LogOrder::Stream);
    }

    #[test]
    fn test_turbo_json_remote_cache() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        let api_url = "url1";
        let login_url = "url2";
        let team_slug = "my-slug";
        let team_id = "an-id";
        let turbo_json_contents = serde_json::to_string_pretty(&serde_json::json!({
            "remoteCache": {
                "enabled": true,
                "apiUrl": api_url,
                "loginUrl": login_url,
                "teamSlug": team_slug,
                "teamId": team_id,
                "signature": true,
                "preflight": false,
                "timeout": 123
            }
        }))
        .unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents(&turbo_json_contents)
            .unwrap();

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions::default(),
            global_config_path: None,
            environment: Some(HashMap::default()),
        };

        let config = builder.build().unwrap();
        // Directly accessing field to make sure we're not getting the default value
        assert_eq!(config.enabled, Some(true));
        assert_eq!(config.api_url(), api_url);
        assert_eq!(config.login_url(), login_url);
        assert_eq!(config.team_slug(), Some(team_slug));
        assert_eq!(config.team_id(), Some(team_id));
        assert!(config.signature());
        assert!(!config.preflight());
        assert_eq!(config.timeout(), 123);
    }

    #[test]
    fn test_experimental_observability_otel_precedence() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        // Lowest-priority source: turbo.json
        let turbo_json_contents = serde_json::to_string_pretty(&serde_json::json!({
            "futureFlags": {
                "experimentalObservability": true
            },
            "experimentalObservability": {
                "otel": {
                    "enabled": true,
                    "endpoint": "https://turbo-json.example/otel",
                    "timeoutMs": 1111,
                    "metrics": {
                        "runSummary": true,
                        "taskDetails": false
                    }
                }
            }
        }))
        .unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents(&turbo_json_contents)
            .unwrap();

        // Higher-priority source: env vars
        let mut env: HashMap<OsString, OsString> = HashMap::new();
        env.insert(
            "turbo_experimental_otel_endpoint".into(),
            "https://env.example/otel".to_string().into(),
        );
        env.insert(
            "turbo_experimental_otel_timeout_ms".into(),
            "2222".to_string().into(),
        );
        env.insert(
            "turbo_experimental_otel_metrics_task_details".into(),
            "1".to_string().into(),
        );
        env.insert(
            "turbo_experimental_otel_metrics_task_attributes_id".into(),
            "1".to_string().into(),
        );

        // Highest-priority source: builder overrides
        let override_config = ConfigurationOptions {
            experimental_observability: Some(ExperimentalObservabilityOptions {
                otel: Some(ExperimentalOtelOptions {
                    endpoint: Some("https://override.example/otel".to_string()),
                    protocol: Some(ExperimentalOtelProtocol::HttpProtobuf),
                    timeout_ms: Some(3333),
                    metrics: Some(ExperimentalOtelMetricsOptions {
                        run_summary: Some(false),
                        task_details: Some(false),
                        run_attributes: None,
                        task_attributes: None,
                    }),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config,
            global_config_path: None,
            environment: Some(env),
        };

        let config = builder.build().unwrap();
        let otel = config
            .experimental_observability()
            .and_then(|obs| obs.otel.as_ref())
            .expect("expected experimental observability otel config");

        // Builder override wins for fields it explicitly sets.
        assert_eq!(
            otel.endpoint.as_deref(),
            Some("https://override.example/otel")
        );
        assert_eq!(otel.protocol, Some(ExperimentalOtelProtocol::HttpProtobuf));
        assert_eq!(otel.timeout_ms, Some(3333));
        assert_eq!(
            otel.metrics.as_ref().and_then(|m| m.run_summary),
            Some(false)
        );
        // Override's task_details=false wins over env's task_details=true.
        assert_eq!(
            otel.metrics.as_ref().and_then(|m| m.task_details),
            Some(false)
        );

        // Fields not set by higher-priority sources are filled in by lower-priority
        // sources via deep merge.  Override did not set `enabled`, so turbo.json's
        // `enabled: true` is used.
        assert_eq!(otel.enabled, Some(true));
        // Override did not set `task_attributes`; env var set it, so it is used.
        assert_eq!(
            otel.metrics
                .as_ref()
                .and_then(|m| m.task_attributes.as_ref())
                .and_then(|ta| ta.id),
            Some(true)
        );
    }

    #[test]
    fn test_multiple_turbo_configs() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        // Create both turbo.json and turbo.jsonc
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);

        turbo_json_path.create_with_contents("{}").unwrap();
        turbo_jsonc_path.create_with_contents("{}").unwrap();

        // Test ConfigurationOptions.root_turbo_json_path
        let config = ConfigurationOptions::default();
        let result = config.root_turbo_json_path(repo_root);
        assert!(result.is_err());
    }

    #[test]
    fn test_only_turbo_json() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        // Create only turbo.json
        let turbo_json_path = repo_root.join_component(CONFIG_FILE);
        turbo_json_path.create_with_contents("{}").unwrap();

        // Test ConfigurationOptions.root_turbo_json_path
        let config = ConfigurationOptions::default();
        let result = config.root_turbo_json_path(repo_root);

        assert_eq!(result.unwrap(), turbo_json_path);
    }

    #[test]
    fn test_only_turbo_jsonc() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        // Create only turbo.jsonc
        let turbo_jsonc_path = repo_root.join_component(CONFIG_FILE_JSONC);
        turbo_jsonc_path.create_with_contents("{}").unwrap();

        // Test ConfigurationOptions.root_turbo_json_path
        let config = ConfigurationOptions::default();
        let result = config.root_turbo_json_path(repo_root);

        assert_eq!(result.unwrap(), turbo_jsonc_path);
    }

    #[test]
    fn test_no_turbo_config() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        // Test ConfigurationOptions.root_turbo_json_path
        let config = ConfigurationOptions::default();
        let result = config.root_turbo_json_path(repo_root);

        assert_eq!(result.unwrap(), repo_root.join_component(CONFIG_FILE));
    }

    #[test]
    fn test_resolve_cache_dir_explicit_overrides_worktree() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        let config = ConfigurationOptions {
            cache_dir: Some(camino::Utf8PathBuf::from("/explicit/cache")),
            ..Default::default()
        };

        let result = config.resolve_cache_dir(repo_root);

        assert_eq!(result.path, "/explicit/cache");
        assert!(
            !result.is_shared_worktree,
            "Explicit cacheDir should not be marked as shared worktree"
        );
    }

    #[test]
    fn test_resolve_cache_dir_non_git_fallback() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        // Don't initialize git - just use the temp directory
        let config = ConfigurationOptions::default();

        let result = config.resolve_cache_dir(repo_root);

        // Should fall back to default cache dir
        assert_eq!(result.path, ConfigurationOptions::DEFAULT_CACHE_DIR);
        assert!(
            !result.is_shared_worktree,
            "Non-git directory should not be marked as shared worktree"
        );
    }

    #[test]
    fn test_resolve_cache_dir_default_returns_relative_path() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init", "."])
            .current_dir(&repo_root)
            .output()
            .expect("git init failed");

        let config = ConfigurationOptions::default();
        let result = config.resolve_cache_dir(&repo_root);

        // In main worktree, should return relative default path
        assert_eq!(result.path, ConfigurationOptions::DEFAULT_CACHE_DIR);
        assert!(
            !result.is_shared_worktree,
            "Main worktree should not be marked as shared"
        );
    }

    #[test]
    fn test_resolve_cache_dir_captures_git_root() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        std::process::Command::new("git")
            .args(["init", "."])
            .current_dir(&repo_root)
            .output()
            .expect("git init failed");

        let config = ConfigurationOptions::default();
        let result = config.resolve_cache_dir(&repo_root);

        // git_root should be captured from worktree detection so SCM::new
        // can skip its own git rev-parse subprocess
        assert!(
            result.git_root.is_some(),
            "git_root should be captured when worktree detection succeeds"
        );
        assert_eq!(
            result.git_root.unwrap(),
            repo_root,
            "git_root should match repo root in a non-worktree repo"
        );
    }

    #[test]
    fn test_resolve_cache_dir_explicit_skips_git_root() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPath::from_std_path(tmp_dir.path()).unwrap();

        let config = ConfigurationOptions {
            cache_dir: Some(camino::Utf8PathBuf::from("/my/cache")),
            ..Default::default()
        };

        let result = config.resolve_cache_dir(repo_root);

        // When explicit cache_dir is set, no worktree detection runs,
        // so git_root is not available
        assert!(
            result.git_root.is_none(),
            "git_root should be None when explicit cache_dir bypasses detection"
        );
    }

    /// Integration test that verifies linked worktree returns absolute path to
    /// main cache
    #[test]
    fn test_resolve_cache_dir_linked_worktree_returns_absolute_path() {
        use std::process::Command;

        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();

        // Initialize git repo with initial commit
        Command::new("git")
            .args(["init", "."])
            .current_dir(&repo_root)
            .output()
            .expect("git init failed");
        Command::new("git")
            .args(["config", "--local", "user.name", "test"])
            .current_dir(&repo_root)
            .output()
            .expect("git config failed");
        Command::new("git")
            .args(["config", "--local", "user.email", "test@example.com"])
            .current_dir(&repo_root)
            .output()
            .expect("git config failed");

        let readme = repo_root.join_component("README.md");
        readme.create_with_contents("# Test").unwrap();

        Command::new("git")
            .args(["add", "."])
            .current_dir(&repo_root)
            .output()
            .expect("git add failed");
        Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&repo_root)
            .output()
            .expect("git commit failed");

        // Create a linked worktree
        let worktree_path = repo_root.join_component("linked-worktree");
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                worktree_path.as_str(),
                "-b",
                "test-branch",
            ])
            .current_dir(&repo_root)
            .output()
            .expect("git worktree add failed");
        assert!(
            output.status.success(),
            "git worktree add failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );

        let config = ConfigurationOptions::default();

        // Test from main worktree - should return relative path
        let main_result = config.resolve_cache_dir(&repo_root);
        assert_eq!(
            main_result.path.as_str(),
            ConfigurationOptions::DEFAULT_CACHE_DIR
        );
        assert!(!main_result.is_shared_worktree);

        // Test from linked worktree - should return absolute path to main worktree's
        // cache
        let linked_result = config.resolve_cache_dir(&worktree_path);
        let expected_cache_path = repo_root.join_component(".turbo").join_component("cache");
        assert_eq!(
            linked_result.path.as_str(),
            expected_cache_path.as_str(),
            "Linked worktree should use main worktree's cache. Got: {}, Expected: {}",
            linked_result.path,
            expected_cache_path
        );
        assert!(
            linked_result.is_shared_worktree,
            "Linked worktree should be marked as shared"
        );
    }

    #[test]
    fn test_no_update_notifier_from_turbo_json_full_build() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(r#"{"noUpdateNotifier": true}"#)
            .unwrap();

        let builder = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions::default(),
            global_config_path: None,
            environment: Some(HashMap::default()),
        };

        let config = builder.build().unwrap();
        assert!(
            config.no_update_notifier(),
            "noUpdateNotifier from turbo.json should be respected"
        );
    }

    #[test]
    fn test_turbo_json_url_sources_are_recorded() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(
                r#"{
                    "remoteCache": {
                        "apiUrl": "https://attacker.test/api",
                        "loginUrl": "https://attacker.test"
                    }
                }"#,
            )
            .unwrap();

        let config = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions::default(),
            global_config_path: None,
            environment: Some(HashMap::default()),
        }
        .build()
        .unwrap();

        assert_eq!(
            config.api_url_source(),
            Some(ConfigurationSource::TurboJson)
        );
        assert_eq!(
            config.login_url_source(),
            Some(ConfigurationSource::TurboJson)
        );
    }

    // Use explicit auth paths and environment snapshots so these tests never
    // read developer credentials or depend on process-wide environment changes.
    #[test_case::test_case("turbo.json")]
    #[test_case::test_case("turbo.jsonc")]
    #[test_case::test_case(".turbo/config.json")]
    fn test_saved_vercel_classification_survives_api_url_overrides(config_file: &str) {
        const CUSTOM_API: &str = "http://127.0.0.1:9099";
        for token_source in ["auth", "global", "legacy", "env", "vercel_env", "cli"] {
            for authorization in ["none", "cli", "env", "global"] {
                let tmp = TempDir::new().unwrap();
                let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
                let repo = root.join_component("repo");
                repo.create_dir_all().unwrap();
                let config_path = repo.join_components(&config_file.split('/').collect::<Vec<_>>());
                config_path.ensure_dir().unwrap();
                let remote = serde_json::json!({
                    "apiUrl": CUSTOM_API,
                    "teamId": "team_test"
                });
                let contents = if config_file.starts_with(".turbo/") {
                    remote
                } else {
                    serde_json::json!({"remoteCache": remote})
                };
                config_path
                    .create_with_contents(contents.to_string())
                    .unwrap();

                let inputs = crate::ConfigurationFileInputs {
                    global_config_path: root.join_component("config.json"),
                    global_auth_path: root.join_component("auth.json"),
                    legacy_auth_path: Some(root.join_component("legacy-auth.json")),
                };
                let mut global = serde_json::json!({});
                let mut environment = HashMap::new();
                let mut builder = TurborepoConfigBuilder::new(&repo);
                match token_source {
                    "auth" => inputs
                        .global_auth_path
                        .create_with_contents(r#"{"token":"sentinel"}"#)
                        .unwrap(),
                    "global" => global["token"] = "sentinel".into(),
                    "legacy" => inputs
                        .legacy_auth_path
                        .as_ref()
                        .unwrap()
                        .create_with_contents(r#"{"token":"sentinel"}"#)
                        .unwrap(),
                    "env" => {
                        environment.insert("TURBO_TOKEN".into(), "sentinel".into());
                    }
                    "vercel_env" => {
                        environment.insert("VERCEL_ARTIFACTS_TOKEN".into(), "sentinel".into());
                        environment.insert("VERCEL_ARTIFACTS_OWNER".into(), "team_test".into());
                    }
                    "cli" => builder = builder.with_token(Some("sentinel".into())),
                    _ => unreachable!(),
                }
                match authorization {
                    "cli" => builder = builder.with_api_url(Some(CUSTOM_API.into())),
                    "env" => {
                        environment.insert("TURBO_API".into(), CUSTOM_API.into());
                    }
                    "global" => global["apiUrl"] = CUSTOM_API.into(),
                    "none" => {}
                    _ => unreachable!(),
                }
                inputs
                    .global_config_path
                    .create_with_contents(global.to_string())
                    .unwrap();
                let config = builder.build_with_inputs(environment, inputs).unwrap();
                assert_eq!(config.api_url(), CUSTOM_API);
                assert_eq!(config.token(), Some("sentinel"));
                assert_eq!(
                    config.credential().unwrap().is_saved_vercel(),
                    matches!(token_source, "auth" | "legacy"),
                    "{config_file}, {token_source}, {authorization}"
                );
            }
        }
    }

    #[test_case::test_case("repo_custom")]
    #[test_case::test_case("cli_custom")]
    #[test_case::test_case("env_custom")]
    #[test_case::test_case("global_custom")]
    #[test_case::test_case("local_custom")]
    #[test_case::test_case("selected_root_custom")]
    #[test_case::test_case("repo_vercel")]
    #[test_case::test_case("cli_vercel")]
    #[test_case::test_case("env_vercel")]
    #[test_case::test_case("default_vercel")]
    fn coexisting_credentials_follow_final_destination(url_source: &str) {
        use turborepo_types::Credential;

        const CUSTOM: &str = "http://127.0.0.1:9099";
        for stores in [
            "both",
            "auth_only",
            "legacy_only",
            "generic_legacy",
            "generic_only",
        ] {
            for explicit_token in ["none", "local", "vercel_env", "env", "cli"] {
                let tmp = TempDir::new().unwrap();
                let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
                let repo = root.join_component("repo");
                repo.create_dir_all().unwrap();
                let inputs = crate::ConfigurationFileInputs {
                    global_config_path: root.join_component("config.json"),
                    global_auth_path: root.join_component("auth.json"),
                    legacy_auth_path: Some(root.join_component("legacy.json")),
                };
                let custom = url_source.ends_with("custom");
                let api = if custom { CUSTOM } else { DEFAULT_API_URL };
                let repo_api = match url_source {
                    "repo_custom" | "cli_vercel" | "env_vercel" => CUSTOM,
                    _ => DEFAULT_API_URL,
                };
                repo.join_component("turbo.json")
                    .create_with_contents(
                        serde_json::json!({"remoteCache":{"apiUrl":repo_api}}).to_string(),
                    )
                    .unwrap();
                let mut global =
                    serde_json::json!({"teamId":"team_existing", "teamSlug":"existing-scope"});
                if matches!(stores, "both" | "generic_legacy" | "generic_only") {
                    global["token"] = "generic-global".into();
                }
                if matches!(stores, "both" | "auth_only") {
                    inputs
                        .global_auth_path
                        .create_with_contents(r#"{"token":"dedicated-vercel"}"#)
                        .unwrap();
                }
                if matches!(stores, "both" | "legacy_only" | "generic_legacy") {
                    inputs
                        .legacy_auth_path
                        .as_ref()
                        .unwrap()
                        .create_with_contents(r#"{"token":"legacy-vercel"}"#)
                        .unwrap();
                }
                let mut environment = HashMap::new();
                let mut builder = TurborepoConfigBuilder::new(&repo);
                match url_source {
                    "cli_custom" | "cli_vercel" => builder = builder.with_api_url(Some(api.into())),
                    "env_custom" | "env_vercel" => {
                        environment.insert("TURBO_API".into(), api.into());
                    }
                    "global_custom" => global["apiUrl"] = api.into(),
                    "local_custom" => {
                        let path = repo.join_components(&[".turbo", "config.json"]);
                        path.ensure_dir().unwrap();
                        path.create_with_contents(serde_json::json!({"apiUrl":api}).to_string())
                            .unwrap();
                    }
                    "selected_root_custom" => {
                        let path = repo.join_component("selected.json");
                        path.create_with_contents(
                            serde_json::json!({"remoteCache":{"apiUrl":api}}).to_string(),
                        )
                        .unwrap();
                        builder = builder.with_root_turbo_json_path(Some(path));
                    }
                    "default_vercel" => repo
                        .join_component("turbo.json")
                        .create_with_contents("{}")
                        .unwrap(),
                    _ => {}
                }
                match explicit_token {
                    "cli" => {
                        builder = builder.with_token(Some("explicit-cli".into()));
                        environment.insert("TURBO_TOKEN".into(), "explicit-env".into());
                    }
                    "env" => {
                        environment.insert("TURBO_TOKEN".into(), "explicit-env".into());
                    }
                    "vercel_env" => {
                        environment.insert(
                            "VERCEL_ARTIFACTS_TOKEN".into(),
                            "explicit-vercel_env".into(),
                        );
                        environment.insert("VERCEL_ARTIFACTS_OWNER".into(), "team_existing".into());
                    }
                    "local" => {
                        let path = repo.join_components(&[".turbo", "config.json"]);
                        path.ensure_dir().unwrap();
                        let mut local = if url_source == "local_custom" {
                            serde_json::json!({"apiUrl":api})
                        } else {
                            serde_json::json!({})
                        };
                        local["token"] = "explicit-local".into();
                        path.create_with_contents(local.to_string()).unwrap();
                    }
                    _ => {}
                }
                inputs
                    .global_config_path
                    .create_with_contents(global.to_string())
                    .unwrap();
                let config = builder.build_with_inputs(environment, inputs).unwrap();
                assert_eq!(config.api_url(), api, "{url_source}");
                let expected = if explicit_token != "none" {
                    Credential::from(format!("explicit-{explicit_token}"))
                } else if matches!(stores, "generic_only" | "generic_legacy")
                    || (custom && stores == "both")
                {
                    Credential::from("generic-global")
                } else if matches!(stores, "both" | "auth_only") {
                    Credential::saved_vercel("dedicated-vercel".into())
                } else {
                    Credential::saved_vercel("legacy-vercel".into())
                };
                assert_eq!(
                    config.credential(),
                    Some(&expected),
                    "{url_source}, {stores}, {explicit_token}"
                );
                assert_eq!(config.team_id(), Some("team_existing"));
                assert_eq!(config.team_slug(), Some("existing-scope"));
                assert_eq!(
                    turborepo_api_client::validate_credential_destination(
                        config.credential().unwrap(),
                        &api.parse().unwrap()
                    )
                    .is_ok(),
                    !custom || !expected.is_saved_vercel(),
                    "{url_source}, {stores}, {explicit_token}"
                );
            }
        }
    }

    #[tokio::test]
    async fn source_classification_controls_actual_cache_egress() {
        use turborepo_api_client::{APIClient, CacheClient};
        let server = httpmock::MockServer::start_async().await;
        for source in ["auth", "global", "legacy", "env", "cli"] {
            for override_token in [false, true] {
                let tmp = TempDir::new().unwrap();
                let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
                root.join_component("turbo.json")
                    .create_with_contents(
                        serde_json::json!({"remoteCache": {"apiUrl": server.base_url()}})
                            .to_string(),
                    )
                    .unwrap();
                let inputs = crate::ConfigurationFileInputs {
                    global_auth_path: root.join_component("auth.json"),
                    global_config_path: root.join_component("config.json"),
                    legacy_auth_path: Some(root.join_component("legacy.json")),
                };
                let mut environment = HashMap::new();
                let mut builder = TurborepoConfigBuilder::new(&root);
                let document = r#"{"token":"synthetic-source-sentinel","tokenPolicy":"Generic","credentialOrigin":"Generic"}"#;
                match source {
                    "auth" => inputs
                        .global_auth_path
                        .create_with_contents(document)
                        .unwrap(),
                    "global" => inputs
                        .global_config_path
                        .create_with_contents(document)
                        .unwrap(),
                    "legacy" => inputs
                        .legacy_auth_path
                        .as_ref()
                        .unwrap()
                        .create_with_contents(document)
                        .unwrap(),
                    "env" => {
                        environment
                            .insert("TURBO_TOKEN".into(), "synthetic-source-sentinel".into());
                    }
                    "cli" => builder = builder.with_token(Some("synthetic-source-sentinel".into())),
                    _ => unreachable!(),
                }
                builder = builder.with_api_url(Some(server.base_url()));
                if override_token {
                    builder = builder.with_token(Some("synthetic-source-sentinel".into()));
                }
                let config = builder.build_with_inputs(environment, inputs).unwrap();
                let client = APIClient::new(config.api_url(), None, None, "test", true).unwrap();
                let protected = matches!(source, "auth" | "legacy") && !override_token;
                let received = server
                    .mock_async(|when, then| {
                        when.header("authorization", "Bearer synthetic-source-sentinel");
                        then.status(200)
                            .header("Access-Control-Allow-Headers", "Authorization")
                            .body("artifact");
                    })
                    .await;
                let result = client
                    .fetch_artifact("sentinel", config.credential().unwrap(), None, None)
                    .await;
                if protected {
                    assert!(matches!(
                        result,
                        Err(turborepo_api_client::Error::SavedVercelCredentialDestination)
                    ));
                    received.assert_calls_async(0).await;
                } else {
                    assert!(result.unwrap().is_some());
                    received.assert_calls_async(2).await;
                }
                received.delete_async().await;
            }
        }
    }

    #[test]
    fn test_cli_login_url_source_overrides_turbo_json_source() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(
                r#"{
                    "remoteCache": {
                        "loginUrl": "https://attacker.test"
                    }
                }"#,
            )
            .unwrap();

        let config = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions {
                login_url: Some("https://cache.example.com".to_string()),
                ..Default::default()
            },
            global_config_path: None,
            environment: Some(HashMap::default()),
        }
        .build()
        .unwrap();

        assert_eq!(config.login_url(), "https://cache.example.com");
        assert_eq!(config.login_url_source(), Some(ConfigurationSource::Cli));
    }

    #[test]
    fn test_env_login_url_source_overrides_turbo_json_source() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();

        repo_root
            .join_component("turbo.json")
            .create_with_contents(
                r#"{
                    "remoteCache": {
                        "loginUrl": "https://attacker.test"
                    }
                }"#,
            )
            .unwrap();

        let mut env = HashMap::new();
        env.insert(
            OsString::from("turbo_login"),
            OsString::from("https://cache.example.com"),
        );

        let config = TurborepoConfigBuilder {
            repo_root,
            override_config: ConfigurationOptions::default(),
            global_config_path: None,
            environment: Some(env),
        }
        .build()
        .unwrap();

        assert_eq!(config.login_url(), "https://cache.example.com");
        assert_eq!(
            config.login_url_source(),
            Some(ConfigurationSource::Environment)
        );
    }
}
