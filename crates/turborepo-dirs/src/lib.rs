use std::path::PathBuf;

use dirs_next::config_dir as dirs_config_dir;

/// Returns the path to the user's configuration directory. This is a wrapper
/// around `dirs_next::config_dir` that also checks the `TURBO_CONFIG_DIR_PATH`
/// environment variable. If the environment variable is set, it will return
/// that path instead of `dirs_next::config_dir`.
pub fn config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("TURBO_CONFIG_DIR_PATH") {
        return Some(PathBuf::from(dir));
    }
    dirs_config_dir()
}

/// Returns the path to the user's configuration directory. This is a wrapper
/// around `dirs_next::config_dir` that also checks the `VERCEL_CONFIG_DIR_PATH`
/// environment variable. If the environment variable is set, it will return
/// that path instead of `dirs_next::config_dir`.
pub fn vercel_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VERCEL_CONFIG_DIR_PATH") {
        return Some(PathBuf::from(dir));
    }
    dirs_config_dir()
}
