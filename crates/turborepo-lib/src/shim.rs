//! Shim module for turborepo-lib.
//!
//! This module provides the integration between the `turborepo-shim` crate and
//! `turborepo-lib`. It implements the traits required by the shim and
//! re-exports types for backward compatibility.

use std::sync::Arc;

use miette::Diagnostic;
use shared_child::SharedChild;
use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::inference::RepoState;
// Re-export types from turborepo-shim for backward compatibility.
// These exports are used by other parts of turborepo-lib and external code.
#[allow(unused_imports)]
pub use turborepo_shim::{turbo_version_has_shim, ShimArgs, TurboState, INVOCATION_DIR_ENV_VAR};
use turborepo_shim::{
    ChildSpawner, ConfigProvider, ShimConfigurationOptions, ShimResult, ShimRuntime, TurboRunner,
    VersionProvider,
};
use turborepo_ui::ColorConfig;

use crate::{cli, get_version, tracing::TurboSubscriber};

/// Errors that can occur during shim execution.
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    /// Error from the shim runtime
    #[error(transparent)]
    #[diagnostic(transparent)]
    Shim(#[from] turborepo_shim::Error),

    /// Error from the CLI
    #[error(transparent)]
    #[diagnostic(transparent)]
    Cli(#[from] cli::Error),
}

/// Implementation of `TurboRunner` that calls into `turborepo-lib`'s CLI.
struct TurboCliRunner<'a> {
    subscriber: &'a TurboSubscriber,
}

impl<'a> TurboCliRunner<'a> {
    fn new(subscriber: &'a TurboSubscriber) -> Self {
        Self { subscriber }
    }
}

impl TurboRunner for TurboCliRunner<'_> {
    type Error = cli::Error;

    fn run(&self, repo_state: Option<RepoState>, ui: ColorConfig) -> Result<i32, Self::Error> {
        cli::run(repo_state, self.subscriber, ui)
    }
}

/// Implementation of `ConfigProvider` that uses `turborepo-lib`'s configuration
/// system.
struct TurboConfigProvider;

impl ConfigProvider for TurboConfigProvider {
    fn get_config(
        &self,
        root: &AbsoluteSystemPath,
        root_turbo_json: Option<&AbsoluteSystemPathBuf>,
    ) -> ShimConfigurationOptions {
        // When the full config pipeline succeeds, its value is authoritative —
        // higher-priority sources such as environment variables are already
        // folded in.  Only fall back to reading turbo.json directly when the
        // pipeline itself errors (e.g. malformed global config or auth files),
        // which can cause it to abort before reaching the turbo.json source.
        // Falling back unconditionally would allow turbo.json to override a
        // higher-priority source like `TURBO_NO_UPDATE_NOTIFIER=0`.
        let no_update_notifier =
            match crate::config::resolve_configuration_for_shim(root, root_turbo_json) {
                Ok(config) => config.no_update_notifier(),
                Err(e) => {
                    tracing::debug!("Failed to resolve configuration for shim: {e}");
                    read_no_update_notifier_from_turbo_json(root, root_turbo_json)
                }
            };

        ShimConfigurationOptions::new(Some(no_update_notifier))
    }
}

/// Reads `noUpdateNotifier` directly from turbo.json as a fallback.
///
/// Called only when the full configuration pipeline errors — for example,
/// a malformed global config or auth file can abort the pipeline before
/// turbo.json is ever processed. In that case we parse turbo.json in
/// isolation so the update-notifier flag is still honoured.
///
/// Because this bypasses the pipeline, higher-priority sources (env vars,
/// CLI flags) are not consulted. This is acceptable because the fallback
/// only fires when the pipeline is broken, and a wrong update-notification
/// preference is low-severity.
fn read_no_update_notifier_from_turbo_json(
    root: &AbsoluteSystemPath,
    root_turbo_json: Option<&AbsoluteSystemPathBuf>,
) -> bool {
    let turbo_json_path = root_turbo_json.cloned().or_else(|| {
        turborepo_config::resolve_turbo_config_path(root)
            .map_err(|e| {
                tracing::debug!("Failed to resolve turbo config path in fallback: {e}");
                e
            })
            .ok()
    });

    let Some(path) = turbo_json_path else {
        return false;
    };

    let contents = match path.read_existing_to_string() {
        Ok(Some(contents)) => contents,
        Ok(None) => return false,
        Err(e) => {
            tracing::debug!("Failed to read {path} for noUpdateNotifier fallback: {e}");
            return false;
        }
    };

    let file_path = path.file_name().unwrap_or("turbo.json");
    match turborepo_turbo_json::RawRootTurboJson::parse(&contents, file_path) {
        Ok(raw) => raw
            .no_update_notifier
            .map(|v| *v.as_inner())
            .unwrap_or(false),
        Err(e) => {
            tracing::debug!("Failed to parse {file_path} for noUpdateNotifier fallback: {e}");
            false
        }
    }
}

/// Implementation of `VersionProvider` that returns the current turbo version.
struct TurboVersionProvider;

impl VersionProvider for TurboVersionProvider {
    fn get_version(&self) -> &'static str {
        get_version()
    }
}

/// Implementation of `ChildSpawner` that uses `turborepo-lib`'s spawn_child
/// function.
struct TurboChildSpawner;

impl ChildSpawner for TurboChildSpawner {
    fn spawn(&self, command: std::process::Command) -> std::io::Result<Arc<SharedChild>> {
        crate::spawn_child(command)
    }
}

