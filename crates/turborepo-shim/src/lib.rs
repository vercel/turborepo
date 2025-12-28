//! Shim for invoking the correct turbo binary version.
//!
//! This crate handles the logic for finding and executing the correct version
//! of turbo based on the repository configuration. It supports:
//!
//! - Finding locally installed turbo in node_modules
//! - Spawning the correct local turbo version as a child process
//! - Falling back to the global turbo when no local version is found
//! - Running update checks for new turbo versions
//!
//! The crate uses trait-based dependency injection to avoid circular
//! dependencies with `turborepo-lib`.

mod local_turbo_config;
mod local_turbo_state;
mod parser;
pub mod run;
mod turbo_state;

use std::sync::Arc;

// Re-exports
pub use local_turbo_state::turbo_version_has_shim;
pub use parser::ShimArgs;
pub use run::{run, run_with_args, Error, ShimResult, ShimRuntime, INVOCATION_DIR_ENV_VAR};
use shared_child::SharedChild;
pub use turbo_state::TurboState;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_repository::inference::RepoState;
use turborepo_ui::ColorConfig;

/// Trait for running the turbo CLI when shim determines current binary should
/// execute.
///
/// This trait is used by the shim to call back into the main turbo CLI
/// implementation when it determines that the currently running binary is the
/// correct one to use (either because it's the local installation, or because
/// no local installation was found).
///
/// # Example Implementation
///
/// ```ignore
/// struct TurboCliRunner;
///
/// impl TurboRunner for TurboCliRunner {
///     type Error = cli::Error;
///     
///     fn run(&self, repo_state: Option<RepoState>, ui: ColorConfig) -> Result<i32, Self::Error> {
///         cli::run(repo_state, ui)
///     }
/// }
/// ```
pub trait TurboRunner: Send + Sync {
    /// The error type returned by the runner.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Run the turbo CLI with the given repository state and UI configuration.
    ///
    /// # Arguments
    ///
    /// * `repo_state` - Optional repository state if inference succeeded
    /// * `ui` - Color configuration for terminal output
    ///
    /// # Returns
    ///
    /// The exit code from the CLI execution, or an error if execution failed.
    fn run(&self, repo_state: Option<RepoState>, ui: ColorConfig) -> Result<i32, Self::Error>;
}

/// Trait for spawning child processes with proper signal handling.
///
/// This trait abstracts over the process spawning mechanism, allowing the shim
/// to spawn child turbo processes while properly handling signals and cleanup.
///
/// The default implementation uses `shared_child` to spawn processes that can
/// be waited on from multiple threads and properly handle signals.
pub trait ChildSpawner: Send + Sync {
    /// Spawn a child process from the given command.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to spawn
    ///
    /// # Returns
    ///
    /// A shared handle to the spawned child process.
    fn spawn(&self, command: std::process::Command) -> std::io::Result<Arc<SharedChild>>;
}

/// Default implementation of `ChildSpawner` using `shared_child`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultChildSpawner;

impl ChildSpawner for DefaultChildSpawner {
    fn spawn(&self, mut command: std::process::Command) -> std::io::Result<Arc<SharedChild>> {
        SharedChild::spawn(&mut command).map(Arc::new)
    }
}

/// Trait for getting turbo configuration options.
///
/// This trait abstracts over the configuration loading mechanism, allowing
/// the shim to access configuration without directly depending on the full
/// configuration infrastructure in `turborepo-lib`.
pub trait ConfigProvider: Send + Sync {
    /// Get configuration options for the given repository root.
    ///
    /// # Arguments
    ///
    /// * `root` - The repository root path
    /// * `root_turbo_json` - Optional path to a custom root turbo.json
    ///
    /// # Returns
    ///
    /// Configuration options that can be used to check settings like
    /// `no_update_notifier`.
    fn get_config(
        &self,
        root: &AbsoluteSystemPath,
        root_turbo_json: Option<&AbsoluteSystemPathBuf>,
    ) -> ShimConfigurationOptions;
}

/// Minimal configuration options needed by the shim.
///
/// This struct contains only the configuration fields that the shim needs,
/// avoiding a dependency on the full `ConfigurationOptions` from
/// `turborepo-lib`.
#[derive(Debug, Default, Clone)]
pub struct ShimConfigurationOptions {
    no_update_notifier: Option<bool>,
}

impl ShimConfigurationOptions {
    /// Create a new configuration with the given options.
    pub fn new(no_update_notifier: Option<bool>) -> Self {
        Self { no_update_notifier }
    }

    /// Returns whether the update notifier should be disabled.
    ///
    /// Returns `true` if updates should not be checked, `false` otherwise.
    pub fn no_update_notifier(&self) -> bool {
        self.no_update_notifier.unwrap_or(false)
    }
}

/// Default configuration provider that returns default options.
///
/// This can be used when no configuration infrastructure is available,
/// or for testing purposes.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultConfigProvider;

impl ConfigProvider for DefaultConfigProvider {
    fn get_config(
        &self,
        _root: &AbsoluteSystemPath,
        _root_turbo_json: Option<&AbsoluteSystemPathBuf>,
    ) -> ShimConfigurationOptions {
        ShimConfigurationOptions::default()
    }
}

/// Trait for getting the current turbo version.
///
/// This is used to decouple version detection from the shim logic.
pub trait VersionProvider: Send + Sync {
    /// Get the version of the currently running turbo binary.
    fn get_version(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shim_configuration_options_default() {
        let opts = ShimConfigurationOptions::default();
        assert!(!opts.no_update_notifier());
    }

    #[test]
    fn test_shim_configuration_options_with_no_update_notifier() {
        let opts = ShimConfigurationOptions::new(Some(true));
        assert!(opts.no_update_notifier());

        let opts = ShimConfigurationOptions::new(Some(false));
        assert!(!opts.no_update_notifier());

        let opts = ShimConfigurationOptions::new(None);
        assert!(!opts.no_update_notifier());
    }

    #[test]
    fn test_default_config_provider() {
        let provider = DefaultConfigProvider;
        let fake_root =
            AbsoluteSystemPathBuf::new(if cfg!(windows) { "C:\\repo" } else { "/repo" }).unwrap();
        let config = provider.get_config(&fake_root, None);
        assert!(!config.no_update_notifier());
    }
}
