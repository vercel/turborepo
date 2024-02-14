use std::{
    borrow::Cow,
    collections::HashSet,
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use console::{Style, StyledObject};
use futures::{stream::FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, Instrument, Span};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ci::{Vendor, VendorBehavior};
use turborepo_env::{EnvironmentVariableMap, ResolvedEnvMode};
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, ROOT_PKG_NAME},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::{
    generic::GenericEventBuilder, task::PackageTaskEventBuilder, EventBuilder, TrackedErrors,
};
use turborepo_ui::{ColorSelector, OutputClient, OutputSink, OutputWriter, PrefixedUI, UI};
use which::which;

use crate::{
    cli::EnvMode,
    engine::{Engine, ExecutionOptions, StopExecution},
    opts::RunOpts,
    process::{ChildExit, Command, ProcessManager},
    run::{
        global_hash::GlobalHashableInputs,
        summary::{
            self, GlobalHashSummary, RunTracker, SpacesTaskClient, SpacesTaskInformation,
            TaskExecutionSummary, TaskTracker,
        },
        task_access::TaskAccess,
        task_id::TaskId,
        RunCache, TaskCache,
    },
    task_hash::{self, PackageInputsHashes, TaskHashTracker, TaskHashTrackerState, TaskHasher},
};

// This holds the whole world
pub struct Visitor<'a> {
    color_cache: ColorSelector,
    dry: bool,
    global_env: EnvironmentVariableMap,
    global_env_mode: EnvMode,
    manager: ProcessManager,
    run_opts: &'a RunOpts,
    package_graph: Arc<PackageGraph>,
    repo_root: &'a AbsoluteSystemPath,
    run_cache: Arc<RunCache>,
    run_tracker: RunTracker,
    task_access: TaskAccess,
    sink: OutputSink<StdWriter>,
    task_hasher: TaskHasher<'a>,
    ui: UI,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot find package {package_name} for task {task_id}")]
    MissingPackage {
        package_name: PackageName,
        task_id: TaskId<'static>,
    },
    #[error(
        "root task {task_name} ({command}) looks like it invokes turbo and might cause a loop"
    )]
    RecursiveTurbo { task_name: String, command: String },
    #[error("Could not find definition for task")]
    MissingDefinition,
    #[error("error while executing engine: {0}")]
    Engine(#[from] crate::engine::ExecuteError),
    #[error(transparent)]
    TaskHash(#[from] task_hash::Error),
    #[error(transparent)]
    RunSummary(#[from] summary::Error),
}

impl<'a> Visitor<'a> {
    // Disabling this lint until we stop adding state to the visitor.
    // Once we have the full picture we will go about grouping these pieces of data
    // together
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        package_graph: Arc<PackageGraph>,
        run_cache: Arc<RunCache>,
        run_tracker: RunTracker,
        task_access: TaskAccess,
        run_opts: &'a RunOpts,
        package_inputs_hashes: PackageInputsHashes,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        global_env_mode: EnvMode,
        ui: UI,
        silent: bool,
        manager: ProcessManager,
        repo_root: &'a AbsoluteSystemPath,
        global_env: EnvironmentVariableMap,
    ) -> Self {
        let task_hasher = TaskHasher::new(
            package_inputs_hashes,
            run_opts,
            env_at_execution_start,
            global_hash,
        );
        let sink = Self::sink(run_opts, silent);
        let color_cache = ColorSelector::default();

        Self {
            color_cache,
            dry: false,
            global_env_mode,
            manager,
            run_opts,
            package_graph,
            repo_root,
            run_cache,
            run_tracker,
            task_access,
            sink,
            task_hasher,
            ui,
            global_env,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn visit(
        &self,
        engine: Arc<Engine>,
        telemetry: &GenericEventBuilder,
    ) -> Result<Vec<TaskError>, Error> {
        let concurrency = self.run_opts.concurrency as usize;
        let (node_sender, mut node_stream) = mpsc::channel(concurrency);
        let engine_handle = {
            let engine = engine.clone();
            tokio::spawn(engine.execute(ExecutionOptions::new(false, concurrency), node_sender))
        };
        let mut tasks = FuturesUnordered::new();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let span = Span::current();

        let factory = ExecContextFactory::new(self, errors.clone(), self.manager.clone(), &engine);

        while let Some(message) = node_stream.recv().await {
            let span = tracing::debug_span!(parent: &span, "queue_task", task = %message.info);
            let _enter = span.enter();
            let crate::engine::Message { info, callback } = message;
            let package_name = PackageName::from(info.package());

            let workspace_info =
                self.package_graph
                    .package_info(&package_name)
                    .ok_or_else(|| Error::MissingPackage {
                        package_name: package_name.clone(),
                        task_id: info.clone(),
                    })?;

            let package_task_event =
                PackageTaskEventBuilder::new(info.package(), info.task()).with_parent(telemetry);
            let command = workspace_info
                .package_json
                .scripts
                .get(info.task())
                .cloned();

            match command {
                Some(cmd) if info.package() == ROOT_PKG_NAME && turbo_regex().is_match(&cmd) => {
                    package_task_event.track_error(TrackedErrors::RecursiveError);
                    return Err(Error::RecursiveTurbo {
                        task_name: info.to_string(),
                        command: cmd.to_string(),
                    });
                }
                _ => (),
            }

            let task_definition = engine
                .task_definition(&info)
                .ok_or(Error::MissingDefinition)?;

            let task_env_mode = match self.global_env_mode {
                // Task env mode is only independent when global env mode is `infer`.
                EnvMode::Infer if task_definition.pass_through_env.is_some() => {
                    ResolvedEnvMode::Strict
                }
                // If we're in infer mode we have just detected non-usage of strict env vars.
                // But our behavior's actual meaning of this state is `loose`.
                EnvMode::Infer => ResolvedEnvMode::Loose,
                // Otherwise we just use the global env mode.
                EnvMode::Strict => ResolvedEnvMode::Strict,
                EnvMode::Loose => ResolvedEnvMode::Loose,
            };
            package_task_event.track_env_mode(&task_env_mode.to_string());

            let dependency_set = engine.dependencies(&info).ok_or(Error::MissingDefinition)?;

            let task_hash_telemetry = package_task_event.child();
            let task_hash = self.task_hasher.calculate_task_hash(
                &info,
                task_definition,
                task_env_mode,
                workspace_info,
                dependency_set,
                task_hash_telemetry,
            )?;

            debug!("task {} hash is {}", info, task_hash);
            // We do this calculation earlier than we do in Go due to the `task_hasher`
            // being !Send. In the future we can look at doing this right before
            // task execution instead.
            let execution_env =
                self.task_hasher
                    .env(&info, task_env_mode, task_definition, &self.global_env)?;

            let task_cache = self.run_cache.task_cache(
                task_definition,
                workspace_info,
                info.clone(),
                &task_hash,
            );

            // Drop to avoid holding the span across an await
            drop(_enter);

            // here is where we do the logic split
            match self.dry {
                true => {
                    let dry_run_exec_context =
                        factory.dry_run_exec_context(info.clone(), task_cache);
                    let tracker = self.run_tracker.track_task(info.into_owned());
                    tasks.push(tokio::spawn(async move {
                        dry_run_exec_context.execute_dry_run(tracker).await;
                    }));
                }
                false => {
                    // TODO(gsoltis): if/when we fix https://github.com/vercel/turbo/issues/937
                    // the following block should never get hit. In the meantime, keep it after
                    // hashing so that downstream tasks can count on the hash existing
                    //
                    // bail if the script doesn't exist or is empty
                    if command.map_or(true, |s| s.is_empty()) {
                        continue;
                    }

                    let workspace_directory = self.repo_root.resolve(workspace_info.package_path());

                    let persistent = task_definition.persistent;
                    let mut exec_context = factory.exec_context(
                        info.clone(),
                        task_hash,
                        task_cache,
                        workspace_directory,
                        execution_env,
                        persistent,
                        self.task_access.clone(),
                    );

                    let vendor_behavior =
                        Vendor::infer().and_then(|vendor| vendor.behavior.as_ref());

                    let output_client = self.output_client(&info, vendor_behavior);
                    let tracker = self.run_tracker.track_task(info.clone().into_owned());
                    let spaces_client = self.run_tracker.spaces_task_client();
                    let parent_span = Span::current();
                    let execution_telemetry = package_task_event.child();

                    tasks.push(tokio::spawn(async move {
                        exec_context
                            .execute(
                                parent_span.id(),
                                tracker,
                                output_client,
                                callback,
                                spaces_client,
                                &execution_telemetry,
                            )
                            .await;
                    }));
                }
            }
        }

        // Wait for the engine task to finish and for all of our tasks to finish
        engine_handle.await.expect("engine execution panicked")?;
        // This will poll the futures until they are all completed
        while let Some(result) = tasks.next().await {
            result.expect("task executor panicked");
        }
        drop(factory);

        // Write out the traced-config.json file if we have one
        self.task_access.save().await;

        let errors = Arc::into_inner(errors)
            .expect("only one strong reference to errors should remain")
            .into_inner()
            .expect("mutex poisoned");

        Ok(errors)
    }

    /// Finishes visiting the tasks, creates the run summary, and either
    /// prints, saves, or sends it to spaces.
    #[tracing::instrument(skip(
        self,
        packages,
        global_hash_inputs,
        engine,
        env_at_execution_start
    ))]
    pub(crate) async fn finish(
        self,
        exit_code: i32,
        packages: HashSet<PackageName>,
        global_hash_inputs: GlobalHashableInputs<'_>,
        engine: &Engine,
        env_at_execution_start: &EnvironmentVariableMap,
        pkg_inference_root: Option<&AnchoredSystemPath>,
    ) -> Result<(), Error> {
        let Self {
            package_graph,
            ui,
            run_opts,
            repo_root,
            global_env_mode,
            task_hasher,
            ..
        } = self;

        let global_hash_summary = GlobalHashSummary::try_from(global_hash_inputs)?;

        Ok(self
            .run_tracker
            .finish(
                exit_code,
                &package_graph,
                ui,
                repo_root,
                pkg_inference_root,
                run_opts,
                packages,
                global_hash_summary,
                global_env_mode,
                engine,
                task_hasher.task_hash_tracker(),
                env_at_execution_start,
            )
            .await?)
    }

    fn sink(run_opts: &RunOpts, silent: bool) -> OutputSink<StdWriter> {
        let (out, err) = if silent {
            (std::io::sink().into(), std::io::sink().into())
        } else if run_opts.should_redirect_stderr_to_stdout() {
            (std::io::stdout().into(), std::io::stdout().into())
        } else {
            (std::io::stdout().into(), std::io::stderr().into())
        };
        OutputSink::new(out, err)
    }

    fn output_client(
        &self,
        task_id: &TaskId,
        vendor_behavior: Option<&VendorBehavior>,
    ) -> OutputClient<impl std::io::Write> {
        let behavior = match self.run_opts.log_order {
            crate::opts::ResolvedLogOrder::Stream if self.run_tracker.spaces_enabled() => {
                turborepo_ui::OutputClientBehavior::InMemoryBuffer
            }
            crate::opts::ResolvedLogOrder::Stream => {
                turborepo_ui::OutputClientBehavior::Passthrough
            }
            crate::opts::ResolvedLogOrder::Grouped => turborepo_ui::OutputClientBehavior::Grouped,
        };

        let mut logger = self.sink.logger(behavior);
        if let Some(vendor_behavior) = vendor_behavior {
            let group_name = if self.run_opts.single_package {
                task_id.task().to_string()
            } else {
                format!("{}:{}", task_id.package(), task_id.task())
            };
            let (header, footer) = (
                (vendor_behavior.group_prefix)(&group_name),
                (vendor_behavior.group_suffix)(&group_name),
            );
            logger.with_header_footer(Some(header), Some(footer));

            let (error_header, error_footer) = (
                vendor_behavior.error_group_prefix.map(|f| f(&group_name)),
                vendor_behavior.error_group_suffix.map(|f| f(&group_name)),
            );
            logger.with_error_header_footer(error_header, error_footer);
        }
        logger
    }

    fn prefix<'b>(&self, task_id: &'b TaskId) -> Cow<'b, str> {
        match self.run_opts.log_prefix {
            crate::opts::ResolvedLogPrefix::Task if self.run_opts.single_package => {
                task_id.task().into()
            }
            crate::opts::ResolvedLogPrefix::Task => {
                format!("{}:{}", task_id.package(), task_id.task()).into()
            }
            crate::opts::ResolvedLogPrefix::None => "".into(),
        }
    }

    // Task ID as displayed in error messages
    fn display_task_id(&self, task_id: &TaskId) -> String {
        match self.run_opts.single_package {
            true => task_id.task().to_string(),
            false => task_id.to_string(),
        }
    }

    fn prefixed_ui<W: Write>(
        ui: UI,
        is_github_actions: bool,
        client: &OutputClient<W>,
        prefix: StyledObject<String>,
    ) -> PrefixedUI<OutputWriter<'_, W>> {
        let mut prefixed_ui = PrefixedUI::new(ui, client.stdout(), client.stderr())
            .with_output_prefix(prefix.clone())
            // TODO: we can probably come up with a more ergonomic way to achieve this
            .with_error_prefix(
                Style::new().apply_to(format!("{}ERROR: ", ui.apply(prefix.clone()))),
            )
            .with_warn_prefix(prefix);
        if is_github_actions {
            prefixed_ui = prefixed_ui
                .with_error_prefix(Style::new().apply_to("[ERROR] ".to_string()))
                .with_warn_prefix(Style::new().apply_to("[WARN] ".to_string()));
        }
        prefixed_ui
    }

    /// Only used for the hashing comparison between Rust and Go. After port,
    /// should delete
    pub fn into_task_hash_tracker(self) -> TaskHashTrackerState {
        self.task_hasher.into_task_hash_tracker_state()
    }

    pub fn dry_run(&mut self) {
        self.dry = true;
    }
}

