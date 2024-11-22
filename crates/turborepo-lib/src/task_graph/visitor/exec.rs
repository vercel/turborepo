use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use console::StyledObject;
use tokio::sync::oneshot;
use tracing::{error, Instrument};
use turborepo_env::{platform::PlatformEnv, EnvironmentVariableMap};
use turborepo_repository::package_manager::PackageManager;
use turborepo_telemetry::events::{task::PackageTaskEventBuilder, TrackedErrors};
use turborepo_ui::{ColorConfig, OutputWriter};

use super::{
    command::{CommandFactory, MicroFrontendProxyProvider, PackageGraphCommandProvider},
    error::{TaskError, TaskErrorCause, TaskWarning},
    output::TaskCacheOutput,
    TaskOutput, Visitor,
};
use crate::{
    config::UIMode,
    engine::{Engine, StopExecution},
    process::{ChildExit, Command, ProcessManager},
    run::{
        summary::{SpacesTaskClient, SpacesTaskInformation, TaskExecutionSummary, TaskTracker},
        task_access::TaskAccess,
        task_id::TaskId,
        CacheOutput, TaskCache,
    },
    task_hash::TaskHashTracker,
};

pub struct ExecContextFactory<'a> {
    visitor: &'a Visitor<'a>,
    errors: Arc<Mutex<Vec<TaskError>>>,
    manager: ProcessManager,
    engine: &'a Arc<Engine>,
    command_factory: CommandFactory<'a>,
}

