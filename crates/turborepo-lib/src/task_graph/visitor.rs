use std::sync::{Arc, OnceLock};

use futures::{stream::FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::sync::mpsc;

use crate::{
    engine::{Engine, ExecutionOptions},
    opts::Opts,
    package_graph::{PackageGraph, WorkspaceName},
    run::task_id::{self, TaskId},
};

// This holds the whole world
pub struct Visitor<'a> {
    package_graph: Arc<PackageGraph>,
    opts: &'a Opts<'a>,
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
}

impl<'a> Visitor<'a> {
    pub fn new(package_graph: Arc<PackageGraph>, opts: &'a Opts) -> Self {
        Self {
            package_graph,
            opts,
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
            let package_json = self
                .package_graph
                .package_json(&package_name)
                .ok_or_else(|| Error::MissingPackage {
                    package_name: package_name.clone(),
                    task_id: info.clone(),
                })?;

            let command = package_json.scripts.get(info.task()).cloned();

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

            let _task_def = engine
                .task_definition(&info)
                .ok_or(Error::MissingDefinition)?;

            tasks.push(tokio::spawn(async move {
                println!(
                    "Executing {info}: {}",
                    command.as_deref().unwrap_or("no script def")
                );
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
