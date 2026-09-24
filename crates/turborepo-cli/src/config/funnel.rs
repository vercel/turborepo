use std::{collections::HashMap, ffi::OsString};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use super::{ConfigurationFileInputs, ConfigurationOptions, Error, TurborepoConfigBuilder};

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
    environment: HashMap<OsString, OsString>,
    file_inputs: ConfigurationFileInputs,
) -> Result<ConfigurationOptions, Error> {
    TurborepoConfigBuilder::new(repo_root)
        .with_override_config(overrides)
        .build_with_inputs(environment, file_inputs)
}

pub fn resolve_configuration_for_shim(
    repo_root: &AbsoluteSystemPath,
    root_turbo_json_path: Option<&AbsoluteSystemPathBuf>,
    environment: HashMap<OsString, OsString>,
    file_inputs: ConfigurationFileInputs,
) -> Result<ConfigurationOptions, Error> {
    resolve_configuration_with_overrides(
        repo_root,
        ConfigurationOptions {
            root_turbo_json_path: root_turbo_json_path.cloned(),
            ..Default::default()
        },
        environment,
        file_inputs,
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, ffi::OsString, fs};

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_types::LogOrder;

    use super::{
        resolve_configuration_for_shim, resolve_configuration_with_overrides,
        ConfigurationFileInputs,
    };
    use crate::config::{ConfigurationOptions, CONFIG_FILE};

    fn file_inputs(temp_dir: &TempDir) -> ConfigurationFileInputs {
        ConfigurationFileInputs {
            global_config_path: AbsoluteSystemPathBuf::try_from(
                temp_dir.path().join("global-config.json"),
            )
            .unwrap(),
            global_auth_path: AbsoluteSystemPathBuf::try_from(
                temp_dir.path().join("global-auth.json"),
            )
            .unwrap(),
            legacy_auth_path: None,
        }
    }

    fn environment() -> HashMap<OsString, OsString> {
        HashMap::new()
    }

    #[test]
    fn test_turbo_json_no_update_notifier_propagates_through_shim_config() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(r#"{"noUpdateNotifier": true}"#)
            .unwrap();

        let config =
            resolve_configuration_for_shim(&repo_root, None, environment(), file_inputs(&tmp_dir))
                .unwrap();
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
            environment(),
            file_inputs(&tmp_dir),
        )
        .unwrap();

        assert!(merged.no_update_notifier());
    }

    #[test]
    fn test_explicit_environment_overrides_config_files() {
        let tmp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(r#"{"concurrency": "1"}"#)
            .unwrap();
        fs::create_dir_all(repo_root.join_component(".turbo").as_std_path()).unwrap();
        repo_root
            .join_components(&[".turbo", "config.json"])
            .create_with_contents(r#"{"concurrency": "2"}"#)
            .unwrap();
        let environment =
            HashMap::from([(OsString::from("TURBO_CONCURRENCY"), OsString::from("3"))]);

        let config = resolve_configuration_with_overrides(
            &repo_root,
            ConfigurationOptions::default(),
            environment,
            file_inputs(&tmp_dir),
        )
        .unwrap();

        assert_eq!(config.concurrency.as_deref(), Some("3"));
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
            environment(),
            file_inputs(&tmp_dir),
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
            environment(),
            file_inputs(&tmp_dir),
        )
        .unwrap();

        assert!(!merged.force());
    }
}
