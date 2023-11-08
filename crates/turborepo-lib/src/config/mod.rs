mod turbo;
mod turbo_config;

use std::io;

use thiserror::Error;
pub use turbo::{
    validate_extends, validate_no_package_task_syntax, RawTurboJSON, SpacesJson, TurboJson,
};
pub use turbo_config::{ConfigurationOptions, TurborepoConfigBuilder};
use turbopath::AbsoluteSystemPathBuf;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Global config path not found")]
    NoGlobalConfigPath,
    #[error(transparent)]
    PackageJson(#[from] turborepo_repository::package_json::Error),
    #[error(
        "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create \
         one"
    )]
    NoTurboJSON,
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Camino(#[from] camino::FromPathBufError),
    #[error("Encountered an IO error while attempting to read {config_path}: {error}")]
    FailedToReadConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error("Encountered an IO error while attempting to set {config_path}: {error}")]
    FailedToSetConfig {
        config_path: AbsoluteSystemPathBuf,
        error: io::Error,
    },
    #[error(
        "Package tasks (<package>#<task>) are not allowed in single-package repositories: found \
         {task_id}"
    )]
    PackageTaskInSinglePackageMode { task_id: String },
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(
        "You specified \"{value}\" in the \"{key}\" key. You should not prefix your environment \
         variables with \"{env_pipeline_delimiter}\""
    )]
    InvalidEnvPrefix {
        value: String,
        key: String,
        env_pipeline_delimiter: &'static str,
    },
    #[error(transparent)]
    PathError(#[from] turbopath::PathError),
    #[error("\"{actual}\". Use \"{wanted}\" instead")]
    UnnecessaryPackageTaskSyntax { actual: String, wanted: String },
    #[error("You can only extend from the root workspace")]
    ExtendFromNonRoot,
    #[error("No \"extends\" key found")]
    NoExtends,
    #[error("Failed to create APIClient: {0}")]
    ApiClient(#[source] turborepo_api_client::Error),
    #[error("{0} is not UTF8.")]
    Encoding(String),
    #[error("TURBO_SIGNATURE should be either 1 or 0.")]
    InvalidSignature,
    #[error("TURBO_REMOTE_CACHE_ENABLED should be either 1 or 0.")]
    InvalidRemoteCacheEnabled,
    #[error("TURBO_REMOTE_CACHE_TIMEOUT: error parsing timeout.")]
    InvalidRemoteCacheTimeout(#[source] std::num::ParseIntError),
    #[error("TURBO_PREFLIGHT should be either 1 or 0.")]
    InvalidPreflight,
}
