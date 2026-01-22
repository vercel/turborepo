//! Task execution logic for Turborepo.
//!
//! This module contains the core task execution infrastructure, including:
//! - `TaskExecutor`: Executes a single task with caching, output handling, and
//!   error collection
//! - Provider traits for abstracting dependencies (hash tracking, errors,
//!   warnings)
//! - Result types (`ExecOutcome`, `SuccessOutcome`, `InternalError`)

use std::{
    io::Write,
    time::{Duration, Instant},
};

use console::{Style, StyledObject};
use tokio::sync::oneshot;
use tracing::{Instrument, error};
use turbopath::AnchoredSystemPathBuf;
use turborepo_cache::CacheHitMetadata;
use turborepo_env::{EnvironmentVariableMap, platform::PlatformEnv};
use turborepo_process::{ChildExit, Command, ProcessManager};
use turborepo_run_cache::{CacheOutput, TaskCache};
use turborepo_run_summary::TaskTracker;
use turborepo_task_id::TaskId;
use turborepo_telemetry::events::{TrackedErrors, task::PackageTaskEventBuilder};
use turborepo_types::{ContinueMode, StopExecution, UIMode};
use turborepo_ui::{ColorConfig, OutputWriter};

use crate::{TaskAccessProvider, TaskCacheOutput, TaskOutput};

/// Windows NT status codes that indicate out-of-memory conditions.
/// These are the signed i32 representations of the unsigned NT status codes.
#[cfg(windows)]
mod windows_oom {
    /// STATUS_NO_MEMORY (0xC0000017) - insufficient memory to complete
    /// operation
    pub const STATUS_NO_MEMORY: i32 = 0xC0000017_u32 as i32;
    /// STATUS_STACK_OVERFLOW (0xC00000FD) - stack overflow, often
    /// memory-related
    pub const STATUS_STACK_OVERFLOW: i32 = 0xC00000FD_u32 as i32;
    /// STATUS_COMMITMENT_LIMIT (0xC000012D) - system committed memory limit
    /// reached
    pub const STATUS_COMMITMENT_LIMIT: i32 = 0xC000012D_u32 as i32;

    /// Check if an exit code indicates an out-of-memory condition on Windows.
    pub fn is_oom_exit_code(code: i32) -> bool {
        matches!(
            code,
            STATUS_NO_MEMORY | STATUS_STACK_OVERFLOW | STATUS_COMMITMENT_LIMIT
        )
    }

    /// Get a human-readable description of the Windows OOM exit code.
    pub fn oom_description(code: i32) -> &'static str {
        match code {
            STATUS_NO_MEMORY => "STATUS_NO_MEMORY: insufficient memory",
            STATUS_STACK_OVERFLOW => "STATUS_STACK_OVERFLOW: stack overflow",
            STATUS_COMMITMENT_LIMIT => "STATUS_COMMITMENT_LIMIT: system memory limit reached",
            _ => "unknown memory error",
        }
    }
}

/// Get a description for an OOM-related exit code, if applicable.
fn oom_description(code: i32) -> Option<&'static str> {
    #[cfg(windows)]
    {
        if windows_oom::is_oom_exit_code(code) {
            Some(windows_oom::oom_description(code))
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        if code == 137 {
            Some("SIGKILL (signal 9): likely killed by OOM killer")
        } else {
            None
        }
    }
}

// =============================================================================
// Result Types
// =============================================================================

/// The outcome of task execution.
#[derive(Debug)]
pub enum ExecOutcome {
    /// All operations during execution succeeded
    Success(SuccessOutcome),
    /// An error with the task execution
    Task {
        exit_code: Option<i32>,
        message: String,
    },
    /// Task didn't execute normally due to a shutdown being initiated by
    /// another task
    Shutdown,
    /// Task was stopped to be restarted
    Restarted,
}

/// The type of successful outcome.
#[derive(Debug)]
pub enum SuccessOutcome {
    /// Task output was restored from cache
    CacheHit,
    /// Task was executed
    Run,
}