impl<'a> ExecContextFactory<'a> {
    pub fn new(
        visitor: &'a Visitor<'a>,
        errors: Arc<Mutex<Vec<TaskError>>>,
        manager: ProcessManager,
        engine: &'a Arc<Engine>,
    ) -> Result<Self, super::Error> {
        let pkg_graph_provider = PackageGraphCommandProvider::new(
            visitor.repo_root,
            &visitor.package_graph,
            visitor.run_opts.task_args(),
            visitor.micro_frontends_configs,
        );
        let mut command_factory = CommandFactory::new();
        if let Some(micro_frontends_configs) = visitor.micro_frontends_configs {
            command_factory.add_provider(MicroFrontendProxyProvider::new(
                visitor.repo_root,
                &visitor.package_graph,
                engine,
                micro_frontends_configs,
            ));
        }
        command_factory.add_provider(pkg_graph_provider);

        Ok(Self {
            visitor,
            errors,
            manager,
            engine,
            command_factory,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn exec_context(
        &self,
        task_id: TaskId<'static>,
        task_hash: String,
        task_cache: TaskCache,
        mut execution_env: EnvironmentVariableMap,
        takes_input: bool,
        task_access: TaskAccess,
    ) -> Result<Option<ExecContext>, super::Error> {
        let task_id_for_display = self.visitor.display_task_id(&task_id);
        let task_id_string = &task_id.to_string();
        self.populate_env(&mut execution_env, &task_hash, &task_access);
        let Some(cmd) = self
            .command_factory
            .command(&task_id, execution_env.clone())?
        else {
            return Ok(None);
        };
        Ok(Some(ExecContext {
            engine: self.engine.clone(),
            ui_mode: self.visitor.run_opts.ui_mode,
            color_config: self.visitor.color_config,
            is_github_actions: self.visitor.run_opts.is_github_actions,
            pretty_prefix: self
                .visitor
                .color_cache
                .prefix_with_color(task_id_string, &self.visitor.prefix(&task_id)),
            task_id,
            task_id_for_display,
            task_cache,
            hash_tracker: self.visitor.task_hasher.task_hash_tracker(),
            package_manager: *self.visitor.package_graph.package_manager(),
            manager: self.manager.clone(),
            task_hash,
            execution_env,
            continue_on_error: self.visitor.run_opts.continue_on_error,
            errors: self.errors.clone(),
            warnings: self.visitor.warnings.clone(),
            takes_input,
            task_access,
            cmd,
            platform_env: PlatformEnv::new(),
        }))
    }

    pub fn dry_run_exec_context(
        &self,
        task_id: TaskId<'static>,
        task_cache: TaskCache,
    ) -> DryRunExecContext {
        DryRunExecContext {
            task_id,
            task_cache,
            hash_tracker: self.visitor.task_hasher.task_hash_tracker(),
        }
    }

    // Add any env vars that `turbo` provides to the task environment
    fn populate_env(
        &self,
        execution_env: &mut EnvironmentVariableMap,
        task_hash: &str,
        task_access: &TaskAccess,
    ) {
        // Always last to make sure it overwrites any user configured env var.
        execution_env.insert("TURBO_HASH".to_owned(), task_hash.to_owned());

        // Allow downstream tools to detect if the task is being ran with TUI
        if self.visitor.run_opts.ui_mode.use_tui() {
            execution_env.insert("TURBO_IS_TUI".to_owned(), "true".to_owned());
        }

        // enable task access tracing
        // set the trace file env var - frameworks that support this can use it to
        // write out a trace file that we will use to automatically cache the task
        if task_access.is_enabled() {
            let (task_access_trace_key, trace_file) = task_access.get_env_var(task_hash);
            execution_env.insert(task_access_trace_key, trace_file.to_string());
        }
    }
}

pub struct ExecContext {
    engine: Arc<Engine>,
    color_config: ColorConfig,
    ui_mode: UIMode,
    is_github_actions: bool,
    pretty_prefix: StyledObject<String>,
    task_id: TaskId<'static>,
    task_id_for_display: String,
    task_cache: TaskCache,
    hash_tracker: TaskHashTracker,
    package_manager: PackageManager,
    manager: ProcessManager,
    task_hash: String,
    execution_env: EnvironmentVariableMap,
    continue_on_error: bool,
    errors: Arc<Mutex<Vec<TaskError>>>,
    warnings: Arc<Mutex<Vec<TaskWarning>>>,
    takes_input: bool,
    task_access: TaskAccess,
    cmd: Command,
    platform_env: PlatformEnv,
}

enum ExecOutcome {
    // All operations during execution succeeded
    Success(SuccessOutcome),
    // An error with the task execution
    Task {
        exit_code: Option<i32>,
        message: String,
    },
    // Task didn't execute normally due to a shutdown being initiated by another task
    Shutdown,
}

enum SuccessOutcome {
    CacheHit,
    Run,
}

impl ExecContext {
    pub async fn execute_dry_run(&mut self, tracker: TaskTracker<()>) {
        if let Ok(Some(status)) = self.task_cache.exists().await {
            self.hash_tracker
                .insert_cache_status(self.task_id.clone(), status);
        }

        tracker.dry_run().await;
    }
    pub async fn execute(
        &mut self,
        parent_span_id: Option<tracing::Id>,
        tracker: TaskTracker<()>,
        output_client: TaskOutput<impl Write>,
        callback: oneshot::Sender<Result<(), StopExecution>>,
        spaces_client: Option<SpacesTaskClient>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<(), InternalError> {
        let tracker = tracker.start().await;
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
        let logs = match output_client.finish(is_error, is_cache_hit) {
            Ok(logs) => logs,
            Err(e) => {
                telemetry.track_error(TrackedErrors::DaemonFailedToMarkOutputsAsCached);
                error!("unable to flush output client: {e}");
                result = Err(InternalError::Io(e));
                None
            }
        };

        match result {
            Ok(ExecOutcome::Success(outcome)) => {
                let task_summary = match outcome {
                    SuccessOutcome::CacheHit => tracker.cached().await,
                    SuccessOutcome::Run => tracker.build_succeeded(0).await,
                };
                callback.send(Ok(())).ok();
                if let Some(client) = spaces_client {
                    let logs = logs.expect("spaces enabled logs should be collected");
                    let info = self.spaces_task_info(self.task_id.clone(), task_summary, logs);
                    client.finish_task(info).await.ok();
                }
            }
            Ok(ExecOutcome::Task { exit_code, message }) => {
                let task_summary = tracker.build_failed(exit_code, message).await;
                callback
                    .send(match self.continue_on_error {
                        true => Ok(()),
                        false => Err(StopExecution),
                    })
                    .ok();

                match (spaces_client, self.continue_on_error) {
                    // Nothing to do
                    (None, true) => (),
                    // Shut down manager
                    (None, false) => self.manager.stop().await,
                    // Send task
                    (Some(client), true) => {
                        let logs = logs.expect("spaced enabled logs should be collected");
                        let info = self.spaces_task_info(self.task_id.clone(), task_summary, logs);
                        client.finish_task(info).await.ok();
                    }
                    // Send task and shut down manager
                    (Some(client), false) => {
                        let logs = logs.unwrap_or_default();
                        let info = self.spaces_task_info(self.task_id.clone(), task_summary, logs);
                        // Ignore spaces result as that indicates handler is shut down and we are
                        // unable to send information to spaces
                        let (_spaces_result, _) =
                            tokio::join!(client.finish_task(info), self.manager.stop());
                    }
                }
            }
            Ok(ExecOutcome::Shutdown) => {
                tracker.cancel();
                callback.send(Err(StopExecution)).ok();
                // Probably overkill here, but we should make sure the process manager is
                // stopped if we think we're shutting down.
                self.manager.stop().await;
            }
            Err(e) => {
                tracker.cancel();
                callback.send(Err(StopExecution)).ok();
                self.manager.stop().await;
                return Err(e);
            }
        }

        Ok(())
    }

    fn prefixed_ui<'a, W: Write>(
        &self,
        output_client: &'a TaskOutput<W>,
    ) -> TaskCacheOutput<OutputWriter<'a, W>> {
        match output_client {
            TaskOutput::Direct(client) => TaskCacheOutput::Direct(Visitor::prefixed_ui(
                self.color_config,
                self.is_github_actions,
                client.stdout(),
                client.stderr(),
                self.pretty_prefix.clone(),
            )),
            TaskOutput::UI(task) => TaskCacheOutput::UI(task.clone()),
        }
    }

    async fn execute_inner(
        &mut self,
        output_client: &TaskOutput<impl Write>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<ExecOutcome, InternalError> {
        let task_start = Instant::now();
        let mut prefixed_ui = self.prefixed_ui(output_client);

        if self.ui_mode.has_sender() {
            if let TaskOutput::UI(task) = output_client {
                let output_logs = self.task_cache.output_logs().into();
                task.start(output_logs);
            }
        }

        if !self.task_cache.is_caching_disabled() {
            let missing_platform_env = self.platform_env.validate(&self.execution_env);
            if let Some(warning) = TaskWarning::new(&self.task_id_for_display, missing_platform_env)
            {
                self.warnings
                    .lock()
                    .expect("warnings lock poisoned")
                    .push(warning);
            }
        }

        match self
            .task_cache
            .restore_outputs(&mut prefixed_ui, telemetry)
            .await
        {
            Ok(Some(status)) => {
                // we need to set expanded outputs
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

        let cmd = self.cmd.clone();

        let mut process = match self.manager.spawn(cmd, Duration::from_millis(500)) {
            Some(Ok(child)) => child,
            // Turbo was unable to spawn a process
            Some(Err(e)) => {
                // Note: we actually failed to spawn, but this matches the Go output
                prefixed_ui.error(&format!("command finished with error: {e}"));
                let error_string = e.to_string();
                self.errors
                    .lock()
                    .expect("lock poisoned")
                    .push(TaskError::from_spawn(self.task_id_for_display.clone(), e));
                return Ok(ExecOutcome::Task {
                    exit_code: None,
                    message: error_string,
                });
            }
            // Turbo is shutting down
            None => {
                return Ok(ExecOutcome::Shutdown);
            }
        };

        if self.ui_mode.has_sender() && self.takes_input {
            if let TaskOutput::UI(task) = output_client {
                if let Some(stdin) = process.stdin() {
                    task.set_stdin(stdin);
                }
            }
        }

        // Even if user does not have the TUI and cannot interact with a task, we keep
        // stdin open for persistent tasks as some programs will shut down if stdin is
        // closed.
        if !self.takes_input && !self.manager.closing_stdin_ends_process() {
            process.stdin();
        }

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
                // TODO: how can this happen? we only update the
                // exit status with Some and it is only initialized with
                // None. Is it still running?
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
                        // If no errors, update hash tracker with expanded outputs
                        self.hash_tracker.insert_expanded_outputs(
                            self.task_id.clone(),
                            self.task_cache.expanded_outputs().to_vec(),
                        );
                    }
                }

                // Return success outcome
                Ok(ExecOutcome::Success(SuccessOutcome::Run))
            }
            ChildExit::Finished(Some(code)) => {
                // If there was an error, flush the buffered output
                if let Err(e) = stdout_writer.flush() {
                    error!("error flushing logs: {e}");
                }
                if let Err(e) = self.task_cache.on_error(&mut prefixed_ui) {
                    error!("error reading logs: {e}");
                }
                let error = TaskErrorCause::from_execution(process.label().to_string(), code);
                let message = error.to_string();
                if self.continue_on_error {
                    prefixed_ui.warn("command finished with error, but continuing...");
                } else {
                    prefixed_ui.error(&format!("command finished with error: {error}"));
                }
                self.errors
                    .lock()
                    .expect("lock poisoned")
                    .push(TaskError::new(self.task_id_for_display.clone(), error));
                Ok(ExecOutcome::Task {
                    exit_code: Some(code),
                    message,
                })
            }
            // The child exited in a way where we can't figure out how it finished so we assume it
            // failed.
            ChildExit::Finished(None) | ChildExit::Failed => Err(InternalError::UnknownChildExit),
            // Something else killed the child
            ChildExit::KilledExternal => Err(InternalError::ExternalKill),
            // The child was killed by turbo indicating a shutdown
            ChildExit::Killed => Ok(ExecOutcome::Shutdown),
        }
    }

    fn spaces_task_info(
        &self,
        task_id: TaskId<'static>,
        execution_summary: TaskExecutionSummary,
        logs: Vec<u8>,
    ) -> SpacesTaskInformation {
        let dependencies = self.engine.dependencies(&task_id);
        let dependents = self.engine.dependents(&task_id);
        let cache_status = self.hash_tracker.cache_status(&task_id);
        SpacesTaskInformation {
            task_id,
            execution_summary,
            logs,
            hash: self.task_hash.clone(),
            cache_status,
            dependencies,
            dependents,
        }
    }
}

pub struct DryRunExecContext {
    task_id: TaskId<'static>,
    task_cache: TaskCache,
    hash_tracker: TaskHashTracker,
}

#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unable to determine why task exited")]
    UnknownChildExit,
    #[error("unable to find package manager binary: {0}")]
    Which(#[from] which::Error),
    #[error("external process killed a task")]
    ExternalKill,
    #[error("error writing logs: {0}")]
    Logs(#[from] crate::run::CacheError),
}
impl DryRunExecContext {
    pub async fn execute_dry_run(&self, tracker: TaskTracker<()>) -> Result<(), InternalError> {
        // may also need to do framework & command stuff?
        if let Ok(Some(status)) = self.task_cache.exists().await {
            self.hash_tracker
                .insert_cache_status(self.task_id.clone(), status);
        }
        tracker.dry_run().await;
        Ok(())
    }
}
