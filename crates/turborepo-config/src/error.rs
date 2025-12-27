//! Error types for turborepo-config
//!
//! This module contains all error types used by the configuration system.

use std::io;

use convert_case::{Case, Casing};
use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_errors::TURBO_SITE;
use turborepo_repository::package_graph::PackageName;

/// Error for invalid environment variable prefix usage
#[derive(Debug, Error, Diagnostic)]
#[error("Environment variables should not be prefixed with \"{env_pipeline_delimiter}\"")]
#[diagnostic(
    code(invalid_env_prefix),
    url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab))
)]
pub struct InvalidEnvPrefixError {
    pub value: String,
    pub key: String,
    #[source_code]
    pub text: NamedSource<String>,
    #[label("variable with invalid prefix declared here")]
    pub span: Option<SourceSpan>,
    pub env_pipeline_delimiter: &'static str,
}

/// Error for unnecessary package task syntax usage
#[derive(Debug, Error, Diagnostic)]
#[diagnostic(
    code(unnecessary_package_task_syntax),
    url("{}/messages/{}", TURBO_SITE, self.code().unwrap().to_string().to_case(Case::Kebab))
)]
#[error("\"{actual}\". Use \"{wanted}\" instead.")]
pub struct UnnecessaryPackageTaskSyntaxError {
    pub actual: String,
    pub wanted: String,
    #[label("unnecessary package syntax found here")]
    pub span: Option<SourceSpan>,
    #[source_code]
    pub text: NamedSource<String>,
}

/// Main error enum for configuration operations
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
    // NOTE: TurboJsonParseError variant is defined in turborepo-lib's turbo_json::parser module.
    // When the turbo_json module is extracted to turborepo-config, this variant should be added:
    // #[error(transparent)]
    // #[diagnostic(transparent)]
    // TurboJsonParseError(#[from] crate::turbo_json::parser::Error),
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