// A tiny enum that allows us to use the same type for stdout and stderr without
// the use of Box<dyn Write>
enum StdWriter {
    Out(std::io::Stdout),
    Err(std::io::Stderr),
    Null(std::io::Sink),
}

impl StdWriter {
    fn writer(&mut self) -> &mut dyn std::io::Write {
        match self {
            StdWriter::Out(out) => out,
            StdWriter::Err(err) => err,
            StdWriter::Null(null) => null,
        }
    }
}

impl From<std::io::Stdout> for StdWriter {
    fn from(value: std::io::Stdout) -> Self {
        Self::Out(value)
    }
}

impl From<std::io::Stderr> for StdWriter {
    fn from(value: std::io::Stderr) -> Self {
        Self::Err(value)
    }
}

impl From<std::io::Sink> for StdWriter {
    fn from(value: std::io::Sink) -> Self {
        Self::Null(value)
    }
}

impl std::io::Write for StdWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer().flush()
    }
}

fn turbo_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)turbo(?:$|\s)").unwrap())
}

// Error that comes from the execution of the task
#[derive(Debug, thiserror::Error, Clone)]
#[error("{task_id}: {cause}")]
pub struct TaskError {
    task_id: String,
    cause: TaskErrorCause,
}

#[derive(Debug, thiserror::Error, Clone)]
enum TaskErrorCause {
    #[error("unable to spawn child process: {msg}")]
    // We eagerly serialize this in order to allow us to implement clone
    Spawn { msg: String },
    #[error("command {command} exited ({exit_code})")]
    Exit { command: String, exit_code: i32 },
}