/// Internal errors that can occur during task execution.
#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unable to determine why task exited")]
    UnknownChildExit,
    #[error("unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
    #[error("error with cache: {0}")]
    Cache(#[from] turborepo_run_cache::Error),
}

// =============================================================================
// Provider Traits
// =============================================================================

/// Provider trait for hash tracking operations.
///
/// This abstracts the `TaskHashTracker` from `turborepo-task-hash`.
pub trait HashTrackerProvider: Clone + Send {
    /// Record the cache status for a task.
    fn insert_cache_status(&self, task_id: TaskId<'static>, status: CacheHitMetadata);

    /// Record the expanded outputs for a task.
    fn insert_expanded_outputs(
        &self,
        task_id: TaskId<'static>,
        outputs: Vec<AnchoredSystemPathBuf>,
    );
}

/// Provider trait for collecting task errors.
pub trait TaskErrorCollector: Clone + Send {
    /// Push an error from a spawn failure.
    fn push_spawn_error(&self, task_id: String, error: std::io::Error);

    /// Push an error from task execution.
    fn push_execution_error(&self, task_id: String, command: String, exit_code: i32);
}

/// Provider trait for collecting task warnings.
pub trait TaskWarningCollector: Clone + Send {
    /// Create and push a warning for missing platform env vars, if applicable.
    fn push_platform_env_warning(&self, task_id: &str, missing_vars: Vec<String>);
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a prefixed UI for task output.
///
/// This creates a `PrefixedUI` configured with appropriate prefixes for
/// normal output, errors, and warnings.
pub fn prefixed_ui<W: Write>(
    color_config: ColorConfig,
    is_github_actions: bool,
    stdout: W,
    stderr: W,
    prefix: StyledObject<String>,
    include_timestamps: bool,
) -> turborepo_ui::PrefixedUI<W> {
    let mut prefixed_ui = turborepo_ui::PrefixedUI::new(color_config, stdout, stderr)
        .with_output_prefix(prefix.clone())
        .with_error_prefix(
            Style::new().apply_to(format!("{}ERROR: ", color_config.apply(prefix.clone()))),
        )
        .with_warn_prefix(prefix)
        .with_timestamps(include_timestamps);
    if is_github_actions {
        prefixed_ui = prefixed_ui
            .with_error_prefix(Style::new().apply_to("[ERROR] ".to_string()))
            .with_warn_prefix(Style::new().apply_to("[WARN] ".to_string()));
    }
    prefixed_ui
}

// =============================================================================
// TaskExecutor
// =============================================================================

/// Executes a single task.
///
/// This struct encapsulates all the logic for executing a task, including:
/// - Checking and restoring from cache
/// - Spawning the process
/// - Capturing output
/// - Saving to cache on success
/// - Reporting errors and warnings
///
/// It uses the concrete `TaskCache` from turborepo-run-cache but is generic
/// over the hash tracker, error collector, and warning collector to allow
/// different implementations (e.g., for testing).
pub struct TaskExecutor<H, E, W, A> {
    // Task identification
    pub task_id: TaskId<'static>,
    pub task_id_for_display: String,
    pub task_hash: String,

    // Execution
    pub cmd: Command,
    pub execution_env: EnvironmentVariableMap,
    pub manager: ProcessManager,
    pub takes_input: bool,

    // Configuration
    pub continue_on_error: ContinueMode,
    pub ui_mode: UIMode,
    pub color_config: ColorConfig,
    pub is_github_actions: bool,
    pub pretty_prefix: StyledObject<String>,

    // Cache
    pub task_cache: TaskCache,

    // Providers
    pub hash_tracker: H,
    pub errors: E,
    pub warnings: W,
    pub task_access: A,

