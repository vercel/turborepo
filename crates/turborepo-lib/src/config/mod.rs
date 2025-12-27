//! Configuration module - re-exports from turborepo-config
//!
//! This module provides a thin re-export layer over turborepo-config.
//! The `Error` type is now fully defined in turborepo-config with a
//! `TurboJsonParseError` variant that accepts any boxed diagnostic error.

// Re-export the resolve function
pub use turborepo_config::resolve_turbo_config_path;
// Re-export everything from turborepo-config
// Note: FutureFlags is NOT re-exported here because turborepo-lib has its own
// FutureFlags type in turbo_json::future_flags that is used throughout the crate.
// Note: EnvMode and LogOrder are re-exported from cli/mod.rs and turbo_json/mod.rs
pub use turborepo_config::{
    ConfigurationOptions, Error, InvalidEnvPrefixError, TurborepoConfigBuilder, UIMode,
    UnnecessaryPackageTaskSyntaxError, CONFIG_FILE, CONFIG_FILE_JSONC,
};

/// Extension trait to convert turbo_json::parser::Error into config::Error
impl From<crate::turbo_json::parser::Error> for Error {
    fn from(err: crate::turbo_json::parser::Error) -> Self {
        Error::TurboJsonParseError(Box::new(err))
    }
}

#[cfg(test)]
mod test {
    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPath;

    use super::{ConfigurationOptions, CONFIG_FILE, CONFIG_FILE_JSONC};
    #[allow(unused_imports)]
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
