use std::{
    borrow::Cow,
    io::Write,
    sync::{Arc, OnceLock},
};

use console::{Style, StyledObject};
use futures::{stream::FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::sync::mpsc;
use tracing::{debug, error};
use turborepo_env::{EnvironmentVariableMap, ResolvedEnvMode};
use turborepo_ui::{ColorSelector, OutputClient, OutputSink, OutputWriter, PrefixedUI, UI};

use crate::{
    cli::EnvMode,
    engine::{Engine, ExecutionOptions},
    opts::Opts,
    package_graph::{PackageGraph, WorkspaceName},
    run::{
        task_id::{self, TaskId},
        RunCache,
    },
    task_hash,
    task_hash::{PackageInputsHashes, TaskHasher},
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
    ) -> Self {
        let task_hasher = TaskHasher::new(
            package_inputs_hashes,
            opts,
            env_at_execution_start,
            global_hash,
        );
        let sink = Self::sink(opts);
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
        }
    }

    pub async fn visit(&self, engine: Arc<Engine>) -> Result<(), Error> {
        let concurrency = self.opts.run_opts.concurrency as usize;
        let (node_sender, mut node_stream) = mpsc::channel(concurrency);

        let engine_handle = {
            let engine = engine.clone();
            tokio::spawn(engine.execute(ExecutionOptions::new(false, concurrency), node_sender))
        };

        let mut tasks = FuturesUnordered::new();

        while let Some(message) = node_stream.recv().await {
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
                EnvMode::Infer if !task_definition.pass_through_env.is_empty() => {
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

            let task_cache =
                self.run_cache
                    .task_cache(task_definition, workspace_dir, info.clone(), &task_hash);

            // TODO(gsoltis): if/when we fix https://github.com/vercel/turbo/issues/937
            // the following block should never get hit. In the meantime, keep it after
            // hashing so that downstream tasks can count on the hash existing
            //
            // bail if the script doesn't exist
            let Some(command) = command else { continue };

            let output_client = self.output_client();
            let prefix = self.prefix(&info);
            let pretty_prefix = self.color_cache.prefix_with_color(&task_hash, &prefix);
            let ui = self.ui;

            tasks.push(tokio::spawn(async move {
                let _task_cache = task_cache;
                let mut prefixed_ui =
                    Self::prefixed_ui(ui, is_github_actions, &output_client, pretty_prefix);
                prefixed_ui.output(command);
                callback.send(Ok(())).unwrap();
            }));
        }

        // Wait for the engine task to finish and for all of our tasks to finish
        engine_handle.await.expect("engine execution panicked")?;
        // This will poll the futures until they are all completed
        while let Some(result) = tasks.next().await {
            result.expect("task executor panicked");
        }

        Ok(())
    }

    fn sink(opts: &Opts) -> OutputSink<StdWriter> {
        let err_writer = match matches!(
            opts.run_opts.log_order,
            crate::opts::ResolvedLogOrder::Grouped
        ) && opts.run_opts.is_github_actions
        {
            // If we're running on Github Actions, force everything to stdout
            // so as not to have out-of-order log lines
            true => std::io::stdout().into(),
            false => std::io::stderr().into(),
        };
        OutputSink::new(std::io::stdout().into(), err_writer)
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
}

// A tiny enum that allows us to use the same type for stdout and stderr without
// the use of Box<dyn Write>
enum StdWriter {
    Out(std::io::Stdout),
    Err(std::io::Stderr),
}

impl StdWriter {
    fn writer(&mut self) -> &mut dyn std::io::Write {
        match self {
            StdWriter::Out(out) => out,
            StdWriter::Err(err) => err,
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
