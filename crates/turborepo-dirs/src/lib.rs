//! Platform-specific directory utilities
//! A small patch on top of `dirs_next` that makes use of turbopath and respects
//! `VERCEL_CONFIG_DIR_PATH` as an override.

use dirs_next::config_dir as dirs_config_dir;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};

/// Returns the path to the user's configuration directory.
///
/// This is a wrapper around `dirs_next::config_dir` that also checks the
/// `TURBO_CONFIG_DIR_PATH` environment variable. If the environment variable
/// is set, it will return that path instead of `dirs_next::config_dir`.
pub fn config_dir() -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    if let Ok(dir) = std::env::var("TURBO_CONFIG_DIR_PATH") {
        return AbsoluteSystemPathBuf::new(dir).map(Some);
    }

    dirs_config_dir()
        .map(AbsoluteSystemPathBuf::try_from)
        .transpose()
}

/// Returns the path to the user's configuration directory.
///
/// This is a wrapper around `dirs_next::config_dir` that also checks the
///  `VERCEL_CONFIG_DIR_PATH` environment variable. If the environment variable
/// is set, it will return that path instead of `dirs_next::config_dir`.
pub fn vercel_config_dir() -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    if let Ok(dir) = std::env::var("VERCEL_CONFIG_DIR_PATH") {
        return AbsoluteSystemPathBuf::new(dir).map(Some);
    }

    dirs_config_dir()
        .map(AbsoluteSystemPathBuf::try_from)
        .transpose()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Config directory not found.")]
    ConfigDirNotFound,
}
