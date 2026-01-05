//! Task execution for Turborepo
//!
//! This crate provides the task execution infrastructure for Turborepo.
//!
//! # Architecture
//!
//! The executor is designed to be decoupled from the rest of turborepo-lib
//! through trait abstractions:
//!
//! - [`MfeConfigProvider`]: Abstraction for microfrontends configuration
//! - [`TaskAccessProvider`]: Abstraction for task access tracing
//! - [`HashTrackerProvider`]: Abstraction for hash tracking
//! - [`TaskErrorCollector`]: Abstraction for error collection
//! - [`TaskWarningCollector`]: Abstraction for warning collection
//!
//! The main execution logic is in [`TaskExecutor`], which handles:
//! - Cache checking and restoration
//! - Process spawning and output handling
//! - Cache saving on success
//! - Error and warning collection

mod command;
mod exec;
mod output;
mod visitor;

pub use command::{
    CommandFactory, CommandProvider, CommandProviderError, MicroFrontendProxyProvider,
    PackageGraphCommandProvider, PackageInfoProvider,
};
pub use exec::{
    DryRunExecutor, ExecOutcome, HashTrackerProvider, InternalError, SuccessOutcome,
    TaskErrorCollector, TaskExecutor, TaskWarningCollector, prefixed_ui,
};
pub use output::{StdWriter, TaskCacheOutput, TaskOutput};
use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_task_id::TaskId;
// Re-export StopExecution from turborepo-types for convenience
pub use turborepo_types::StopExecution;
use turborepo_types::{ContinueMode, EnvMode, ResolvedLogOrder, ResolvedLogPrefix, UIMode};
pub use visitor::{
    EngineExecutor, EngineMessage, EngineProvider, TaskCallback, TaskHashProvider, turbo_regex,
};

/// Configuration for task execution.
///
/// This struct contains all the options needed to configure task execution,
/// extracted from the full `RunOpts` to reduce coupling.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutorConfig {
    /// Environment mode for task execution (strict or loose)
    pub env_mode: EnvMode,
    /// Log ordering mode (stream or grouped)
    pub log_order: ResolvedLogOrder,
    /// Log prefix mode (task name or none)
    pub log_prefix: ResolvedLogPrefix,
    /// Whether this is a single-package (non-monorepo) run
    pub single_package: bool,
    /// Whether running on GitHub Actions (affects output grouping)
    pub is_github_actions: bool,
    /// Maximum number of concurrent tasks
    pub concurrency: u32,
    /// UI mode (TUI, stream, or web)
    pub ui_mode: UIMode,
    /// How to handle task failures
    pub continue_on_error: ContinueMode,
    /// Whether to redirect stderr to stdout (for GitHub Actions log grouping)
    pub redirect_stderr_to_stdout: bool,
    /// Whether to infer the framework for each workspace
    pub framework_inference: bool,
}

/// Trait for microfrontends configuration provider.
///
/// This trait abstracts the microfrontends configuration to allow the executor
/// to work with MFE features without depending on the full
/// MicrofrontendsConfigs implementation in turborepo-lib.
///
/// # Implementors
/// - `MicrofrontendsConfigs` in turborepo-lib
pub trait MfeConfigProvider: Send + Sync {
    /// Returns true if the task has an associated microfrontends proxy
    fn task_has_mfe_proxy(&self, task_id: &TaskId) -> bool;

    /// Returns the development port for a task, if configured
    fn dev_task_port(&self, task_id: &TaskId) -> Option<u16>;

    /// Returns true if the task should use Turborepo's built-in proxy
    fn task_uses_turborepo_proxy(&self, task_id: &TaskId) -> bool;

    /// Returns true if any of the given tasks are dev tasks
    fn has_dev_task<'a>(&self, task_ids: impl Iterator<Item = &'a TaskId<'static>>) -> bool;

    /// Returns true if all configs should use the Turborepo proxy
    fn should_use_turborepo_proxy(&self) -> bool;

    /// Returns the dev tasks for a package, as a map from task ID to
    /// application name
    fn dev_tasks(&self, package_name: &str) -> Option<Vec<(TaskId<'static>, String)>>;

    /// Returns the config filename path for a package
    fn config_filename(&self, package_name: &str) -> Option<String>;
}

/// Trait for task access tracing provider.
///
/// This trait abstracts task access tracing to allow the executor to work with
/// automatic caching features without depending on the full TaskAccess
/// implementation in turborepo-lib.
///
/// # Implementors
/// - `TaskAccess` in turborepo-lib
pub trait TaskAccessProvider: Clone + Send + Sync {
    /// Returns true if task access tracing is enabled
    fn is_enabled(&self) -> bool;

    /// Returns the environment variable key and trace file path for a task
    fn get_env_var(&self, task_hash: &str) -> (String, AbsoluteSystemPathBuf);

    /// Returns whether the task can be cached based on its trace.
    ///
    /// Returns `None` if tracing is disabled or the trace can't be found.
    /// Returns `Some(true)` if the task can be cached.
    /// Returns `Some(false)` if the task cannot be cached (e.g., network
    /// access).
    fn can_cache(&self, task_hash: &str, task_id: &str) -> Option<bool>;

    /// Saves the task access trace results
    fn save(&self) -> impl Future<Output = ()> + Send;
}

/// A no-op implementation of MfeConfigProvider for when MFE is not configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMfeConfig;

impl MfeConfigProvider for NoMfeConfig {
    fn task_has_mfe_proxy(&self, _task_id: &TaskId) -> bool {
        false
    }

    fn dev_task_port(&self, _task_id: &TaskId) -> Option<u16> {
        None
    }

    fn task_uses_turborepo_proxy(&self, _task_id: &TaskId) -> bool {
        false
    }

    fn has_dev_task<'a>(&self, _task_ids: impl Iterator<Item = &'a TaskId<'static>>) -> bool {
        false
    }

    fn should_use_turborepo_proxy(&self) -> bool {
        false
    }

    fn dev_tasks(&self, _package_name: &str) -> Option<Vec<(TaskId<'static>, String)>> {
        None
    }

    fn config_filename(&self, _package_name: &str) -> Option<String> {
        None
    }
}

/// A no-op implementation of TaskAccessProvider for when task access tracing is
/// disabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTaskAccess;

impl TaskAccessProvider for NoTaskAccess {
    fn is_enabled(&self) -> bool {
        false
    }

    fn get_env_var(&self, _task_hash: &str) -> (String, AbsoluteSystemPathBuf) {
        // This should never be called when tracing is disabled
        (String::new(), AbsoluteSystemPathBuf::default())
    }

    fn can_cache(&self, _task_hash: &str, _task_id: &str) -> Option<bool> {
        None
    }

    async fn save(&self) {
        // No-op
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_mfe_config() {
        let config = NoMfeConfig;
        let task_id = TaskId::new("web", "dev");

        assert!(!config.task_has_mfe_proxy(&task_id));
        assert!(config.dev_task_port(&task_id).is_none());
        assert!(!config.task_uses_turborepo_proxy(&task_id));
        assert!(!config.has_dev_task([task_id].iter()));
        assert!(!config.should_use_turborepo_proxy());
    }

    #[test]
    fn test_no_task_access() {
        let provider = NoTaskAccess;

        assert!(!provider.is_enabled());
        assert!(provider.can_cache("hash", "task").is_none());
    }

    #[tokio::test]
    async fn test_no_task_access_save() {
        let provider = NoTaskAccess;
        provider.save().await; // Should not panic
    }
}
