use std::sync::{Arc, OnceLock};

use futures::{stream::FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::sync::mpsc;
use tracing::debug;
use turborepo_env::{EnvironmentVariableMap, ResolvedEnvMode};

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
    pub fn new(
        package_graph: Arc<PackageGraph>,
        run_cache: Arc<RunCache>,
        opts: &'a Opts,
        package_inputs_hashes: PackageInputsHashes,
        env_at_execution_start: &'a EnvironmentVariableMap,
        global_hash: &'a str,
        global_env_mode: EnvMode,
    ) -> Self {
        let task_hasher = TaskHasher::new(
            package_inputs_hashes,
            opts,
            env_at_execution_start,
            global_hash,
        );

        Self {
            run_cache,
            package_graph,
            opts,
            task_hasher,
            global_env_mode,
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

            tasks.push(tokio::spawn(async move {
                println!(
                    "Executing {info}: {}",
                    command.as_deref().unwrap_or("no script def")
                );
                let _task_cache = task_cache;
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
}

fn turbo_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|\s)turbo(?:$|\s)").unwrap())
}
