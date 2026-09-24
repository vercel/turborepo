use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_config::ExperimentalObservabilityOptions;

use super::Args;
use crate::config::{
    resolve_configuration_with_overrides, ConfigurationOptions, Error as ConfigError,
};

/// Converts CLI args into the top-precedence configuration override layer.
pub(crate) fn cli_overrides_from_args(args: &Args) -> Result<ConfigurationOptions, ConfigError> {
    Ok(ConfigurationOptions {
        api_url: args.api.clone(),
        login_url: args.login.clone(),
        team_slug: args.team.clone(),
        token: args.token.clone(),
        timeout: args.remote_cache_timeout,
        preflight: args.preflight.then_some(true),
        ui: args.ui.map(Into::into),
        allow_no_package_manager: args
            .dangerously_disable_package_manager_check
            .then_some(true),
        daemon: args.run_args().and_then(|run_args| run_args.daemon()),
        env_mode: args
            .execution_args()
            .and_then(|execution_args| execution_args.env_mode.map(Into::into)),
        cache_dir: args
            .execution_args()
            .and_then(|execution_args| execution_args.cache_dir.clone().map(Into::into)),
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
            .and_then(|execution_args| execution_args.log_order.map(Into::into)),
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
        experimental_observability: args
            .experimental_otel_args
            .to_config()
            .map(|otel| ExperimentalObservabilityOptions { otel: Some(otel) }),
        ..Default::default()
    })
}

pub(crate) fn resolve_configuration_from_args(
    repo_root: &AbsoluteSystemPath,
    args: &Args,
) -> Result<ConfigurationOptions, ConfigError> {
    let overrides = cli_overrides_from_args(args)?;
    let (environment, file_inputs) = crate::cli::configuration_inputs_from_process()?;
    resolve_configuration_with_overrides(repo_root, overrides, environment, file_inputs)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use turborepo_types::LogOrder;

    use super::{cli_overrides_from_args, Args};

    fn parse_args(args: &[&str]) -> Args {
        Args::parse_args(args.iter().map(OsString::from).collect()).unwrap()
    }

    #[test]
    fn test_cli_overrides_capture_no_update_notifier() {
        let args = parse_args(&["turbo", "--no-update-notifier", "run", "build"]);
        let overrides = cli_overrides_from_args(&args).unwrap();

        assert_eq!(overrides.no_update_notifier, Some(true));
    }

    #[test]
    fn test_cli_overrides_capture_log_order() {
        let args = parse_args(&["turbo", "run", "build", "--log-order", "stream"]);
        let overrides = cli_overrides_from_args(&args).unwrap();

        assert_eq!(overrides.log_order, Some(LogOrder::Stream));
    }

    #[test]
    fn test_cli_force_override_parses_optional_boolean() {
        for (args, expected) in [
            (vec!["turbo", "run", "build"], None),
            (vec!["turbo", "run", "build", "--force"], Some(true)),
            (vec!["turbo", "run", "build", "--force=true"], Some(true)),
            (vec!["turbo", "run", "build", "--force=false"], Some(false)),
        ] {
            let args = parse_args(&args);
            let overrides = cli_overrides_from_args(&args).unwrap();

            assert_eq!(overrides.force, expected);
        }
    }
}
