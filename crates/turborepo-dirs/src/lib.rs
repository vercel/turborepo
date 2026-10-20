//! Platform-specific directory utilities
//! A small patch on top of `dirs_next` that makes use of turbopath and respects
//! `VERCEL_CONFIG_DIR_PATH` as an override.

use std::path::PathBuf;

use dirs_next::config_dir as dirs_config_dir;
use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};

/// Returns the path to the user's configuration directory.
///
/// This is a wrapper around `dirs_next::config_dir` that also checks the
/// `TURBO_CONFIG_DIR_PATH` environment variable. If the environment variable
/// is set, it will return that path instead of `dirs_next::config_dir`.
pub fn config_dir() -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    config_dir_from_parts(
        std::env::var("TURBO_CONFIG_DIR_PATH").ok().as_deref(),
        dirs_config_dir(),
        std::env::current_dir,
    )
}

/// Returns the path to the user's configuration directory.
///
/// This is a wrapper around `dirs_next::config_dir` that also checks the
///  `VERCEL_CONFIG_DIR_PATH` environment variable. If the environment variable
/// is set, it will return that path instead of `dirs_next::config_dir`.
pub fn vercel_config_dir() -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    vercel_config_dir_from_parts(
        std::env::var("VERCEL_CONFIG_DIR_PATH").ok().as_deref(),
        dirs_config_dir(),
    )
}

fn config_dir_from_parts(
    override_dir: Option<&str>,
    default_config_dir: Option<PathBuf>,
    current_dir: impl FnOnce() -> Result<PathBuf, std::io::Error>,
) -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    if let Some(dir) = override_dir {
        // Reject empty strings per Unix convention
        if dir.is_empty() {
            return Err(PathError::InvalidUnicode(dir.to_string()));
        }

        let raw = PathBuf::from(dir);

        // Resolve to absolute path if necessary
        let abs = if raw.is_absolute() {
            raw
        } else {
            current_dir()?.join(raw)
        };

        let abs_str = abs
            .to_str()
            .ok_or_else(|| PathError::InvalidUnicode(dir.to_string()))?;

        return AbsoluteSystemPathBuf::new(abs_str).map(Some);
    }

    default_config_dir
        .map(AbsoluteSystemPathBuf::try_from)
        .transpose()
}

fn vercel_config_dir_from_parts(
    override_dir: Option<&str>,
    default_config_dir: Option<PathBuf>,
) -> Result<Option<AbsoluteSystemPathBuf>, PathError> {
    if let Some(dir) = override_dir {
        // Reject empty strings per Unix convention.
        if dir.is_empty() {
            return Err(PathError::InvalidUnicode(dir.to_string()));
        }

        return AbsoluteSystemPathBuf::new(dir).map(Some);
    }

    default_config_dir
        .map(AbsoluteSystemPathBuf::try_from)
        .transpose()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Config directory not found.")]
    ConfigDirNotFound,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn current_dir() -> Result<PathBuf, std::io::Error> {
        Ok(if cfg!(windows) {
            PathBuf::from("C:\\repo")
        } else {
            PathBuf::from("/repo")
        })
    }

    #[test]
    fn test_config_dir_with_env_var() {
        let test_path = if cfg!(windows) {
            "C:\\test\\config"
        } else {
            "/test/config"
        };

        let result = config_dir_from_parts(Some(test_path), None, current_dir);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert_eq!(path.unwrap().as_str(), test_path);
    }

    #[test]
    fn test_config_dir_with_relative_path() {
        let result = config_dir_from_parts(Some("relative/path"), None, current_dir);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        // Verify it was resolved to an absolute path
        assert!(path.unwrap().as_path().is_absolute());
    }

    #[test]
    fn test_config_dir_without_env_var() {
        let default_path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\test\\config")
        } else {
            PathBuf::from("/Users/test/config")
        };
        let result = config_dir_from_parts(None, Some(default_path), current_dir);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert!(path.unwrap().as_path().is_absolute());
    }

    #[test]
    fn test_vercel_config_dir_with_env_var() {
        let test_path = if cfg!(windows) {
            "C:\\vercel\\config"
        } else {
            "/vercel/config"
        };

        let result = vercel_config_dir_from_parts(Some(test_path), None);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert_eq!(path.unwrap().as_str(), test_path);
    }

    #[test]
    fn test_vercel_config_dir_with_invalid_env_var() {
        let result = vercel_config_dir_from_parts(Some("relative/path"), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_vercel_config_dir_without_env_var() {
        let default_path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\test\\config")
        } else {
            PathBuf::from("/Users/test/config")
        };
        let result = vercel_config_dir_from_parts(None, Some(default_path));
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert!(path.unwrap().as_path().is_absolute());
    }

    #[test]
    fn test_config_dir_empty_env_var() {
        let result = config_dir_from_parts(Some(""), None, current_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_vercel_config_dir_empty_env_var() {
        let result = vercel_config_dir_from_parts(Some(""), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_dir_and_vercel_config_dir_independence() {
        let turbo_path = if cfg!(windows) {
            "C:\\Users\\test\\config"
        } else {
            "/Users/test/config"
        };
        let default_vercel_path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\test\\vercel")
        } else {
            PathBuf::from("/Users/test/vercel")
        };

        let turbo_result = config_dir_from_parts(Some(turbo_path), None, current_dir);
        let vercel_result = vercel_config_dir_from_parts(None, Some(default_vercel_path));

        assert!(turbo_result.is_ok(), "turbo_result should be ok");
        let turbo_path_result = turbo_result.unwrap();
        assert!(turbo_path_result.is_some(), "turbo path should be some");
        assert_eq!(turbo_path_result.unwrap().as_str(), turbo_path);

        assert!(vercel_result.is_ok(), "vercel_result should be ok");
        let vercel_path = vercel_result.unwrap().expect("vercel path should be some");
        assert_ne!(vercel_path.as_str(), turbo_path);
    }

    #[test]
    fn test_vercel_config_dir_and_config_dir_independence() {
        let vercel_path = if cfg!(windows) {
            "C:\\Users\\test\\vercel"
        } else {
            "/Users/test/vercel"
        };
        let default_turbo_path = if cfg!(windows) {
            PathBuf::from("C:\\Users\\test\\config")
        } else {
            PathBuf::from("/Users/test/config")
        };

        let vercel_result = vercel_config_dir_from_parts(Some(vercel_path), None);
        let turbo_result = config_dir_from_parts(None, Some(default_turbo_path), current_dir);

        assert!(vercel_result.is_ok(), "vercel_result should be ok");
        let vercel_path_result = vercel_result.unwrap();
        assert!(vercel_path_result.is_some(), "vercel path should be some");
        assert_eq!(vercel_path_result.unwrap().as_str(), vercel_path);

        assert!(turbo_result.is_ok(), "turbo_result should be ok");
        let turbo_path = turbo_result.unwrap().expect("turbo path should be some");
        assert_ne!(turbo_path.as_str(), vercel_path);
    }

    #[test]
    fn test_error_display() {
        // Test that the Error enum formats correctly
        let error = Error::ConfigDirNotFound;
        assert_eq!(error.to_string(), "Config directory not found.");
    }

    #[test]
    fn test_error_debug() {
        // Test that the Error enum can be debug formatted
        let error = Error::ConfigDirNotFound;
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("ConfigDirNotFound"));
    }
}
