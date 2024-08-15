use std::{
    borrow::Cow,
    collections::HashSet,
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use console::{Style, StyledObject};
use either::Either;
use futures::{stream::FuturesUnordered, StreamExt};
use itertools::Itertools;
use miette::{Diagnostic, NamedSource, SourceSpan};
use regex::Regex;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, Instrument, Span};
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath};
use turborepo_ci::{Vendor, VendorBehavior};
use turborepo_env::EnvironmentVariableMap;
use turborepo_repository::{
    package_graph::{PackageGraph, PackageName, ROOT_PKG_NAME},
    package_manager::PackageManager,
};
use turborepo_telemetry::events::{
    generic::GenericEventBuilder, task::PackageTaskEventBuilder, EventBuilder, TrackedErrors,
};
use turborepo_ui::{
    tui::{self, event::CacheResult, AppSender, TuiTask},
    ColorConfig, ColorSelector, OutputClient, OutputSink, OutputWriter, PrefixedUI,
};
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
        CacheOutput, RunCache, TaskCache,
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
    task_access: &'a TaskAccess,
    sink: OutputSink<StdWriter>,
    task_hasher: TaskHasher<'a>,
    color_config: ColorConfig,
    experimental_ui_sender: Option<AppSender>,
    is_watch: bool,
}