    // Platform env validation
    pub platform_env: PlatformEnv,
}

impl<H, E, W, A> TaskExecutor<H, E, W, A>
where
    H: HashTrackerProvider,
    E: TaskErrorCollector,
    W: TaskWarningCollector,
    A: TaskAccessProvider,
{
    /// Execute a dry run (only check cache status).
    pub async fn execute_dry_run(&mut self, tracker: TaskTracker<()>) {
        if let Ok(Some(status)) = self.task_cache.exists().await {
            self.hash_tracker
                .insert_cache_status(self.task_id.clone(), status);
        }
        tracker.dry_run().await;
    }

    /// Execute the task.
    ///
    /// This is the main entry point for task execution. It:
    /// 1. Starts tracking
    /// 2. Executes the task (checking cache, running process, saving outputs)
    /// 3. Reports the outcome via the callback
    /// 4. Updates the tracker
    pub async fn execute<O: Write>(
        &mut self,
        parent_span_id: Option<tracing::Id>,
        tracker: TaskTracker<()>,
        output_client: TaskOutput<O>,
        callback: oneshot::Sender<Result<(), StopExecution>>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<(), InternalError> {
        let tracker: TaskTracker<chrono::DateTime<chrono::Local>> = tracker.start().await;
        let span = tracing::debug_span!("execute_task", task = %self.task_id.task());
        span.follows_from(parent_span_id);

        let mut result = self
            .execute_inner(&output_client, telemetry)
            .instrument(span)
            .await;

        // If the task resulted in an error, do not group in order to better highlight
        // the error.
        let is_error = matches!(result, Ok(ExecOutcome::Task { .. }));
        let is_cache_hit = matches!(result, Ok(ExecOutcome::Success(SuccessOutcome::CacheHit)));
        if let Err(e) = output_client.finish(is_error, is_cache_hit) {
            telemetry.track_error(TrackedErrors::DaemonFailedToMarkOutputsAsCached);
            error!("unable to flush output client: {e}");
            result = Err(InternalError::Io(e));
        }

        match result {
            Ok(ExecOutcome::Success(outcome)) => {
                match outcome {
                    SuccessOutcome::CacheHit => tracker.cached().await,
                    SuccessOutcome::Run => tracker.build_succeeded(0).await,
                };
                callback.send(Ok(())).ok();
            }
            Ok(ExecOutcome::Task { exit_code, message }) => {
                tracker.build_failed(exit_code, message).await;
                callback
                    .send(match self.continue_on_error {
                        ContinueMode::Always => Ok(()),
                        ContinueMode::DependenciesSuccessful => Err(StopExecution::DependentTasks),
                        ContinueMode::Never => Err(StopExecution::AllTasks),
                    })
                    .ok();

                match self.continue_on_error {
                    ContinueMode::Always | ContinueMode::DependenciesSuccessful => (),
                    ContinueMode::Never => self.manager.stop().await,
                }
            }
            Ok(ExecOutcome::Shutdown) => {
                tracker.cancel();
                callback.send(Err(StopExecution::AllTasks)).ok();
                self.manager.stop().await;
            }
            Ok(ExecOutcome::Restarted) => {
                tracker.cancel();
                callback.send(Err(StopExecution::DependentTasks)).ok();
            }
            Err(e) => {
                tracker.cancel();
                callback.send(Err(StopExecution::AllTasks)).ok();
                self.manager.stop().await;
                return Err(e);
            }
        }

        Ok(())
    }

    fn prefixed_ui<'a, O: Write>(
        &self,
        output_client: &'a TaskOutput<O>,
    ) -> TaskCacheOutput<OutputWriter<'a, O>> {
        match output_client {
            TaskOutput::Direct(client) => TaskCacheOutput::Direct(prefixed_ui(
                self.color_config,
                self.is_github_actions,
                client.stdout(),
                client.stderr(),
                self.pretty_prefix.clone(),
                self.ui_mode.should_include_timestamps(),
            )),
            TaskOutput::UI(task) => TaskCacheOutput::UI(task.clone()),
        }
    }

