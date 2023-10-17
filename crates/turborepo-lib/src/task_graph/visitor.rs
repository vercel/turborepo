use std::{
    borrow::Cow,
    io::Write,
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use console::{Style, StyledObject};
use futures::{stream::FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::{process::Command, sync::mpsc};
use tracing::{debug, error, Span};
use turbopath::AbsoluteSystemPath;
use turborepo_env::{EnvironmentVariableMap, ResolvedEnvMode};
use turborepo_ui::{ColorSelector, OutputClient, OutputSink, OutputWriter, PrefixedUI, UI};

use crate::{
    cli::EnvMode,
    engine::{Engine, ExecutionOptions, StopExecution},
    opts::Opts,
    package_graph::{PackageGraph, WorkspaceName},
    process::{ChildExit, ProcessManager},
    run::{
        task_id::{self, TaskId},
        RunCache,
    },
    task_hash::{self, PackageInputsHashes, TaskHashTrackerState, TaskHasher},
};

// This holds the whole world
pub struct Visitor<'a> {
    run_cache: Arc<RunCache>,
    package_graph: Arc<PackageGraph>,
    opts: &'a Opts<'a>,
    task_hasher: TaskHasher<'a>,
    global_env_mode: EnvMode,
    sink: OutputSink<StdWriter>,
    color_cache: ColorSelector,
    ui: UI,
    manager: ProcessManager,
    repo_root: &'a AbsoluteSystemPath,
    dry: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot find package {package_name} for task {task_id}")]
    MissingPackage {
        package_name: WorkspaceName,
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
}

impl<'a> Visitor<'a> {
    // Disabling this lint until we stop adding state to the visitor.
    // Once we have the full picture we will go about grouping these pieces of data
    // together
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        package_graph: Arc<PackageGraph>,
        run_cache: Arc<RunCache>,
        opts: &'a Opts,
        package_inputs_hashes: PackageInputsHashes,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        global_env_mode: EnvMode,
        ui: UI,
        silent: bool,
        manager: ProcessManager,
        repo_root: &'a AbsoluteSystemPath,
    ) -> Self {
        let task_hasher = TaskHasher::new(
            package_inputs_hashes,
            opts,
            env_at_execution_start,
            global_hash,
        );
        let sink = Self::sink(opts, silent);
        let color_cache = ColorSelector::default();

        Self {
            run_cache,
            package_graph,
            opts,
            task_hasher,
            global_env_mode,
            sink,
            color_cache,
            ui,
            manager,
            repo_root,
            dry: false,
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn visit(&self, engine: Arc<Engine>) -> Result<Vec<TaskError>, Error> {
        let concurrency = self.opts.run_opts.concurrency as usize;
        let (node_sender, mut node_stream) = mpsc::channel(concurrency);
        let engine_handle = {
            let engine = engine.clone();
            tokio::spawn(engine.execute(ExecutionOptions::new(false, concurrency), node_sender))
        };
        let mut tasks = FuturesUnordered::new();
        let errors = Arc::new(Mutex::new(Vec::new()));

        let span = Span::current();

        while let Some(message) = node_stream.recv().await {
            let span = tracing::debug_span!(parent: &span, "queue_task", task = %message.info);
            let _enter = span.enter();
            let crate::engine::Message { info, callback } = message;
            let is_github_actions = self.opts.run_opts.is_github_actions;
            let package_name = WorkspaceName::from(info.package());

            let workspace_dir =
                self.package_graph
                    .workspace_dir(&package_name)
                    .ok_or_else(|| Error::MissingPackage {
                        package_name: package_name.clone(),
                        task_id: info.clone(),
                    })?;
            let workspace_info = self
                .package_graph
                .workspace_info(&package_name)
                .ok_or_else(|| Error::MissingPackage {
                    package_name: package_name.clone(),
                    task_id: info.clone(),
                })?;

            let command = workspace_info
                .package_json
                .scripts
                .get(info.task())
                .cloned();

            match command {
                Some(cmd)
                    if info.package() == task_id::ROOT_PKG_NAME && turbo_regex().is_match(&cmd) =>
                {
                    return Err(Error::RecursiveTurbo {
                        task_name: info.to_string(),
                        command: cmd.to_string(),
                    })
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

            let dependency_set = engine.dependencies(&info).ok_or(Error::MissingDefinition)?;

            let task_hash = self.task_hasher.calculate_task_hash(
                &info,
                task_definition,
                task_env_mode,
                workspace_info,
                dependency_set,
            )?;

            debug!("task {} hash is {}", info, task_hash);
            if self.dry {
                callback.send(Ok(())).ok();
                continue;
            }

            let task_cache =
                self.run_cache
                    .task_cache(task_definition, workspace_dir, info.clone(), &task_hash);

            // TODO(gsoltis): if/when we fix https://github.com/vercel/turbo/issues/937
            // the following block should never get hit. In the meantime, keep it after
            // hashing so that downstream tasks can count on the hash existing
            //
            // bail if the script doesn't exist
            let Some(_command) = command else { continue };

            let output_client = self.output_client();
            let continue_on_error = self.opts.run_opts.continue_on_error;
            let prefix = self.prefix(&info);
            let pretty_prefix = self.color_cache.prefix_with_color(&task_hash, &prefix);
            let ui = self.ui;
            let manager = self.manager.clone();
            let package_manager = self.package_graph.package_manager().clone();
            let workspace_directory = self.repo_root.resolve(workspace_dir);
            let errors = errors.clone();
            let task_id_for_display = self.display_task_id(&info);
            let hash_tracker = self.task_hasher.task_hash_tracker();

            let parent_span = Span::current();
            tasks.push(tokio::spawn(async move {
                let span = tracing::debug_span!("execute_task", task = %info.task());
                span.follows_from(parent_span.id());
                let _enter = span.enter();

                let task_id = info;
                let mut task_cache = task_cache;
                let mut prefixed_ui =
                    Self::prefixed_ui(ui, is_github_actions, &output_client, pretty_prefix.clone());

                match task_cache.restore_outputs(&mut prefixed_ui).await {
                    Ok(_hit) => {
                        // we need to set expanded outputs
                        hash_tracker.insert_expanded_outputs(
                            task_id,
                            task_cache.expanded_outputs().to_vec(),
                        );
                        callback.send(Ok(())).ok();
                        return;
                    }
                    Err(e) if e.is_cache_miss() => (),
                    Err(e) => {
                        prefixed_ui.error(format!("error fetching from cache: {e}"));
                    }
                }

                let mut cmd = Command::new(package_manager.to_string());
                cmd.args(["run", task_id.task()]);
                cmd.current_dir(workspace_directory.as_path());
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());

                let mut stdout_writer =
                    match task_cache.output_writer(pretty_prefix.clone(), output_client.stdout()) {
                        Ok(w) => w,
                        Err(e) => {
                            error!("failed to capture outputs for \"{task_id}\": {e}");
                            callback.send(Err(StopExecution)).ok();
                            return;
                        }
                    };

                let mut process = match manager.spawn(cmd, Duration::from_millis(500)) {
                    Some(Ok(child)) => child,
                    // Turbo was unable to spawn a process
                    Some(Err(e)) => {
                        // Note: we actually failed to spawn, but this matches the Go output
                        prefixed_ui.error(format!("command finished with error: {e}"));
                        errors
                            .lock()
                            .expect("lock poisoned")
                            .push(TaskError::from_spawn(task_id_for_display.clone(), e));
                        callback
                            .send(if continue_on_error {
                                Ok(())
                            } else {
                                Err(StopExecution)
                            })
                            .ok();
                        return;
                    }
                    // Turbo is shutting down
                    None => {
                        callback.send(Ok(())).ok();
                        return;
                    }
                };

                let exit_status = match process
                    .wait_with_piped_outputs(&mut stdout_writer, None)
                    .await
                {
                    Ok(Some(exit_status)) => exit_status,
                    Err(e) => {
                        error!("unable to pipe outputs from command: {e}");
                        callback.send(Err(StopExecution)).ok();
                        manager.stop().await;
                        return;
                    }
                    Ok(None) => {
                        // TODO: how can this happen? we only update the
                        // exit status with Some and it is only initialized with
                        // None. Is it still running?
                        error!("unable to determine why child exited");
                        callback.send(Err(StopExecution)).ok();
                        return;
                    }
                };

                match exit_status {
                    // The task was successful, nothing special needs to happen.
                    ChildExit::Finished(Some(0)) => (),
                    ChildExit::Finished(Some(code)) => {
                        // If there was an error, flush the buffered output
                        if let Err(e) = task_cache.on_error(&mut prefixed_ui) {
                            error!("error reading logs: {e}");
                        }
                        let error =
                            TaskErrorCause::from_execution(process.label().to_string(), code);
                        if continue_on_error {
                            prefixed_ui.warn("command finished with error, but continuing...");
                            callback.send(Ok(())).ok();
                        } else {
                            prefixed_ui.error(format!("command finished with error: {error}"));
                            callback.send(Err(StopExecution)).ok();
                            manager.stop().await;
                        }
                        errors.lock().expect("lock poisoned").push(TaskError {
                            task_id: task_id_for_display.clone(),
                            cause: error,
                        });
                        return;
                    }
                    // All of these indicate a failure where we don't know how to recover
                    ChildExit::Finished(None)
                    | ChildExit::Killed
                    | ChildExit::KilledExternal
                    | ChildExit::Failed => {
                        callback.send(Err(StopExecution)).ok();
                        return;
                    }
                }

                if let Err(e) = stdout_writer.flush() {
                    error!("{e}");
                } else if let Err(e) = task_cache
                    .save_outputs(&mut prefixed_ui, Duration::from_secs(1))
                    .await
                {
                    error!("error caching output: {e}");
                } else {
                    hash_tracker.insert_expanded_outputs(
                        task_id.clone(),
                        task_cache.expanded_outputs().to_vec(),
                    );
                }

                if let Err(e) = output_client.finish() {
                    error!("unable to flush output client: {e}");
                    callback.send(Err(StopExecution)).unwrap();
                    return;
                }

                callback.send(Ok(())).unwrap();
            }));
        }

        // Wait for the engine task to finish and for all of our tasks to finish
        engine_handle.await.expect("engine execution panicked")?;
        // This will poll the futures until they are all completed
        while let Some(result) = tasks.next().await {
            result.expect("task executor panicked");
        }

        let errors = Arc::into_inner(errors)
            .expect("only one strong reference to errors should remain")
            .into_inner()
            .expect("mutex poisoned");

        Ok(errors)
    }

    fn sink(opts: &Opts, silent: bool) -> OutputSink<StdWriter> {
        let (out, err) = if silent {
            (std::io::sink().into(), std::io::sink().into())
        } else if opts.run_opts.should_redirect_stderr_to_stdout() {
            (std::io::stdout().into(), std::io::stdout().into())
        } else {
            (std::io::stdout().into(), std::io::stderr().into())
        };
        OutputSink::new(out, err)
    }

    fn output_client(&self) -> OutputClient<impl std::io::Write> {
        let behavior = match self.opts.run_opts.log_order {
            // TODO once run summary is implemented, we can skip keeping an in memory buffer if it
            // is disabled
            crate::opts::ResolvedLogOrder::Stream => {
                turborepo_ui::OutputClientBehavior::InMemoryBuffer
            }
            crate::opts::ResolvedLogOrder::Grouped => turborepo_ui::OutputClientBehavior::Grouped,
        };
        self.sink.logger(behavior)
    }

    fn prefix<'b>(&self, task_id: &'b TaskId) -> Cow<'b, str> {
        match self.opts.run_opts.log_prefix {
            crate::opts::ResolvedLogPrefix::Task if self.opts.run_opts.single_package => {
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
        match self.opts.run_opts.single_package {
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
                .with_error_prefix(Style::new().apply_to("[ERROR]".to_string()))
                .with_warn_prefix(Style::new().apply_to("[WARN]".to_string()));
        }
        prefixed_ui
    }

    pub fn into_task_hash_tracker(self) -> TaskHashTrackerState {
        self.task_hasher.into_task_hash_tracker_state()
    }

    pub fn dry_run(mut self) -> Self {
        self.dry = true;
        self
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
