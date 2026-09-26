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
    use turborepo_types::{ConfigurationSource, EnvMode, LogOrder};

    use super::{
        ConfigurationFileInputs, resolve_configuration_for_shim,
        resolve_configuration_with_overrides,
    };
    use crate::config::{CONFIG_FILE, ConfigurationOptions};

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

    fn environment_with(values: &[(&str, &str)]) -> HashMap<OsString, OsString> {
        values
            .iter()
            .map(|(name, value)| (OsString::from(*name), OsString::from(*value)))
            .collect()
    }

    fn resolve_with_inputs(
        temp_dir: &TempDir,
        overrides: ConfigurationOptions,
        environment: HashMap<OsString, OsString>,
    ) -> ConfigurationOptions {
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        resolve_configuration_with_overrides(
            &repo_root,
            overrides,
            environment,
            file_inputs(temp_dir),
        )
        .unwrap()
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

    fn write_layered_config(temp_dir: &TempDir) {
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        repo_root
            .join_component(CONFIG_FILE)
            .create_with_contents(
                r#"{"daemon":true,"envMode":"strict","cacheDir":"turbo-json-cache","concurrency":"1"}"#,
            )
            .unwrap();
        let files = file_inputs(temp_dir);
        fs::write(
            files.global_config_path.as_std_path(),
            r#"{"apiUrl":"https://global.example","teamSlug":"global","timeout":20,"daemon":true,"envMode":"loose","scmBase":"global-base","scmHead":"global-head","cacheDir":"global-cache","concurrency":"2"}"#,
        )
        .unwrap();
        fs::create_dir_all(repo_root.join_component(".turbo").as_std_path()).unwrap();
        repo_root
            .join_components(&[".turbo", "config.json"])
            .create_with_contents(
                r#"{"apiUrl":"https://local.example","teamSlug":"local","timeout":30,"daemon":true,"envMode":"loose","scmBase":"local-base","scmHead":"local-head","cacheDir":"local-cache","concurrency":"3"}"#,
            )
            .unwrap();
    }

    #[test]
    fn test_local_config_overrides_global_and_turbo_json() {
        let temp_dir = TempDir::new().unwrap();
        write_layered_config(&temp_dir);

        let config = resolve_with_inputs(&temp_dir, ConfigurationOptions::default(), environment());

        assert_eq!(config.api_url(), "https://local.example");
        assert_eq!(config.team_slug(), Some("local"));
        assert_eq!(config.timeout(), 30);
        assert_eq!(config.concurrency.as_deref(), Some("3"));
    }

    #[test]
    fn test_environment_overrides_files_and_cli_overrides_environment() {
        let temp_dir = TempDir::new().unwrap();
        write_layered_config(&temp_dir);
        let environment = environment_with(&[
            ("TURBO_API", "https://env.example"),
            ("TURBO_TEAM", "env-team"),
            ("TURBO_REMOTE_CACHE_TIMEOUT", "40"),
            ("TURBO_DAEMON", "false"),
            ("TURBO_ENV_MODE", "strict"),
            ("TURBO_SCM_BASE", "env-base"),
            ("TURBO_SCM_HEAD", "env-head"),
            ("TURBO_CACHE_DIR", "env-cache"),
            ("TURBO_CONCURRENCY", "4"),
        ]);

        let from_environment = resolve_with_inputs(
            &temp_dir,
            ConfigurationOptions::default(),
            environment.clone(),
        );
        assert_eq!(from_environment.api_url(), "https://env.example");
        assert_eq!(
            from_environment.api_url_source(),
            Some(ConfigurationSource::Environment)
        );
        assert_eq!(from_environment.team_slug(), Some("env-team"));
        assert_eq!(from_environment.timeout(), 40);
        assert_eq!(from_environment.daemon, Some(false));
        assert_eq!(from_environment.env_mode(), EnvMode::Strict);
        assert_eq!(from_environment.scm_base(), Some("env-base"));
        assert_eq!(from_environment.scm_head(), Some("env-head"));
        assert_eq!(from_environment.cache_dir().as_str(), "env-cache");
        assert_eq!(from_environment.concurrency.as_deref(), Some("4"));

        let cli_overrides = ConfigurationOptions {
            api_url: Some("https://cli.example".to_string()),
            team_slug: Some("cli-team".to_string()),
            timeout: Some(50),
            daemon: Some(true),
            env_mode: Some(EnvMode::Loose),
            scm_base: Some("cli-base".to_string()),
            scm_head: Some("cli-head".to_string()),
            cache_dir: Some("cli-cache".into()),
            concurrency: Some("5".to_string()),
            ..Default::default()
        };
        let from_cli = resolve_with_inputs(&temp_dir, cli_overrides, environment);

        assert_eq!(from_cli.api_url(), "https://cli.example");
        assert_eq!(from_cli.api_url_source(), Some(ConfigurationSource::Cli));
        assert_eq!(from_cli.team_slug(), Some("cli-team"));
        assert_eq!(from_cli.timeout(), 50);
        assert_eq!(from_cli.daemon, Some(true));
        assert_eq!(from_cli.env_mode(), EnvMode::Loose);
        assert_eq!(from_cli.scm_base(), Some("cli-base"));
        assert_eq!(from_cli.scm_head(), Some("cli-head"));
        assert_eq!(from_cli.cache_dir().as_str(), "cli-cache");
        assert_eq!(from_cli.concurrency.as_deref(), Some("5"));
    }

    #[test]
    fn test_root_turbo_json_environment_path_is_overridden_by_cli_path() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path()).unwrap();
        let root_config = repo_root.join_component(CONFIG_FILE);
        root_config
            .create_with_contents(r#"{"concurrency":"1"}"#)
            .unwrap();
        let alternate_config = repo_root.join_component("turbo-alt.json");
        alternate_config
            .create_with_contents(r#"{"concurrency":"2"}"#)
            .unwrap();
        let environment = environment_with(&[("TURBO_ROOT_TURBO_JSON", alternate_config.as_str())]);

        let from_environment = resolve_with_inputs(
            &temp_dir,
            ConfigurationOptions::default(),
            environment.clone(),
        );
        assert_eq!(from_environment.concurrency.as_deref(), Some("2"));

        let cli_overrides = ConfigurationOptions {
            root_turbo_json_path: Some(root_config),
            ..Default::default()
        };
        let from_cli = resolve_with_inputs(&temp_dir, cli_overrides, environment);
        assert_eq!(from_cli.concurrency.as_deref(), Some("1"));
    }
}
