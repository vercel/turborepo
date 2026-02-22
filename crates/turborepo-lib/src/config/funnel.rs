use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use super::{ConfigurationOptions, Error, TurborepoConfigBuilder};
use crate::Args;

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

/// Converts CLI args into the top-precedence configuration override layer.
pub fn cli_overrides_from_args(args: &Args) -> Result<ConfigurationOptions, Error> {
    Ok(ConfigurationOptions {
        api_url: args.api.clone(),
        login_url: args.login.clone(),
        team_slug: args.team.clone(),
        token: args.token.clone(),
        timeout: args.remote_cache_timeout,
        preflight: args.preflight.then_some(true),
        ui: args.ui,
        allow_no_package_manager: args
            .dangerously_disable_package_manager_check
            .then_some(true),
        daemon: args.run_args().and_then(|run_args| run_args.daemon()),
        env_mode: args
            .execution_args()
            .and_then(|execution_args| execution_args.env_mode),
        cache_dir: args
            .execution_args()
            .and_then(|execution_args| execution_args.cache_dir.clone()),
        root_turbo_json_path: args
            .root_turbo_json
            .clone()
            .map(AbsoluteSystemPathBuf::from_cwd)
            .transpose()?,
        force: args
            .run_args()
            .and_then(|run_args| run_args.force.map(|value| value.unwrap_or(true))),
        log_order: args
            .execution_args()
            .and_then(|execution_args| execution_args.log_order),
        remote_only: args.run_args().and_then(|run_args| run_args.remote_only()),
        remote_cache_read_only: args
            .run_args()
            .and_then(|run_args| run_args.remote_cache_read_only()),
        cache: args
            .run_args()
            .and_then(|run_args| run_args.cache.as_deref())
            .map(|cache| cache.parse())
            .transpose()?,
        run_summary: args.run_args().and_then(|run_args| run_args.summarize()),
        allow_no_turbo_json: args.allow_no_turbo_json.then_some(true),
        concurrency: args
            .execution_args()
            .and_then(|execution_args| execution_args.concurrency.clone()),
        no_update_notifier: args.no_update_notifier.then_some(true),
        ..Default::default()
    })
}

pub fn resolve_configuration_with_overrides(
    repo_root: &AbsoluteSystemPath,
    overrides: ConfigurationOptions,
) -> Result<ConfigurationOptions, Error> {
    TurborepoConfigBuilder::new(repo_root)
        .with_override_config(overrides)
        .build()
}

pub fn resolve_configuration_from_args(
    repo_root: &AbsoluteSystemPath,
    args: &Args,
) -> Result<ConfigurationOptions, Error> {
    let overrides = cli_overrides_from_args(args)?;
    resolve_configuration_with_overrides(repo_root, overrides)
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
    use std::ffi::OsString;

    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{cli_overrides_from_args, resolve_configuration_with_overrides};
    use crate::{
        cli::Args,
        config::{ConfigurationOptions, CONFIG_FILE},
    };

    fn parse_args(args: &[&str]) -> Args {
        Args::parse(args.iter().map(OsString::from).collect()).unwrap()
    }

    #[test]
    fn test_cli_overrides_capture_no_update_notifier() {
        let args = parse_args(&["turbo", "--no-update-notifier", "run", "build"]);
        let overrides = cli_overrides_from_args(&args).unwrap();

        assert_eq!(overrides.no_update_notifier, Some(true));
    }

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
}