    async fn execute_inner<O: Write>(
        &mut self,
        output_client: &TaskOutput<O>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<ExecOutcome, InternalError> {
        let task_start = Instant::now();
        let mut prefixed_ui = self.prefixed_ui(output_client);

        if self.ui_mode.has_sender()
            && let TaskOutput::UI(task) = output_client
        {
            let output_logs = self.task_cache.output_logs().into();
            task.start(output_logs);
        }

        // Check platform env warnings
        if !self.task_cache.is_caching_disabled() {
            let missing_platform_env = self.platform_env.validate(&self.execution_env);
            if !missing_platform_env.is_empty() {
                self.warnings
                    .push_platform_env_warning(&self.task_id_for_display, missing_platform_env);
            }
        }

        // Try to restore from cache
        match self
            .task_cache
            .restore_outputs(&mut prefixed_ui, telemetry)
            .await
        {
            Ok(Some(status)) => {
                self.hash_tracker.insert_expanded_outputs(
                    self.task_id.clone(),
                    self.task_cache.expanded_outputs().to_vec(),
                );
                self.hash_tracker
                    .insert_cache_status(self.task_id.clone(), status);
                return Ok(ExecOutcome::Success(SuccessOutcome::CacheHit));
            }
            Ok(None) => (),
            Err(e) => {
                telemetry.track_error(TrackedErrors::ErrorFetchingFromCache);
                prefixed_ui.error(&format!("error fetching from cache: {e}"));
            }
        }

        // Spawn the process
        let cmd = self.cmd.clone();
        let mut process =
            match self
                .manager
                .spawn(cmd, Duration::from_millis(500), self.task_id.clone())
            {
                Some(Ok(child)) => child,
                Some(Err(e)) => {
                    prefixed_ui.error(&format!("command finished with error: {e}"));
                    let error_string = e.to_string();
                    self.errors
                        .push_spawn_error(self.task_id_for_display.clone(), e);
                    return Ok(ExecOutcome::Task {
                        exit_code: None,
                        message: error_string,
                    });
                }
                None => {
                    return Ok(ExecOutcome::Shutdown);
                }
            };

        // Handle stdin for interactive tasks
        if self.ui_mode.has_sender()
            && self.takes_input
            && let TaskOutput::UI(task) = output_client
            && let Some(stdin) = process.stdin()
        {
            task.set_stdin(stdin);
        }

        // Keep stdin open for persistent tasks
        if !self.takes_input && !self.manager.closing_stdin_ends_process() {
            process.stdin();
        }

        // Create output writer and pipe outputs
        let mut stdout_writer = self
            .task_cache
            .output_writer(prefixed_ui.task_writer())
            .inspect_err(|_| {
                telemetry.track_error(TrackedErrors::FailedToCaptureOutputs);
            })?;

        let exit_status = match process.wait_with_piped_outputs(&mut stdout_writer).await {
            Ok(Some(exit_status)) => exit_status,
            Err(e) => {
                telemetry.track_error(TrackedErrors::FailedToPipeOutputs);
                return Err(e.into());
            }
            Ok(None) => {
                telemetry.track_error(TrackedErrors::UnknownChildExit);
                error!("unable to determine why child exited");
                return Err(InternalError::UnknownChildExit);
            }
        };
        let task_duration = task_start.elapsed();

        match exit_status {
            ChildExit::Finished(Some(0)) => {
                // Attempt to flush stdout_writer and log any errors encountered
                if let Err(e) = stdout_writer.flush() {
                    error!("{e}");
                } else if self
                    .task_access
                    .can_cache(&self.task_hash, &self.task_id_for_display)
                    .unwrap_or(true)
                {
                    if let Err(e) = self.task_cache.save_outputs(task_duration, telemetry).await {
                        error!("error caching output: {e}");
                        return Err(e.into());
                    } else {
                        self.hash_tracker.insert_expanded_outputs(
                            self.task_id.clone(),
                            self.task_cache.expanded_outputs().to_vec(),
                        );
                    }
                }

                Ok(ExecOutcome::Success(SuccessOutcome::Run))
            }
            ChildExit::Finished(Some(code)) => {
                if let Err(e) = stdout_writer.flush() {
                    error!("error flushing logs: {e}");
                }
                if let Err(e) = self.task_cache.on_error(&mut prefixed_ui) {
                    error!("error reading logs: {e}");
                }
                // Check if this looks like an OOM-related exit code
                let message = if let Some(oom_desc) = oom_description(code) {
                    format!(
                        "command {} was killed (exit code {}): {}, likely ran out of memory",
                        process.label(),
                        code,
                        oom_desc
                    )
                } else {
                    format!("command {} exited ({})", process.label(), code)
                };
                match self.continue_on_error {
                    ContinueMode::Never => {
                        prefixed_ui.error(&format!("command finished with error: {}", message))
                    }
                    ContinueMode::Always | ContinueMode::DependenciesSuccessful => {
                        prefixed_ui.warn("command finished with error, but continuing...")
                    }
                }
                self.errors.push_execution_error(
                    self.task_id_for_display.clone(),
                    process.label().to_string(),
                    code,
                );
                Ok(ExecOutcome::Task {
                    exit_code: Some(code),
                    message,
                })
            }
            ChildExit::Finished(None) | ChildExit::Failed => {
                // Process exited without a code (e.g., killed by signal) or we failed to get
                // status. Treat as a task failure with exit code 1.
                if let Err(e) = stdout_writer.flush() {
                    error!("error flushing logs: {e}");
                }
                if let Err(e) = self.task_cache.on_error(&mut prefixed_ui) {
                    error!("error reading logs: {e}");
                }
                let message = format!("command {} exited unexpectedly", process.label());
                match self.continue_on_error {
                    ContinueMode::Never => {
                        prefixed_ui.error(&format!("command finished with error: {}", message))
                    }
                    ContinueMode::Always | ContinueMode::DependenciesSuccessful => {
                        prefixed_ui.warn("command finished with error, but continuing...")
                    }
                }
                self.errors.push_execution_error(
                    self.task_id_for_display.clone(),
                    process.label().to_string(),
                    1,
                );
                Ok(ExecOutcome::Task {
                    exit_code: Some(1),
                    message,
                })
            }
            ChildExit::KilledExternal => {
                // Process was killed by an external signal (e.g., OOM killer sending SIGKILL).
                // Use exit code 137 (128 + 9) which is the conventional code for SIGKILL.
                const SIGKILL_EXIT_CODE: i32 = 137;
                if let Err(e) = stdout_writer.flush() {
                    error!("error flushing logs: {e}");
                }
                if let Err(e) = self.task_cache.on_error(&mut prefixed_ui) {
                    error!("error reading logs: {e}");
                }
                let message = format!(
                    "command {} was killed (exit code {}), likely due to running out of memory",
                    process.label(),
                    SIGKILL_EXIT_CODE
                );
                match self.continue_on_error {
                    ContinueMode::Never => {
                        prefixed_ui.error(&format!("command finished with error: {}", message))
                    }
                    ContinueMode::Always | ContinueMode::DependenciesSuccessful => {
                        prefixed_ui.warn("command finished with error, but continuing...")
                    }
                }
                self.errors.push_execution_error(
                    self.task_id_for_display.clone(),
                    process.label().to_string(),
                    SIGKILL_EXIT_CODE,
                );
                Ok(ExecOutcome::Task {
                    exit_code: Some(SIGKILL_EXIT_CODE),
                    message,
                })
            }
            ChildExit::Killed | ChildExit::Interrupted => {
                if process.is_closing() {
                    Ok(ExecOutcome::Shutdown)
                } else {
                    Ok(ExecOutcome::Restarted)
                }
            }
        }
    }
}

/// Execution context for dry runs.
///
/// A simplified executor for dry run mode that only checks cache status.
pub struct DryRunExecutor<H> {
    pub task_id: TaskId<'static>,
    pub task_cache: TaskCache,
    pub hash_tracker: H,
}

impl<H: HashTrackerProvider> DryRunExecutor<H> {
    pub async fn execute_dry_run(&self, tracker: TaskTracker<()>) -> Result<(), InternalError> {
        if let Ok(Some(status)) = self.task_cache.exists().await {
            self.hash_tracker
                .insert_cache_status(self.task_id.clone(), status);
        }
        tracker.dry_run().await;
        Ok(())
    }
}
