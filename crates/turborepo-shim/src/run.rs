//! Shim run logic with trait-based dependency injection.
//!
//! This module contains the main entry point for the shim, refactored to use
//! injected traits instead of direct `crate::` imports from `turborepo-lib`.

use std::{env, process, process::Stdio, sync::Arc, time::Duration};

use camino::Utf8PathBuf;
use dunce::canonicalize as fs_canonicalize;
use miette::Diagnostic;
use shared_child::SharedChild;
use thiserror::Error;
use tiny_gradient::{GradientStr, RGB};
use tracing::{debug, warn};
use turbo_updater::{display_update_check, UpdateCheckConfig};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{
    inference::{RepoMode, RepoState},
    package_manager,
    package_manager::PackageManager,
};
use turborepo_ui::ColorConfig;
use which::which;

use crate::{
    local_turbo_config::LocalTurboConfig,
    local_turbo_state::{turbo_version_has_shim, LocalTurboState},
    parser::ShimArgs,
    ChildSpawner, ConfigProvider, ShimConfigurationOptions, TurboRunner, VersionProvider,
};

const TURBO_GLOBAL_WARNING_DISABLED: &str = "TURBO_GLOBAL_WARNING_DISABLED";

/// Environment variable name for the invocation directory.
/// This is set by the shim to communicate the original invocation directory
/// to the CLI when spawning local turbo or running the global turbo.
pub const INVOCATION_DIR_ENV_VAR: &str = "TURBO_INVOCATION_DIR";

/// Runtime container for injected shim dependencies.
///
/// This struct holds all the trait implementations needed by the shim to
/// execute. By using trait objects, we avoid circular dependencies with
/// `turborepo-lib`.
///
/// # Type Parameters
///
/// * `R` - Implementation of `TurboRunner` for running the CLI
/// * `C` - Implementation of `ConfigProvider` for loading configuration
/// * `S` - Implementation of `ChildSpawner` for spawning child processes
/// * `V` - Implementation of `VersionProvider` for getting the current version
pub struct ShimRuntime<R, C, S, V>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    /// The turbo runner implementation
    pub runner: R,
    /// The config provider implementation
    pub config_provider: C,
    /// The child spawner implementation
    pub child_spawner: S,
    /// The version provider implementation
    pub version_provider: V,
}

impl<R, C, S, V> ShimRuntime<R, C, S, V>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    /// Create a new ShimRuntime with the given implementations.
    pub fn new(runner: R, config_provider: C, child_spawner: S, version_provider: V) -> Self {
        Self {
            runner,
            config_provider,
            child_spawner,
            version_provider,
        }
    }
}

