//! Task execution context and related types.
//!
//! This module provides the core execution infrastructure for running tasks,
//! including error types, outcome enums, and trait definitions for abstracting
//! cache and hash tracking dependencies.
//!
//! The actual `ExecContext` implementation remains in turborepo-lib due to
//! complex dependencies, but uses these types and traits.

use std::time::Duration;

use turbopath::AnchoredSystemPathBuf;
use turborepo_cache::{CacheError, CacheHitMetadata};
// Re-export CacheOutput for use by implementors
pub use turborepo_run_cache::CacheOutput;
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::task::PackageTaskEventBuilder;
use turborepo_types::OutputLogsMode;

// ============================================================================
// Error Types
// ============================================================================

/// Internal errors that can occur during task execution.
///
/// These are errors that are not task failures (non-zero exit codes) but rather
/// infrastructure errors that prevent task execution.
#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    /// IO error during execution
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Unable to determine why task exited
    #[error("unable to determine why task exited")]
    UnknownChildExit,

    /// Unable to find package manager binary
    #[error("unable to find package manager binary: {0}")]
    Which(#[from] which::Error),

    /// External process killed a task
    #[error("external process killed a task")]
    ExternalKill,

    /// Error writing logs
    #[error("error writing logs: {0}")]
    Logs(#[from] CacheError),
}

/// Outcome of task execution.
#[derive(Debug)]
pub enum ExecOutcome {
    /// All operations during execution succeeded
    Success(SuccessOutcome),
    /// An error with the task execution (non-zero exit or spawn failure)
    Task {
        /// Exit code if the task ran
        exit_code: Option<i32>,
        /// Error message
        message: String,
    },
    /// Task didn't execute normally due to a shutdown being initiated
    Shutdown,
    /// Task was stopped to be restarted
    Restarted,
}

/// Type of successful execution.
#[derive(Debug)]
pub enum SuccessOutcome {
    /// Task output was restored from cache
    CacheHit,
    /// Task was executed
    Run,
}

// ============================================================================
// Provider Traits
// ============================================================================

/// Trait for task hash tracking operations.
///
/// This trait abstracts the hash tracker to allow the executor to work with
/// hash tracking without depending on the full TaskHashTracker implementation.
pub trait TaskHashTrackerProvider: Clone + Send + Sync {
    /// Insert the cache status for a task
    fn insert_cache_status(&self, task_id: TaskId<'static>, status: CacheHitMetadata);

    /// Insert the expanded outputs for a task
    fn insert_expanded_outputs(
        &self,
        task_id: TaskId<'static>,
        outputs: Vec<AnchoredSystemPathBuf>,
    );
}

/// Trait for stop execution signaling.
///
/// This trait abstracts the mechanism for signaling task execution should stop.
pub trait StopExecutionProvider: Clone + Send + Sync + 'static {
    /// Signal that dependent tasks should stop
    fn dependent_tasks() -> Self;

    /// Signal that all tasks should stop
    fn all_tasks() -> Self;
}

/// Trait for task error collection.
pub trait TaskErrorProvider: Send + 'static {
    /// Create an error from a spawn failure
    fn from_spawn(task_id: String, error: std::io::Error) -> Self;

    /// Create an error from a task execution failure
    fn from_execution(task_id: String, command: String, exit_code: i32) -> Self;
}

/// Trait for task warning collection.
pub trait TaskWarningProvider: Send + 'static {
    /// Create a warning for missing platform environment variables
    fn from_missing_platform_env(task_id: &str, missing_vars: Vec<String>) -> Option<Self>
    where
        Self: Sized;
}

/// Trait for task cache operations.
///
/// This trait abstracts the task cache to allow the executor to work with
/// caching without depending on the full TaskCache implementation.
///
/// Note: This trait uses associated types and sync methods where possible
/// to avoid complex async trait bounds. Implementors should handle async
/// operations internally.
pub trait TaskCacheProvider: Send {
    /// The error type for cache operations
    type Error: std::error::Error + Send + 'static;

    /// Returns the output logs mode
    fn output_logs(&self) -> OutputLogsMode;

    /// Returns true if caching is disabled for this task
    fn is_caching_disabled(&self) -> bool;

    /// Returns the expanded outputs for this task
    fn expanded_outputs(&self) -> &[AnchoredSystemPathBuf];

    /// Check if cache entry exists (blocking)
    fn exists_blocking(&self) -> Result<Option<CacheHitMetadata>, Self::Error>;

    /// Restore outputs from cache (blocking)
    fn restore_outputs_blocking<O: CacheOutput>(
        &mut self,
        output: &mut O,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<Option<CacheHitMetadata>, Self::Error>;

    /// Save outputs to cache (blocking)
    fn save_outputs_blocking(
        &mut self,
        duration: Duration,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<(), Self::Error>;

    /// Handle error output
    fn on_error<O: CacheOutput>(&self, output: &mut O) -> Result<(), Self::Error>;

    /// Create an output writer for the task
    fn output_writer<W: std::io::Write>(
        &self,
        writer: W,
    ) -> Result<impl std::io::Write, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_error_display() {
        let err = InternalError::UnknownChildExit;
        assert_eq!(err.to_string(), "unable to determine why task exited");

        let err = InternalError::ExternalKill;
        assert_eq!(err.to_string(), "external process killed a task");
    }

    #[test]
    fn test_exec_outcome_variants() {
        let success = ExecOutcome::Success(SuccessOutcome::Run);
        assert!(matches!(success, ExecOutcome::Success(SuccessOutcome::Run)));

        let cache_hit = ExecOutcome::Success(SuccessOutcome::CacheHit);
        assert!(matches!(
            cache_hit,
            ExecOutcome::Success(SuccessOutcome::CacheHit)
        ));

        let task_error = ExecOutcome::Task {
            exit_code: Some(1),
            message: "failed".to_string(),
        };
        assert!(matches!(task_error, ExecOutcome::Task { .. }));

        let shutdown = ExecOutcome::Shutdown;
        assert!(matches!(shutdown, ExecOutcome::Shutdown));

        let restarted = ExecOutcome::Restarted;
        assert!(matches!(restarted, ExecOutcome::Restarted));
    }
}