#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("cannot find package {package_name} for task {task_id}")]
    MissingPackage {
        package_name: PackageName,
        task_id: TaskId<'static>,
    },
    #[error(
        "root task {task_name} ({command}) looks like it invokes turbo and might cause a loop"
    )]
    RecursiveTurbo {
        task_name: String,
        command: String,
        #[label("task found here")]
        span: Option<SourceSpan>,
        #[source_code]
        text: NamedSource,
    },
    #[error("Could not find definition for task")]
    MissingDefinition,
    #[error("error while executing engine: {0}")]
    Engine(#[from] crate::engine::ExecuteError),
    #[error(transparent)]
    TaskHash(#[from] task_hash::Error),
    #[error(transparent)]
    RunSummary(#[from] summary::Error),
    #[error("internal errors encountered: {0}")]
    InternalErrors(String),
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
        task_access: &'a TaskAccess,
        run_opts: &'a RunOpts,
        package_inputs_hashes: PackageInputsHashes,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        global_env_mode: EnvMode,
        color_config: ColorConfig,
        manager: ProcessManager,
        repo_root: &'a AbsoluteSystemPath,
        global_env: EnvironmentVariableMap,
        experimental_ui_sender: Option<AppSender>,
        is_watch: bool,
    ) -> Self {
        let task_hasher = TaskHasher::new(
            package_inputs_hashes,
            run_opts,
            env_at_execution_start,
            global_hash,
        );

        let sink = Self::sink(run_opts);
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
            color_config,
            global_env,
            experimental_ui_sender,
            is_watch,
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn visit(
        &self,
        engine: Arc<Engine>,
        telemetry: &GenericEventBuilder,
    ) -> Result<Vec<TaskError>, Error> {
        for task in engine.tasks().sorted() {
            self.color_cache.color_for_key(&task.to_string());
        }

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
                    let (span, text) = cmd.span_and_text("package.json");
                    return Err(Error::RecursiveTurbo {
                        task_name: info.to_string(),
                        command: cmd.to_string(),
                        span,
                        text,
                    });
                }
                _ => (),
            }

            let task_definition = engine
                .task_definition(&info)
                .ok_or(Error::MissingDefinition)?;

            let task_env_mode = self.global_env_mode;
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
                        dry_run_exec_context.execute_dry_run(tracker).await
                    }));
                }
                false => {
                    // TODO(gsoltis): if/when we fix https://github.com/vercel/turborepo/issues/937
                    // the following block should never get hit. In the meantime, keep it after
                    // hashing so that downstream tasks can count on the hash existing
                    //
                    // bail if the script doesn't exist or is empty
                    if command.map_or(true, |s| s.is_empty()) {
                        continue;
                    }

                    let workspace_directory = self.repo_root.resolve(workspace_info.package_path());

                    let takes_input = task_definition.interactive || task_definition.persistent;
                    let mut exec_context = factory.exec_context(
                        info.clone(),
                        task_hash,
                        task_cache,
                        workspace_directory,
                        execution_env,
                        takes_input,
                        self.task_access.clone(),
                    );

                    let vendor_behavior =
                        Vendor::infer().and_then(|vendor| vendor.behavior.as_ref());

                    let output_client = if let Some(handle) = &self.experimental_ui_sender {
                        TaskOutput::UI(handle.task(info.to_string()))
                    } else {
                        TaskOutput::Direct(self.output_client(&info, vendor_behavior))
                    };
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
                            .await
                    }));
                }
            }
        }

        // Wait for the engine task to finish and for all of our tasks to finish
        engine_handle.await.expect("engine execution panicked")?;
        // This will poll the futures until they are all completed
        let mut internal_errors = Vec::new();
        while let Some(result) = tasks.next().await {
            if let Err(e) = result.unwrap_or_else(|e| panic!("task executor panicked: {e}")) {
                internal_errors.push(e);
            }
        }
        drop(factory);

        if !self.is_watch {
            if let Some(handle) = &self.experimental_ui_sender {
                handle.stop();
            }
        }

        if !internal_errors.is_empty() {
            return Err(Error::InternalErrors(
                internal_errors.into_iter().map(|e| e.to_string()).join(","),
            ));
        }

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

    #[allow(clippy::too_many_arguments)]
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
        packages: &HashSet<PackageName>,
        global_hash_inputs: GlobalHashableInputs<'_>,
        engine: &Engine,
        env_at_execution_start: &EnvironmentVariableMap,
        pkg_inference_root: Option<&AnchoredSystemPath>,
    ) -> Result<(), Error> {
        let Self {
            package_graph,
            color_config: ui,
            run_opts,
            repo_root,
            global_env_mode,
            task_hasher,
            is_watch,
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
                is_watch,
            )
            .await?)
    }

    fn sink(run_opts: &RunOpts) -> OutputSink<StdWriter> {
        let (out, err) = if run_opts.should_redirect_stderr_to_stdout() {
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

            let header_factory = (vendor_behavior.group_prefix)(group_name.to_owned());
            let footer_factory = (vendor_behavior.group_suffix)(group_name.to_owned());

            logger.with_header_footer(Some(header_factory), Some(footer_factory));

            let (error_header, error_footer) = (
                vendor_behavior
                    .error_group_prefix
                    .map(|f| f(group_name.to_owned())),
                vendor_behavior
                    .error_group_suffix
                    .map(|f| f(group_name.to_owned())),
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
        color_config: ColorConfig,
        is_github_actions: bool,
        stdout: W,
        stderr: W,
        prefix: StyledObject<String>,
    ) -> PrefixedUI<W> {
        let mut prefixed_ui = PrefixedUI::new(color_config, stdout, stderr)
            .with_output_prefix(prefix.clone())
            // TODO: we can probably come up with a more ergonomic way to achieve this
            .with_error_prefix(
                Style::new().apply_to(format!("{}ERROR: ", color_config.apply(prefix.clone()))),
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
        // No need to start a TUI on dry run
        self.experimental_ui_sender = None;
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

/// Small wrapper over our two output types that defines a shared interface for
/// interacting with them.
enum TaskOutput<W> {
    Direct(OutputClient<W>),
    UI(tui::TuiTask),
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
    #[error("turbo has internal error processing task")]
    Internal,
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
        takes_input: bool,
        task_access: TaskAccess,
    ) -> ExecContext {
        let task_id_for_display = self.visitor.display_task_id(&task_id);
        let pass_through_args = self.visitor.run_opts.args_for_task(&task_id);
        let task_id_string = &task_id.to_string();
        ExecContext {
            engine: self.engine.clone(),
            color_config: self.visitor.color_config,
            experimental_ui: self.visitor.experimental_ui_sender.is_some(),
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
            workspace_directory,
            manager: self.manager.clone(),
            task_hash,
            execution_env,
            continue_on_error: self.visitor.run_opts.continue_on_error,
            pass_through_args,
            errors: self.errors.clone(),
            takes_input,
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
    color_config: ColorConfig,
    experimental_ui: bool,
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
    takes_input: bool,
    task_access: TaskAccess,
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
        output_client: TaskOutput<impl std::io::Write>,
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
        output_client: &TaskOutput<impl std::io::Write>,
        telemetry: &PackageTaskEventBuilder,
    ) -> Result<ExecOutcome, InternalError> {
        let task_start = Instant::now();
        let mut prefixed_ui = self.prefixed_ui(output_client);

        if self.experimental_ui {
            if let TaskOutput::UI(task) = output_client {
                let output_logs = self.task_cache.output_logs().into();
                task.start(output_logs);
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

        let package_manager_binary = which(self.package_manager.command())?;

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

        // Allow downstream tools to detect if the task is being ran with TUI
        if self.experimental_ui {
            cmd.env("TURBO_IS_TUI", "true");
        }

        // enable task access tracing

        // set the trace file env var - frameworks that support this can use it to
        // write out a trace file that we will use to automatically cache the task
        if self.task_access.is_enabled() {
            let (task_access_trace_key, trace_file) = self.task_access.get_env_var(&self.task_hash);
            cmd.env(task_access_trace_key, trace_file.to_string());
        }

        cmd.open_stdin();

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

        if self.experimental_ui && self.takes_input {
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
            .map_err(|e| {
                telemetry.track_error(TrackedErrors::FailedToCaptureOutputs);
                e
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
                self.errors.lock().expect("lock poisoned").push(TaskError {
                    task_id: self.task_id_for_display.clone(),
                    cause: error,
                });
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

struct DryRunExecContext {
    task_id: TaskId<'static>,
    task_cache: TaskCache,
    hash_tracker: TaskHashTracker,
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

/// Struct for displaying information about task's cache
enum TaskCacheOutput<W> {
    Direct(PrefixedUI<W>),
    UI(TuiTask),
}

impl<W: Write> TaskCacheOutput<W> {
    fn task_writer(&mut self) -> Either<turborepo_ui::PrefixedWriter<&mut W>, TuiTask> {
        match self {
            TaskCacheOutput::Direct(prefixed) => Either::Left(prefixed.output_prefixed_writer()),
            TaskCacheOutput::UI(task) => Either::Right(task.clone()),
        }
    }

    fn warn(&mut self, message: impl std::fmt::Display) {
        match self {
            TaskCacheOutput::Direct(prefixed) => prefixed.warn(message),
            TaskCacheOutput::UI(task) => {
                let _ = write!(task, "\r\n{message}\r\n");
            }
        }
    }
}

impl<W: Write> CacheOutput for TaskCacheOutput<W> {
    fn status(&mut self, message: &str, result: CacheResult) {
        match self {
            TaskCacheOutput::Direct(direct) => direct.output(message),
            TaskCacheOutput::UI(task) => task.status(message, result),
        }
    }

    fn error(&mut self, message: &str) {
        match self {
            TaskCacheOutput::Direct(prefixed) => prefixed.error(message),
            TaskCacheOutput::UI(task) => {
                let _ = write!(task, "{message}\r\n");
            }
        }
    }

    fn replay_logs(&mut self, log_file: &AbsoluteSystemPath) -> Result<(), turborepo_ui::Error> {
        match self {
            TaskCacheOutput::Direct(direct) => {
                let writer = direct.output_prefixed_writer();
                turborepo_ui::replay_logs(writer, log_file)
            }
            TaskCacheOutput::UI(task) => turborepo_ui::replay_logs(task, log_file),
        }
    }
}

/// Struct for displaying information about task
impl<W: Write> TaskOutput<W> {
    pub fn finish(self, use_error: bool, is_cache_hit: bool) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            TaskOutput::Direct(client) => client.finish(use_error),
            TaskOutput::UI(client) if use_error => Ok(Some(client.failed())),
            TaskOutput::UI(client) => Ok(Some(client.succeeded(is_cache_hit))),
        }
    }

    pub fn stdout(&self) -> Either<OutputWriter<W>, TuiTask> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stdout()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }

    pub fn stderr(&self) -> Either<OutputWriter<W>, TuiTask> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stderr()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }

    pub fn task_logs(&self) -> Either<OutputWriter<W>, TuiTask> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stdout()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }
}