/// Errors that can occur during shim execution.
#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    /// Error from argument parsing
    #[error(transparent)]
    #[diagnostic(transparent)]
    Args(#[from] crate::parser::Error),

    /// Error from repository inference
    #[error(transparent)]
    Inference(#[from] turborepo_repository::inference::Error),

    /// Failed to execute local turbo process
    #[error("Failed to execute local `turbo` process.")]
    LocalTurboProcess(#[source] std::io::Error),

    /// Failed to resolve local turbo path
    #[error("Failed to resolve local `turbo` path: {0}")]
    LocalTurboPath(String),

    /// Failed to find npx
    #[error("Failed to find `npx`: {0}")]
    Which(#[from] which::Error),

    /// Failed to execute turbo via npx
    #[error("Failed to execute `turbo` via `npx`.")]
    NpxTurboProcess(#[source] std::io::Error),

    /// Failed to resolve repository root
    #[error("Failed to resolve repository root: {0}")]
    RepoRootPath(AbsoluteSystemPathBuf),

    /// Path error
    #[error(transparent)]
    Path(#[from] turbopath::PathError),

    /// CLI error from the runner - stored as a miette::Report to preserve the
    /// full diagnostic chain (related errors, source code, etc.)
    #[error("{0:?}")]
    Cli(miette::Report),
}

/// Main entry point for the shim with injected dependencies.
///
/// This function handles the complete shim logic including arg parsing:
/// 1. Normalizes environment variables
/// 2. Parses command-line arguments
/// 3. Sets up error reporting
/// 4. Determines whether to run local or global turbo
/// 5. Executes the appropriate turbo binary
///
/// # Arguments
///
/// * `runtime` - The runtime container with all injected dependencies
///
/// # Returns
///
/// The exit code from turbo execution, or an error if execution failed.
pub fn run<R, C, S, V>(runtime: &ShimRuntime<R, C, S, V>) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    normalize_config_dir_env_vars();
    let args = ShimArgs::parse()?;
    run_with_args(runtime, args)
}

/// Main entry point for the shim with injected dependencies and pre-parsed
/// args.
///
/// This function handles the complete shim logic:
/// 1. Sets up error reporting based on color configuration
/// 2. Determines whether to run local or global turbo
/// 3. Executes the appropriate turbo binary
///
/// # Arguments
///
/// * `runtime` - The runtime container with all injected dependencies
/// * `args` - Pre-parsed shim arguments
///
/// # Returns
///
/// The exit code from turbo execution, or an error if execution failed.
pub fn run_with_args<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    args: ShimArgs,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    let color_config = args.color_config();

    if color_config.should_strip_ansi {
        // Let's not crash just because we failed to set up the hook
        let _ = miette::set_hook(Box::new(|_| {
            Box::new(
                miette::MietteHandlerOpts::new()
                    .show_related_errors_as_nested()
                    .color(false)
                    .unicode(false)
                    .build(),
            )
        }));
    } else {
        let _ = miette::set_hook(Box::new(|_| {
            Box::new(
                miette::MietteHandlerOpts::new()
                    .show_related_errors_as_nested()
                    .build(),
            )
        }));
    }

    debug!(
        "Global turbo version: {}",
        runtime.version_provider.get_version()
    );

    // If skip_infer is passed, we're probably running local turbo with
    // global turbo having handled the inference. We can run without any
    // concerns.
    if args.skip_infer {
        return run_cli(runtime, None, color_config);
    }

    // If the TURBO_BINARY_PATH is set, we do inference but we do not use
    // it to execute local turbo. We simply use it to set the `--single-package`
    // and `--cwd` flags.
    if is_turbo_binary_path_set() {
        let repo_state = RepoState::infer(&args.cwd)?;
        debug!("Repository Root: {}", repo_state.root);
        return run_cli(runtime, Some(repo_state), color_config);
    }

    match RepoState::infer(&args.cwd) {
        Ok(repo_state) => {
            debug!("Repository Root: {}", repo_state.root);
            run_correct_turbo(runtime, repo_state, args, color_config)
        }
        Err(err) => {
            // If we cannot infer, we still run global turbo. This allows for global
            // commands like login/logout/link/unlink to still work
            debug!("Repository inference failed: {}", err);
            debug!("Running command as global turbo");
            run_cli(runtime, None, color_config)
        }
    }
}

/// Helper to run the CLI through the runtime's runner.
fn run_cli<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    repo_state: Option<RepoState>,
    ui: ColorConfig,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    runtime
        .runner
        .run(repo_state, ui)
        .map_err(|e| Error::Cli(miette::Report::new(e)))
}

/// Attempts to run correct turbo by finding nearest package.json,
/// then finding local turbo installation. If the current binary is the
/// local turbo installation, then we run current turbo. Otherwise we
/// kick over to the local turbo installation.
fn run_correct_turbo<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    repo_state: RepoState,
    shim_args: ShimArgs,
    ui: ColorConfig,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    let package_manager = repo_state.package_manager.as_ref();

    if let Some(turbo_state) = LocalTurboState::infer(&repo_state.root) {
        let config = runtime
            .config_provider
            .get_config(&repo_state.root, shim_args.root_turbo_json.as_ref());

        try_check_for_updates(&shim_args, turbo_state.version(), &config, package_manager);

        if turbo_state.local_is_self() {
            env::set_var(INVOCATION_DIR_ENV_VAR, shim_args.invocation_dir.as_path());
            debug!("Currently running turbo is local turbo.");
            run_cli(runtime, Some(repo_state), ui)
        } else {
            spawn_local_turbo(runtime, &repo_state, turbo_state, shim_args)
        }
    } else if let Some(local_config) = LocalTurboConfig::infer(&repo_state) {
        debug!(
            "Found configuration for turbo version {}",
            local_config.turbo_version()
        );
        spawn_npx_turbo(
            runtime,
            &repo_state,
            local_config.turbo_version(),
            shim_args,
        )
    } else {
        let version = runtime.version_provider.get_version();
        let config = runtime
            .config_provider
            .get_config(&repo_state.root, shim_args.root_turbo_json.as_ref());
        try_check_for_updates(&shim_args, version, &config, package_manager);

        // cli::run checks for this env var, rather than an arg, so that we can support
        // calling old versions without passing unknown flags.
        env::set_var(INVOCATION_DIR_ENV_VAR, shim_args.invocation_dir.as_path());
        debug!("Running command as global turbo");

        let should_warn_on_global = env::var(TURBO_GLOBAL_WARNING_DISABLED)
            .map_or(true, |disable| !matches!(disable.as_str(), "1" | "true"));

        let declared_version = repo_state
            .root_package_json
            .dependencies
            .as_ref()
            .and_then(|deps| deps.get("turbo"))
            .or_else(|| {
                repo_state
                    .root_package_json
                    .dev_dependencies
                    .as_ref()
                    .and_then(|deps| deps.get("turbo"))
            });

        if should_warn_on_global {
            if let Some(declared_version) = declared_version {
                warn!(
                    "No locally installed `turbo` found in your repository. Using globally \
                     installed version ({version}), which can cause unexpected \
                     behavior.\n\nInstalling the version in your repository ({declared_version}) \
                     before calling `turbo` will result in more predictable behavior across \
                     environments."
                );
            } else {
                warn!(
                    "No locally installed `turbo` found in your repository. Using globally \
                     installed version ({version}). Using a specified version in your repository \
                     will result in more predictable behavior."
                );
            }
        }
        run_cli(runtime, Some(repo_state), ui)
    }
}

fn spawn_local_turbo<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    repo_state: &RepoState,
    local_turbo_state: LocalTurboState,
    mut shim_args: ShimArgs,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    let local_turbo_path = fs_canonicalize(local_turbo_state.binary()).map_err(|_| {
        Error::LocalTurboPath(local_turbo_state.binary().to_string_lossy().to_string())
    })?;
    debug!(
        "Running local turbo binary in {}\n",
        local_turbo_path.display()
    );
    let cwd = fs_canonicalize(&repo_state.root)
        .map_err(|_| Error::RepoRootPath(repo_state.root.clone()))?;

    let raw_args = modify_args_for_local(&mut shim_args, repo_state, local_turbo_state.version());

    // We spawn a process that executes the local turbo
    // that we've found in node_modules/.bin/turbo.
    let mut command = process::Command::new(local_turbo_path);
    command
        .args(&raw_args)
        // rather than passing an argument that local turbo might not understand, set
        // an environment variable that can be optionally used
        .env(INVOCATION_DIR_ENV_VAR, shim_args.invocation_dir.as_path())
        .current_dir(cwd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    spawn_child_turbo(runtime, command, Error::LocalTurboProcess)
}

fn spawn_npx_turbo<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    repo_state: &RepoState,
    turbo_version: &str,
    mut shim_args: ShimArgs,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    debug!("Running turbo@{turbo_version} via npx");
    let npx_path = which("npx")?;
    let cwd = fs_canonicalize(&repo_state.root)
        .map_err(|_| Error::RepoRootPath(repo_state.root.clone()))?;

    let raw_args = modify_args_for_local(&mut shim_args, repo_state, turbo_version);

    let mut command = process::Command::new(npx_path);
    command.arg("-y");
    command.arg(format!("turbo@{turbo_version}"));

    // rather than passing an argument that local turbo might not understand, set
    // an environment variable that can be optionally used
    command
        .args(&raw_args)
        .env(INVOCATION_DIR_ENV_VAR, shim_args.invocation_dir.as_path())
        .current_dir(cwd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    spawn_child_turbo(runtime, command, Error::NpxTurboProcess)
}

fn modify_args_for_local(
    shim_args: &mut ShimArgs,
    repo_state: &RepoState,
    local_version: &str,
) -> Vec<String> {
    let supports_skip_infer_and_single_package = turbo_version_has_shim(local_version);
    let already_has_single_package_flag = shim_args
        .remaining_turbo_args
        .contains(&"--single-package".to_string());
    let should_add_single_package_flag = repo_state.mode == RepoMode::SinglePackage
        && !already_has_single_package_flag
        && supports_skip_infer_and_single_package;

    debug!(
        "supports_skip_infer_and_single_package {:?}",
        supports_skip_infer_and_single_package
    );

    let mut raw_args: Vec<_> = if supports_skip_infer_and_single_package {
        vec!["--skip-infer".to_string()]
    } else {
        Vec::new()
    };

    raw_args.append(&mut shim_args.remaining_turbo_args);

    // We add this flag after the raw args to avoid accidentally passing it
    // as a global flag instead of as a run flag.
    if should_add_single_package_flag {
        raw_args.push("--single-package".to_string());
    }

    raw_args.push("--".to_string());
    raw_args.append(&mut shim_args.forwarded_args);

    raw_args
}

fn spawn_child_turbo<R, C, S, V>(
    runtime: &ShimRuntime<R, C, S, V>,
    command: process::Command,
    err: fn(std::io::Error) -> Error,
) -> Result<i32, Error>
where
    R: TurboRunner,
    C: ConfigProvider,
    S: ChildSpawner,
    V: VersionProvider,
{
    let child: Arc<SharedChild> = runtime.child_spawner.spawn(command).map_err(err)?;

    let exit_status = child.wait().map_err(err)?;
    let exit_code = exit_status.code().unwrap_or_else(|| {
        debug!("child turbo failed to report exit code");
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let signal = exit_status.signal();
            let core_dumped = exit_status.core_dumped();
            debug!(
                "child turbo caught signal {:?}. Core dumped? {}",
                signal, core_dumped
            );
        }
        2
    });

    Ok(exit_code)
}

