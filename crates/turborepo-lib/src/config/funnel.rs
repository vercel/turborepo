use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use super::{ConfigurationOptions, Error, TurborepoConfigBuilder};

/// Ordered from lowest to highest precedence.
pub const CONFIGURATION_PRECEDENCE: &[ConfigurationSource] = &[
    ConfigurationSource::TurboJson,
    ConfigurationSource::GlobalConfig,
    ConfigurationSource::GlobalAuth,
    ConfigurationSource::LocalConfig,
    ConfigurationSource::OverrideEnvironment,
    ConfigurationSource::Environment,
    ConfigurationSource::Cli,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigurationSource {
    TurboJson,
    GlobalConfig,
    GlobalAuth,
    LocalConfig,
    Environment,
    OverrideEnvironment,
    Cli,
}

pub fn resolve_configuration_with_overrides(
    repo_root: &AbsoluteSystemPath,
    overrides: ConfigurationOptions,
) -> Result<ConfigurationOptions, Error> {
    TurborepoConfigBuilder::new(repo_root)
        .with_override_config(overrides)
        .build()
}

pub fn resolve_configuration_for_shim(
    repo_root: &AbsoluteSystemPath,
    root_turbo_json_path: Option<&AbsoluteSystemPathBuf>,
) -> Result<ConfigurationOptions, Error> {
    resolve_configuration_with_overrides(
        repo_root,
        ConfigurationOptions {
            root_turbo_json_path: root_turbo_json_path.cloned(),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_types::LogOrder;

    use super::resolve_configuration_with_overrides;
    use crate::config::{ConfigurationOptions, CONFIG_FILE};

    #[test]
    fn test_turbo_json_no_update_notifier_propagates_through_shim_config() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(r#"{"noUpdateNotifier": true}"#)
            .unwrap();

        let config = super::resolve_configuration_for_shim(&repo_root, None).unwrap();
        assert!(
            config.no_update_notifier(),
            "noUpdateNotifier from turbo.json should propagate through \
             resolve_configuration_for_shim"
        );
    }

    #[test]
    fn test_cli_overrides_are_highest_precedence() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(r#"{"noUpdateNotifier": false}"#)
            .unwrap();

        let merged = resolve_configuration_with_overrides(
            &repo_root,
            ConfigurationOptions {
                no_update_notifier: Some(true),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(merged.no_update_notifier());
    }

    #[test]
    fn test_cli_log_order_overrides_local_config() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        fs::create_dir_all(repo_root.join_component(".turbo").as_std_path()).unwrap();
        repo_root
            .join_components(&[".turbo", "config.json"])
            .create_with_contents(r#"{"logOrder": "grouped"}"#)
            .unwrap();

        let merged = resolve_configuration_with_overrides(
            &repo_root,
            ConfigurationOptions {
                log_order: Some(LogOrder::Stream),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(merged.log_order(), LogOrder::Stream);
    }

    #[test]
    fn test_cli_force_overrides_lower_precedence_config() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        fs::create_dir_all(repo_root.join_component(".turbo").as_std_path()).unwrap();
        repo_root
            .join_components(&[".turbo", "config.json"])
            .create_with_contents(r#"{"force": true}"#)
            .unwrap();

        let merged = resolve_configuration_with_overrides(
            &repo_root,
            ConfigurationOptions {
                force: Some(false),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(!merged.force());
    }
}