/// Normalize config directory environment variables.
///
/// This must be called early in the shim startup, before arg parsing,
/// to ensure that relative paths in TURBO_CONFIG_DIR_PATH and
/// VERCEL_CONFIG_DIR_PATH are resolved to absolute paths.
fn normalize_config_dir_env_vars() {
    use camino::Utf8PathBuf;
    // Normalize relative config dir env vars to absolute paths early in CLI startup
    for var in ["TURBO_CONFIG_DIR_PATH", "VERCEL_CONFIG_DIR_PATH"] {
        if let Ok(val) = std::env::var(var) {
            match turbopath::AbsoluteSystemPathBuf::new(val.as_str()) {
                Ok(_) => {
                    // already absolute, nothing to do
                }
                Err(turbopath::PathError::NotAbsolute(_)) => {
                    match turbopath::AbsoluteSystemPathBuf::from_cwd(Utf8PathBuf::from(val)) {
                        Ok(abs) => std::env::set_var(var, abs.as_str()),
                        Err(_) => {
                            // invalid value; leave as-is so downstream error
                            // handling can report it
                        }
                    }
                }
                Err(_) => {
                    // invalid value; leave as-is so downstream error handling
                    // can report it
                }
            }
        }
    }
}

/// Main entry point for the shim.
///
/// This function creates the runtime with all the necessary implementations
/// and calls into the `turborepo-shim` crate to execute the appropriate turbo
/// binary.
///
/// The execution flow matches the original shim behavior exactly:
/// 1. Normalize config dir environment variables
/// 2. Parse command-line arguments
/// 3. Create TurboSubscriber with verbosity and color config
/// 4. Create runtime with trait implementations
/// 5. Execute shim logic (miette hook setup, repo inference, turbo execution)
pub fn run() -> Result<i32, Error> {
    // Normalize env vars first, before arg parsing (matches original behavior)
    normalize_config_dir_env_vars();

    // Parse args to get verbosity and color config for the subscriber
    let args = ShimArgs::parse().map_err(turborepo_shim::Error::from)?;
    let color_config = args.color_config();
    let subscriber = TurboSubscriber::new_with_verbosity(args.verbosity, &color_config);

    // Enable chrome tracing as early as possible so that repo inference,
    // config resolution, and CLI parsing are all captured in profiles.
    if let Some((ref file_path, include_args)) = args.profile_file_and_include_args() {
        let _ = subscriber.enable_chrome_tracing(file_path, include_args);
    }

    let _span = tracing::info_span!("shim_run").entered();

    // Create the runtime with all implementations
    let runtime = ShimRuntime::new(
        TurboCliRunner::new(&subscriber),
        TurboConfigProvider,
        TurboChildSpawner,
        TurboVersionProvider,
    );

    // Run the shim with pre-parsed args (avoids double parsing)
    match turborepo_shim::run_with_args(&runtime, args) {
        ShimResult::Ok(code) => Ok(code),
        ShimResult::ShimError(e) => Err(Error::Shim(e)),
        ShimResult::CliError(e) => Err(Error::Cli(e)),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::read_no_update_notifier_from_turbo_json;

    #[test]
    fn fallback_reads_true_from_turbo_json() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        root.join_component("turbo.json")
            .create_with_contents(r#"{"noUpdateNotifier": true}"#)
            .unwrap();

        assert!(read_no_update_notifier_from_turbo_json(&root, None));
    }

    #[test]
    fn fallback_reads_false_from_turbo_json() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        root.join_component("turbo.json")
            .create_with_contents(r#"{"noUpdateNotifier": false}"#)
            .unwrap();

        assert!(!read_no_update_notifier_from_turbo_json(&root, None));
    }

    #[test]
    fn fallback_defaults_false_when_field_absent() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        root.join_component("turbo.json")
            .create_with_contents(r#"{}"#)
            .unwrap();

        assert!(!read_no_update_notifier_from_turbo_json(&root, None));
    }

    #[test]
    fn fallback_returns_false_when_no_turbo_json() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();

        assert!(!read_no_update_notifier_from_turbo_json(&root, None));
    }

    #[test]
    fn fallback_returns_false_for_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        root.join_component("turbo.json")
            .create_with_contents("not valid json at all")
            .unwrap();

        assert!(!read_no_update_notifier_from_turbo_json(&root, None));
    }

    #[test]
    fn fallback_uses_custom_turbo_json_path() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let custom_path = root.join_component("custom-turbo.json");
        custom_path
            .create_with_contents(r#"{"noUpdateNotifier": true}"#)
            .unwrap();

        assert!(read_no_update_notifier_from_turbo_json(
            &root,
            Some(&custom_path)
        ));
    }

    #[test]
    fn fallback_returns_false_for_nonexistent_custom_path() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let nonexistent = root.join_component("does-not-exist.json");

        assert!(!read_no_update_notifier_from_turbo_json(
            &root,
            Some(&nonexistent)
        ));
    }

    #[test]
    fn fallback_reads_from_turbo_jsonc() {
        let tmp = TempDir::new().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        root.join_component("turbo.jsonc")
            .create_with_contents(
                r#"{
                    // Comments are allowed in jsonc
                    "noUpdateNotifier": true
                }"#,
            )
            .unwrap();

        assert!(read_no_update_notifier_from_turbo_json(&root, None));
    }
}
