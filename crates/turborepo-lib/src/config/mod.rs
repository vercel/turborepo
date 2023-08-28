mod turbo;
mod turbo_config;

use std::path::PathBuf;

use camino::Utf8PathBuf;
use config::ConfigError;
#[cfg(not(windows))]
use dirs_next::config_dir;
// Go's xdg implementation uses FOLDERID_LocalAppData for config home
// https://github.com/adrg/xdg/blob/master/paths_windows.go#L28
// Rust xdg implementations uses FOLDERID_RoamingAppData for config home
// We use cache_dir so we can find the config dir that the Go code uses
#[cfg(windows)]
use dirs_next::data_local_dir as config_dir;
use thiserror::Error;
pub use turbo::{
    validate_extends, validate_no_package_task_syntax, RawTurboJSON, SpacesJson, TurboJson,
};
pub use turbo_config::{ConfigurationOptions, TurborepoConfigBuilder};

#[derive(Debug, Error)]
pub enum Error {
    #[error("default config path not found")]
    NoDefaultConfigPath,
    #[error(transparent)]
    PackageJson(#[from] crate::package_json::Error),
    #[error(
        "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create \
         one"
    )]
    NoTurboJSON,
    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Camino(#[from] camino::FromPathBufError),
    #[error(
        "Package tasks (<package>#<task>) are not allowed in single-package repositories: found \
         {task_id}"
    )]
    PackageTaskInSinglePackageMode { task_id: String },
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
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
}

pub fn default_user_config_path() -> Result<Utf8PathBuf, Error> {
    Ok(Utf8PathBuf::try_from(
        config_dir()
            .map(|p| p.join("turborepo").join("config.json"))
            .ok_or(Error::NoDefaultConfigPath)?,
    )?)
}

#[allow(dead_code)]
pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|p| p.join("turborepo"))
}