/// Checks for `TURBO_BINARY_PATH` variable. If it is set,
/// we do not try to find local turbo, we simply run the command as
/// the current binary. This is due to legacy behavior of `TURBO_BINARY_PATH`
/// that lets users dynamically set the path of the turbo binary. Because
/// that conflicts with finding a local turbo installation and
/// executing that binary, these two features are fundamentally incompatible.
fn is_turbo_binary_path_set() -> bool {
    env::var("TURBO_BINARY_PATH").is_ok()
}

fn normalize_config_dir_env_vars() {
    // Normalize relative config dir env vars to absolute paths early in CLI startup
    for var in ["TURBO_CONFIG_DIR_PATH", "VERCEL_CONFIG_DIR_PATH"] {
        if let Ok(val) = env::var(var) {
            match turbopath::AbsoluteSystemPathBuf::new(val.as_str()) {
                Ok(_) => {
                    // already absolute, nothing to do
                }
                Err(turbopath::PathError::NotAbsolute(_)) => {
                    match turbopath::AbsoluteSystemPathBuf::from_cwd(Utf8PathBuf::from(val)) {
                        Ok(abs) => env::set_var(var, abs.as_str()),
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

fn try_check_for_updates(
    args: &ShimArgs,
    current_version: &str,
    config: &ShimConfigurationOptions,
    package_manager: Result<&PackageManager, &package_manager::Error>,
) {
    let package_manager = package_manager.unwrap_or(&PackageManager::Npm);

    if args.should_check_for_update() {
        // custom footer for update message
        let footer = format!(
            "Follow {username} for updates: {url}",
            username = "@turborepo".gradient([RGB::new(0, 153, 247), RGB::new(241, 23, 18)]),
            url = "https://x.com/turborepo"
        );

        let interval = if args.force_update_check {
            // force update check
            Some(Duration::ZERO)
        } else {
            // use default (24 hours)
            None
        };
        // check for updates
        let _ = display_update_check(UpdateCheckConfig {
            package_name: "turbo",
            github_repo: "https://github.com/vercel/turborepo",
            footer: Some(&footer),
            current_version,
            // use default for timeout (800ms)
            timeout: None,
            interval,
            package_manager,
            config_no_update: config.no_update_notifier(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultChildSpawner;

    // Mock error that implements both Error and Diagnostic
    #[derive(Debug, thiserror::Error, Diagnostic)]
    #[error("mock error")]
    struct MockError;

    // Mock implementations for testing
    struct MockRunner;
    impl TurboRunner for MockRunner {
        type Error = MockError;
        fn run(
            &self,
            _repo_state: Option<RepoState>,
            _ui: ColorConfig,
        ) -> Result<i32, Self::Error> {
            Ok(0)
        }
    }

    struct MockConfigProvider;
    impl ConfigProvider for MockConfigProvider {
        fn get_config(
            &self,
            _root: &turbopath::AbsoluteSystemPath,
            _root_turbo_json: Option<&AbsoluteSystemPathBuf>,
        ) -> ShimConfigurationOptions {
            ShimConfigurationOptions::default()
        }
    }

    struct MockVersionProvider;
    impl VersionProvider for MockVersionProvider {
        fn get_version(&self) -> &'static str {
            "2.0.0"
        }
    }

    #[test]
    fn test_shim_runtime_creation() {
        let runtime = ShimRuntime::new(
            MockRunner,
            MockConfigProvider,
            DefaultChildSpawner,
            MockVersionProvider,
        );
        assert_eq!(runtime.version_provider.get_version(), "2.0.0");
    }

    #[test]
    fn test_is_turbo_binary_path_set() {
        // This tests the function when env var is not set
        std::env::remove_var("TURBO_BINARY_PATH");
        assert!(!is_turbo_binary_path_set());
    }
}
