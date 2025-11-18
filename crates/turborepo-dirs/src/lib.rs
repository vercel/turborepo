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
        // Reject empty strings per Unix convention
        if dir.is_empty() {
            return Err(PathError::InvalidUnicode(dir));
        }

        let raw = std::path::PathBuf::from(&dir);

        // Resolve to absolute path if necessary
        let abs = if raw.is_absolute() {
            raw
        } else {
            std::env::current_dir()?.join(raw)
        };

        let abs_str = abs.to_str().ok_or_else(|| PathError::InvalidUnicode(dir))?;

        return AbsoluteSystemPathBuf::new(abs_str).map(Some);
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
        // Reject empty strings per Unix convention.
        if dir.is_empty() {
            return Err(PathError::InvalidUnicode(dir));
        }

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

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn test_config_dir_with_env_var() {
        // Set TURBO_CONFIG_DIR_PATH to an absolute path
        let test_path = if cfg!(windows) {
            "C:\\test\\config"
        } else {
            "/test/config"
        };

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", test_path);
        }

        let result = config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert_eq!(path.unwrap().as_str(), test_path);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_config_dir_with_relative_path() {
        // Set TURBO_CONFIG_DIR_PATH to a relative path (should be resolved to absolute)
        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", "relative/path");
        }

        let result = config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        // Verify it was resolved to an absolute path
        assert!(path.unwrap().as_path().is_absolute());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_config_dir_without_env_var() {
        // Ensure TURBO_CONFIG_DIR_PATH is not set
        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }

        let result = config_dir();
        assert!(result.is_ok());
        // On most systems, config_dir should return Some path
        // We can't assert the exact path since it's platform-specific
        let path = result.unwrap();
        if let Some(p) = path {
            // Verify it's an absolute path
            assert!(p.as_path().is_absolute());
        }
    }

    #[test]
    fn test_vercel_config_dir_with_env_var() {
        // Set VERCEL_CONFIG_DIR_PATH to an absolute path
        let test_path = if cfg!(windows) {
            "C:\\vercel\\config"
        } else {
            "/vercel/config"
        };

        unsafe {
            env::set_var("VERCEL_CONFIG_DIR_PATH", test_path);
        }

        let result = vercel_config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.is_some());
        assert_eq!(path.unwrap().as_str(), test_path);

        unsafe {
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_vercel_config_dir_with_invalid_env_var() {
        // Set VERCEL_CONFIG_DIR_PATH to a relative path (invalid)
        unsafe {
            env::set_var("VERCEL_CONFIG_DIR_PATH", "relative/path");
        }

        let result = vercel_config_dir();
        assert!(result.is_err());

        unsafe {
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_vercel_config_dir_without_env_var() {
        // Ensure VERCEL_CONFIG_DIR_PATH is not set
        unsafe {
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }

        let result = vercel_config_dir();
        assert!(result.is_ok());
        // On most systems, config_dir should return Some path
        // We can't assert the exact path since it's platform-specific
        let path = result.unwrap();
        if let Some(p) = path {
            // Verify it's an absolute path
            assert!(p.as_path().is_absolute());
        }
    }

    #[test]
    fn test_config_dir_empty_env_var() {
        // Set TURBO_CONFIG_DIR_PATH to empty string
        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", "");
        }

        let result = config_dir();
        // Empty string should be invalid as it's not an absolute path
        assert!(result.is_err());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_vercel_config_dir_empty_env_var() {
        // Set VERCEL_CONFIG_DIR_PATH to empty string
        unsafe {
            env::set_var("VERCEL_CONFIG_DIR_PATH", "");
        }

        let result = vercel_config_dir();
        // Empty string should be invalid as it's not an absolute path
        assert!(result.is_err());

        unsafe {
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_config_dir_and_vercel_config_dir_independence() {
        // Test that TURBO_CONFIG_DIR_PATH doesn't affect vercel_config_dir
        // Use a path that would be created by dirs_config_dir to ensure both succeed
        let turbo_path = if cfg!(windows) {
            "C:\\Users\\test\\config"
        } else {
            "/Users/test/config"
        };

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_path);
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }

        let turbo_result = config_dir();
        let vercel_result = vercel_config_dir();

        assert!(turbo_result.is_ok(), "turbo_result should be ok");
        let turbo_path_result = turbo_result.unwrap();
        assert!(turbo_path_result.is_some(), "turbo path should be some");
        assert_eq!(turbo_path_result.unwrap().as_str(), turbo_path);

        assert!(vercel_result.is_ok(), "vercel_result should be ok");
        // vercel_config_dir should return the default, not turbo_path
        if let Some(vercel_path) = vercel_result.unwrap() {
            assert_ne!(vercel_path.as_str(), turbo_path);
        }

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_vercel_config_dir_and_config_dir_independence() {
        // Test that VERCEL_CONFIG_DIR_PATH doesn't affect config_dir
        let vercel_path = if cfg!(windows) {
            "C:\\Users\\test\\vercel"
        } else {
            "/Users/test/vercel"
        };

        unsafe {
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_path);
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }

        let vercel_result = vercel_config_dir();
        let turbo_result = config_dir();

        assert!(vercel_result.is_ok(), "vercel_result should be ok");
        let vercel_path_result = vercel_result.unwrap();
        assert!(vercel_path_result.is_some(), "vercel path should be some");
        assert_eq!(vercel_path_result.unwrap().as_str(), vercel_path);

        assert!(turbo_result.is_ok(), "turbo_result should be ok");
        // config_dir should return the default, not vercel_path
        if let Some(turbo_path) = turbo_result.unwrap() {
            assert_ne!(turbo_path.as_str(), vercel_path);
        }

        unsafe {
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
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