impl TaskError {
    pub fn exit_code(&self) -> Option<i32> {
        match self.cause {
            TaskErrorCause::Exit { exit_code, .. } => Some(exit_code),
            _ => None,
        }
    }

    fn from_spawn(task_id: String, err: std::io::Error) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Spawn {
                msg: err.to_string(),
            },
        }
    }

    fn from_execution(task_id: String, command: String, exit_code: i32) -> Self {
        Self {
            task_id,
            cause: TaskErrorCause::Exit { command, exit_code },
        }
    }
}

impl TaskErrorCause {
    fn from_spawn(err: std::io::Error) -> Self {
        TaskErrorCause::Spawn {
            msg: err.to_string(),
        }
    }

    fn from_execution(command: String, exit_code: i32) -> Self {
        TaskErrorCause::Exit { command, exit_code }
    }
}

struct ExecContextFactory<'a> {
    visitor: &'a Visitor<'a>,
    errors: Arc<Mutex<Vec<TaskError>>>,
    manager: ProcessManager,
    engine: &'a Arc<Engine>,
}

impl<'a> ExecContextFactory<'a> {
    pub fn new(
        visitor: &'a Visitor,
        errors: Arc<Mutex<Vec<TaskError>>>,
        manager: ProcessManager,
        engine: &'a Arc<Engine>,
    ) -> Self {
        Self {
            visitor,
            errors,
            manager,
            engine,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn exec_context(
        &self,
        task_id: TaskId<'static>,
        task_hash: String,
        task_cache: TaskCache,
        workspace_directory: AbsoluteSystemPathBuf,
        execution_env: EnvironmentVariableMap,
        persistent: bool,
        task_access: TaskAccess,
    ) -> ExecContext {
        let task_id_for_display = self.visitor.display_task_id(&task_id);
        let pass_through_args = self.visitor.run_opts.args_for_task(&task_id);
        ExecContext {
            engine: self.engine.clone(),
            ui: self.visitor.ui,
            is_github_actions: self.visitor.run_opts.is_github_actions,
            pretty_prefix: self
                .visitor
                .color_cache
                .prefix_with_color(&task_hash, &self.visitor.prefix(&task_id)),
            task_id,
            task_id_for_display,
            task_cache,
            hash_tracker: self.visitor.task_hasher.task_hash_tracker(),
            package_manager: *self.visitor.package_graph.package_manager(),
            workspace_directory,
            manager: self.manager.clone(),
            task_hash,
            execution_env,
            continue_on_error: self.visitor.run_opts.continue_on_error,
            pass_through_args,
            errors: self.errors.clone(),
            persistent,
            task_access,
        }
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
}

struct ExecContext {
    engine: Arc<Engine>,
    ui: UI,
    is_github_actions: bool,
    pretty_prefix: StyledObject<String>,
    task_id: TaskId<'static>,
    task_id_for_display: String,
    task_cache: TaskCache,
    hash_tracker: TaskHashTracker,
    package_manager: PackageManager,
    workspace_directory: AbsoluteSystemPathBuf,
    manager: ProcessManager,
    task_hash: String,
    execution_env: EnvironmentVariableMap,
    continue_on_error: bool,
    pass_through_args: Option<Vec<String>>,
    errors: Arc<Mutex<Vec<TaskError>>>,
    persistent: bool,
    task_access: TaskAccess,
}

enum ExecOutcome {
    // All operations during execution succeeded
    Success(SuccessOutcome),
    // An internal error that indicates a shutdown should be performed
    Internal,
    // An error with the task execution
    Task {
        exit_code: Option<i32>,
        message: String,
    },
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
        output_client: OutputClient<impl std::io::Write>,
        callback: oneshot::Sender<Result<(), StopExecution>>,
        spaces_client: Option<SpacesTaskClient>,
        telemetry: &PackageTaskEventBuilder,
    ) {
        let tracker = tracker.start().await;
        let span = tracing::debug_span!("execute_task", task = %self.task_id.task());
        span.follows_from(parent_span_id);
        let mut result = self
            .execute_inner(&output_client, telemetry)
            .instrument(span)
            .await;

        // If the task resulted in an error, do not group in order to better highlight
        // the error.
        let is_error = matches!(result, ExecOutcome::Task { .. });
        let logs = match output_client.finish(is_error) {
            Ok(logs) => logs,
            Err(e) => {
                telemetry.track_error(TrackedErrors::DaemonFailedToMarkOutputsAsCached);
                error!("unable to flush output client: {e}");
                result = ExecOutcome::Internal;
                None
            }
        };

        match result {
            ExecOutcome::Success(outcome) => {
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
            ExecOutcome::Internal => {
                tracker.cancel();
                callback.send(Err(StopExecution)).ok();
                self.manager.stop().await;
            }
            ExecOutcome::Task { exit_code, message } => {
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
        }
    }

    async fn execute_inner(
        &mut self,
        output_client: &OutputClient<impl std::io::Write>,
        telemetry: &PackageTaskEventBuilder,
    ) -> ExecOutcome {
        let task_start = Instant::now();
        let mut prefixed_ui = Visitor::prefixed_ui(
            self.ui,
            self.is_github_actions,
            output_client,
            self.pretty_prefix.clone(),
        );

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
                return ExecOutcome::Success(SuccessOutcome::CacheHit);
            }
            Ok(None) => (),
            Err(e) => {
                telemetry.track_error(TrackedErrors::ErrorFetchingFromCache);
                prefixed_ui.error(format!("error fetching from cache: {e}"));
            }
        }

        let Ok(package_manager_binary) = which(self.package_manager.command()) else {
            return ExecOutcome::Internal;
        };

        let mut cmd = Command::new(package_manager_binary);
        let mut args = vec!["run".to_string(), self.task_id.task().to_string()];
        if let Some(pass_through_args) = &self.pass_through_args {
            args.extend(
                self.package_manager
                    .arg_separator(pass_through_args.as_slice())
                    .map(|s| s.to_string()),
            );
            args.extend(pass_through_args.iter().cloned());
        }
        cmd.args(args);
        cmd.current_dir(self.workspace_directory.clone());

        // We clear the env before populating it with variables we expect
        cmd.env_clear();
        cmd.envs(self.execution_env.iter());
        // Always last to make sure it overwrites any user configured env var.
        cmd.env("TURBO_HASH", &self.task_hash);
        // enable task access tracing

        // set the trace file env var - frameworks that support this can use it to
        // write out a trace file that we will use to automatically cache the task
        if self.task_access.is_enabled() {
            let (task_access_trace_key, trace_file) = self.task_access.get_env_var(&self.task_hash);
            cmd.env(task_access_trace_key, trace_file.to_string());
        }

        // Many persistent tasks if started hooked up to a pseudoterminal
        // will shut down if stdin is closed, so we open it even if we don't pass
        // anything to it.
        if self.persistent {
            cmd.open_stdin();
        }

        let mut stdout_writer = match self
            .task_cache
            .output_writer(self.pretty_prefix.clone(), output_client.stdout())
        {
            Ok(w) => w,
            Err(e) => {
                telemetry.track_error(TrackedErrors::FailedToCaptureOutputs);
                error!("failed to capture outputs for \"{}\": {e}", self.task_id);
                return ExecOutcome::Internal;
            }
        };

        let mut process = match self.manager.spawn(cmd, Duration::from_millis(500)) {
            Some(Ok(child)) => child,
            // Turbo was unable to spawn a process
            Some(Err(e)) => {
                // Note: we actually failed to spawn, but this matches the Go output
                prefixed_ui.error(format!("command finished with error: {e}"));
                let error_string = e.to_string();
                self.errors
                    .lock()
                    .expect("lock poisoned")
                    .push(TaskError::from_spawn(self.task_id_for_display.clone(), e));
                return ExecOutcome::Task {
                    exit_code: None,
                    message: error_string,
                };
            }
            // Turbo is shutting down
            None => {
                return ExecOutcome::Internal;
            }
        };

        let exit_status = match process.wait_with_piped_outputs(&mut stdout_writer).await {
            Ok(Some(exit_status)) => exit_status,
            Err(e) => {
                telemetry.track_error(TrackedErrors::FailedToPipeOutputs);
                error!("unable to pipe outputs from command: {e}");
                return ExecOutcome::Internal;
            }
            Ok(None) => {
                // TODO: how can this happen? we only update the
                // exit status with Some and it is only initialized with
                // None. Is it still running?
                telemetry.track_error(TrackedErrors::UnknownChildExit);
                error!("unable to determine why child exited");
                return ExecOutcome::Internal;
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
                    } else {
                        // If no errors, update hash tracker with expanded outputs
                        self.hash_tracker.insert_expanded_outputs(
                            self.task_id.clone(),
                            self.task_cache.expanded_outputs().to_vec(),
                        );
                    }
                }

                // Return success outcome
                ExecOutcome::Success(SuccessOutcome::Run)
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
                    prefixed_ui.error(format!("command finished with error: {error}"));
                }
                self.errors.lock().expect("lock poisoned").push(TaskError {
                    task_id: self.task_id_for_display.clone(),
                    cause: error,
                });
                ExecOutcome::Task {
                    exit_code: Some(code),
                    message,
                }
            }
            // All of these indicate a failure where we don't know how to recover
            ChildExit::Finished(None)
            | ChildExit::Killed
            | ChildExit::KilledExternal
            | ChildExit::Failed => ExecOutcome::Internal,
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

struct DryRunExecContext {
    task_id: TaskId<'static>,
    task_cache: TaskCache,
    hash_tracker: TaskHashTracker,
}

impl DryRunExecContext {
    pub async fn execute_dry_run(&self, tracker: TaskTracker<()>) {
        // may also need to do framework & command stuff?
        if let Ok(Some(status)) = self.task_cache.exists().await {
            self.hash_tracker
                .insert_cache_status(self.task_id.clone(), status);
        }
        tracker.dry_run().await;
    }
}
