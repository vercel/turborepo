//! Configuration module - re-exports from turborepo-config with turborepo-lib
//! specific additions
//!
//! This module provides a thin re-export layer over turborepo-config, adding
//! only the TurboJsonParseError variant which depends on turborepo-lib's
//! turbo_json module.

use std::io;

use convert_case::{Case, Casing};
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
// Re-export the resolve function
pub use turborepo_config::resolve_turbo_config_path;
// Re-export everything from turborepo-config
// Note: FutureFlags is NOT re-exported here because turborepo-lib has its own
// FutureFlags type in turbo_json::future_flags that is used throughout the crate.
// Note: EnvMode and LogOrder are re-exported from cli/mod.rs and turbo_json/mod.rs
pub use turborepo_config::{
    ConfigurationOptions, InvalidEnvPrefixError, TurborepoConfigBuilder, UIMode,
    UnnecessaryPackageTaskSyntaxError, CONFIG_FILE, CONFIG_FILE_JSONC,
};
use turborepo_errors::TURBO_SITE;
use turborepo_repository::package_graph::PackageName;

/// Configuration error type that mirrors turborepo_config::Error and adds
/// TurboJsonParseError which depends on turborepo-lib's turbo_json module.
///
/// This is a complete replica of turborepo_config::Error plus the
/// TurboJsonParseError variant. We need this because:
/// 1. turborepo-lib code uses pattern matching on Error variants (e.g.,
///    `config::Error::NoTurboJSON`)
/// 2. TurboJsonParseError depends on types in turborepo-lib's turbo_json module
/// 3. We can't add TurboJsonParseError to turborepo-config without creating a
///    circular dependency
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("Authentication error: {0}")]
    Auth(#[from] turborepo_auth::Error),
    #[error("Global config path not found.")]
    NoGlobalConfigPath,
    #[error("Global authentication file path not found.")]
    NoGlobalAuthFilePath,
    #[error("Global config directory not found.")]
    NoGlobalConfigDir,
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(
        "Could not find turbo.json or turbo.jsonc.\nFollow directions at https://turborepo.com/docs \
         to create one."
    )]
    NoTurboJSON,
    #[error(
        "Found both turbo.json and turbo.jsonc in the same directory: {directory}\nRemove either \
         turbo.json or turbo.jsonc so there is only one."
    )]
    MultipleTurboConfigs { directory: String },
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Camino(#[from] camino::FromPathBufError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
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
    #[error(
        "Package tasks (<package>#<task>) are not allowed in single-package repositories: found \
         {task_id}"
    )]
    #[diagnostic(code(package_task_in_single_package_mode), url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab)))]
    PackageTaskInSinglePackageMode {
        task_id: String,
        #[source_code]
        text: NamedSource<String>,
        #[label("package task found here")]
        span: Option<SourceSpan>,
    },
    #[error("Interruptible tasks must be persistent.")]
    InterruptibleButNotPersistent {
        #[source_code]
        text: NamedSource<String>,
        #[label("`interruptible` set here")]
        span: Option<SourceSpan>,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidEnvPrefix(Box<InvalidEnvPrefixError>),
    #[error(transparent)]
    PathError(#[from] turbopath::PathError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    UnnecessaryPackageTaskSyntax(Box<UnnecessaryPackageTaskSyntaxError>),
    #[error("You must extend from the root of the workspace first.")]
    ExtendsRootFirst {
        #[label("'//' should be first")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(
        "The \"extends\" key on task \"{task_name}\" can only be used in Package Configurations."
    )]
    TaskExtendsInRoot {
        task_name: String,
        #[label("\"extends\" found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(
        "Cannot set \"extends\": false on task \"{task_name}\" because it is not defined in the \
         extends chain."
    )]
    #[diagnostic(help("{extends_chain}"))]
    TaskNotInExtendsChain {
        task_name: String,
        extends_chain: String,
        #[label("task is not inherited")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("`{field}` cannot contain an environment variable.")]
    InvalidDependsOnValue {
        field: &'static str,
        #[label("environment variable found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("`{field}` cannot contain an absolute path.")]
    AbsolutePathInConfig {
        field: &'static str,
        #[label("absolute path found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("No \"extends\" key found.")]
    NoExtends {
        #[label("add extends key here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Tasks cannot be marked as interactive and cacheable.")]
    InteractiveNoCacheable {
        #[label("marked interactive here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Found `pipeline` field instead of `tasks`.")]
    #[diagnostic(help("Changed in 2.0: `pipeline` has been renamed to `tasks`."))]
    PipelineField {
        #[label("Rename `pipeline` field to `tasks`")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Failed to create APIClient: {0}")]
    ApiClient(#[source] turborepo_api_client::Error),
    #[error("{0} is not UTF8.")]
    Encoding(String),
    #[error("TURBO_SIGNATURE should be either 1 or 0.")]
    InvalidSignature,
    #[error("TURBO_REMOTE_CACHE_ENABLED should be either 1 or 0.")]
    InvalidRemoteCacheEnabled,
    #[error("TURBO_REMOTE_CACHE_TIMEOUT: Error parsing timeout.")]
    InvalidRemoteCacheTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_REMOTE_CACHE_UPLOAD_TIMEOUT: Error parsing timeout.")]
    InvalidUploadTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_PREFLIGHT should be either 1 or 0.")]
    InvalidPreflight,
    #[error("TURBO_LOG_ORDER should be one of: {0}")]
    InvalidLogOrder(String),
    #[error(transparent)]
    #[diagnostic(transparent)]
    TurboJsonParseError(#[from] crate::turbo_json::parser::Error),
    #[error("found absolute path in `cacheDir`")]
    #[diagnostic(help("If absolute paths are required, use `--cache-dir` or `TURBO_CACHE_DIR`."))]
    AbsoluteCacheDir {
        #[label("Make `cacheDir` value a relative unix path.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("Cannot load turbo.json for {0} in single package mode.")]
    InvalidTurboJsonLoad(PackageName),
    #[error("\"$TURBO_ROOT$\" must be used at the start of glob.")]
    InvalidTurboRootUse {
        #[label("\"$TURBO_ROOT$\" must be used at the start of glob.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("\"$TURBO_ROOT$\" must be followed by a '/'.")]
    InvalidTurboRootNeedsSlash {
        #[label("\"$TURBO_ROOT$\" must be followed by a '/'.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error("`with` cannot use dependency relationships.")]
    InvalidTaskWith {
        #[label("Remove `^` from start of task name.")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(
        "The \"futureFlags\" key can only be used in the root turbo.json. Please remove it from \
         Package Configurations."
    )]
    FutureFlagsInPackage {
        #[label("futureFlags key found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource<String>,
    },
    #[error(
        "TURBO_TUI_SCROLLBACK_LENGTH: Invalid value. Use a number for how many lines to keep in \
         scrollback."
    )]
    InvalidTuiScrollbackLength(#[source] std::num::ParseIntError),
    #[error("TURBO_SSO_LOGIN_CALLBACK_PORT: Invalid value. Use a number for the callback port.")]
    InvalidSsoLoginCallbackPort(#[source] std::num::ParseIntError),
}

// Conversion from turborepo_config::Error to this Error type
impl From<turborepo_config::Error> for Error {
    fn from(err: turborepo_config::Error) -> Self {
        match err {
            turborepo_config::Error::Auth(e) => Error::Auth(e),
            turborepo_config::Error::NoGlobalConfigPath => Error::NoGlobalConfigPath,
            turborepo_config::Error::NoGlobalAuthFilePath => Error::NoGlobalAuthFilePath,
            turborepo_config::Error::NoGlobalConfigDir => Error::NoGlobalConfigDir,
            turborepo_config::Error::PackageJson(e) => Error::PackageJson(e),
            turborepo_config::Error::NoTurboJSON => Error::NoTurboJSON,
            turborepo_config::Error::MultipleTurboConfigs { directory } => {
                Error::MultipleTurboConfigs { directory }
            }
            turborepo_config::Error::SerdeJson(e) => Error::SerdeJson(e),
            turborepo_config::Error::Io(e) => Error::Io(e),
            turborepo_config::Error::Camino(e) => Error::Camino(e),
            turborepo_config::Error::Reqwest(e) => Error::Reqwest(e),
            turborepo_config::Error::FailedToReadConfig { config_path, error } => {
                Error::FailedToReadConfig { config_path, error }
            }
            turborepo_config::Error::FailedToSetConfig { config_path, error } => {
                Error::FailedToSetConfig { config_path, error }
            }
            turborepo_config::Error::Cache(e) => Error::Cache(e),
            turborepo_config::Error::PackageTaskInSinglePackageMode {
                task_id,
                text,
                span,
            } => Error::PackageTaskInSinglePackageMode {
                task_id,
                text,
                span,
            },
            turborepo_config::Error::InterruptibleButNotPersistent { text, span } => {
                Error::InterruptibleButNotPersistent { text, span }
            }
            turborepo_config::Error::InvalidEnvPrefix(e) => Error::InvalidEnvPrefix(e),
            turborepo_config::Error::PathError(e) => Error::PathError(e),
            turborepo_config::Error::UnnecessaryPackageTaskSyntax(e) => {
                Error::UnnecessaryPackageTaskSyntax(e)
            }
            turborepo_config::Error::ExtendsRootFirst { span, text } => {
                Error::ExtendsRootFirst { span, text }
            }
            turborepo_config::Error::TaskExtendsInRoot {
                task_name,
                span,
                text,
            } => Error::TaskExtendsInRoot {
                task_name,
                span,
                text,
            },
            turborepo_config::Error::TaskNotInExtendsChain {
                task_name,
                extends_chain,
                span,
                text,
            } => Error::TaskNotInExtendsChain {
                task_name,
                extends_chain,
                span,
                text,
            },
            turborepo_config::Error::InvalidDependsOnValue { field, span, text } => {
                Error::InvalidDependsOnValue { field, span, text }
            }
            turborepo_config::Error::AbsolutePathInConfig { field, span, text } => {
                Error::AbsolutePathInConfig { field, span, text }
            }
            turborepo_config::Error::NoExtends { span, text } => Error::NoExtends { span, text },
            turborepo_config::Error::InteractiveNoCacheable { span, text } => {
                Error::InteractiveNoCacheable { span, text }
            }
            turborepo_config::Error::PipelineField { span, text } => {
                Error::PipelineField { span, text }
            }
            turborepo_config::Error::ApiClient(e) => Error::ApiClient(e),
            turborepo_config::Error::Encoding(s) => Error::Encoding(s),
            turborepo_config::Error::InvalidSignature => Error::InvalidSignature,
            turborepo_config::Error::InvalidRemoteCacheEnabled => Error::InvalidRemoteCacheEnabled,
            turborepo_config::Error::InvalidRemoteCacheTimeout(e) => {
                Error::InvalidRemoteCacheTimeout(e)
            }
            turborepo_config::Error::InvalidUploadTimeout(e) => Error::InvalidUploadTimeout(e),
            turborepo_config::Error::InvalidPreflight => Error::InvalidPreflight,
            turborepo_config::Error::InvalidLogOrder(s) => Error::InvalidLogOrder(s),
            turborepo_config::Error::AbsoluteCacheDir { span, text } => {
                Error::AbsoluteCacheDir { span, text }
            }
            turborepo_config::Error::InvalidTurboJsonLoad(p) => Error::InvalidTurboJsonLoad(p),
            turborepo_config::Error::InvalidTurboRootUse { span, text } => {
                Error::InvalidTurboRootUse { span, text }
            }
            turborepo_config::Error::InvalidTurboRootNeedsSlash { span, text } => {
                Error::InvalidTurboRootNeedsSlash { span, text }
            }
            turborepo_config::Error::InvalidTaskWith { span, text } => {
                Error::InvalidTaskWith { span, text }
            }
            turborepo_config::Error::FutureFlagsInPackage { span, text } => {
                Error::FutureFlagsInPackage { span, text }
            }
            turborepo_config::Error::InvalidTuiScrollbackLength(e) => {
                Error::InvalidTuiScrollbackLength(e)
            }
            turborepo_config::Error::InvalidSsoLoginCallbackPort(e) => {
                Error::InvalidSsoLoginCallbackPort(e)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::{ConfigurationOptions, CONFIG_FILE, CONFIG_FILE_JSONC};
    use crate::config::resolve_turbo_config_path;

    const DEFAULT_API_URL: &str = "https://vercel.com/api";
    const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
    const DEFAULT_TIMEOUT: u64 = 30;

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
}
