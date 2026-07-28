use std::sync::{Arc, Mutex};

use futures::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tracing::debug;
use turborepo_graph_utils::Walker;
use turborepo_task_id::TaskId;
use turborepo_types::StopExecution;

use super::{Built, Engine, TaskDefinitionInfo, TaskNode};

pub struct Message<T, U> {
    pub info: T,
    pub callback: oneshot::Sender<U>,
}

// Type alias used just to make altering the data sent to the visitor easier in
// the future
type VisitorData = TaskId<'static>;
type VisitorResult = Result<(), StopExecution>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    parallel: bool,
    concurrency: usize,
}

impl ExecutionOptions {
    pub fn new(parallel: bool, concurrency: usize) -> Self {
        Self {
            parallel,
            concurrency,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("Semaphore closed before all tasks finished")]
    Semaphore(#[from] tokio::sync::AcquireError),
    #[error("Task worker failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("Engine visitor closed channel before walk finished")]
    Visitor,
    #[error(
        "Task graph contains a cycle — validate_graph should have rejected it before execution"
    )]
    CyclicTaskGraph,
}

impl From<mpsc::error::SendError<Message<VisitorData, VisitorResult>>> for ExecuteError {
    fn from(
        _: mpsc::error::SendError<Message<TaskId<'static>, Result<(), StopExecution>>>,
    ) -> Self {
        ExecuteError::Visitor
    }
}

impl<T: TaskDefinitionInfo + Clone + Send + Sync + 'static> Engine<Built, T> {
    /// Execute a task graph by sending task ids to the visitor
    /// while respecting concurrency limits.
    /// The visitor is expected to handle any error handling on its end.
    /// We enforce this by only allowing the returning of a sentinel error
    /// type which will stop any further execution of tasks.
    /// This will not stop any task which is currently running, simply it will
    /// stop scheduling new tasks.
    // (olszewski) The current impl requires that the visitor receiver is read until
    // finish even once a task sends back the stop signal. This is suboptimal
    // since it would mean the visitor would need to also track if
    // it is cancelled :)
    #[tracing::instrument(skip_all)]
    pub async fn execute(
        self: Arc<Self>,
        options: ExecutionOptions,
        visitor: mpsc::Sender<Message<VisitorData, VisitorResult>>,
    ) -> Result<(), ExecuteError> {
        let ExecutionOptions {
            parallel,
            concurrency,
        } = options;
        let sema = Arc::new(Semaphore::new(concurrency));
        let mut tasks: FuturesUnordered<tokio::task::JoinHandle<Result<(), ExecuteError>>> =
            FuturesUnordered::new();

        if petgraph::algo::is_cyclic_directed(&self.task_graph) {
            return Err(ExecuteError::CyclicTaskGraph);
        }
        let (walker, mut nodes) = Walker::new(&self.task_graph).walk();
        let walker = Arc::new(Mutex::new(walker));

        while let Some((node_id, done)) = nodes.recv().await {
            let visitor = visitor.clone();
            let sema = sema.clone();
            let walker = walker.clone();
            let this = self.clone();

            tasks.push(tokio::spawn(async move {
                let TaskNode::Task(task_id) = this
                    .task_graph
                    .node_weight(node_id)
                    .unwrap_or(&TaskNode::Root)
                else {
                    // Root task has nothing to do so we don't emit any event for it
                    if done.send(true).is_err() {
                        debug!(
                            "Graph walker done callback receiver was closed before done signal \
                             could be sent"
                        );
                    }
                    return Ok(());
                };

                // Acquire the semaphore unless parallel
                let _permit = match parallel {
                    false => Some(sema.acquire().await?),
                    true => None,
                };

                let (message, result) = Message::new(task_id.clone());
                visitor.send(message).await?;

                let mut continue_walking_subgraph = true;
                match result.await.unwrap_or_else(|_| {
                    // If the visitor doesn't send a callback, then we assume the task finished
                    tracing::trace!(
                        "Engine visitor dropped callback sender without sending result"
                    );
                    Ok(())
                }) {
                    Err(StopExecution::AllTasks)
                        if walker
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .cancel()
                            .is_err() =>
                    {
                        debug!("Unable to cancel graph walk");
                    }
                    Err(StopExecution::DependentTasks) => {
                        continue_walking_subgraph = false;
                    }
                    _ => (),
                };
                if done.send(continue_walking_subgraph).is_err() {
                    debug!("Graph walk done receiver closed before node was finished processing");
                }
                Ok(())
            }));
        }

        while let Some(res) = tasks.next().await {
            res??;
        }

        Ok(())
    }
}

impl<T, U> Message<T, U> {
    pub fn new(info: T) -> (Self, oneshot::Receiver<U>) {
        let (callback, receiver) = oneshot::channel();
        (Self { info, callback }, receiver)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use tokio::sync::mpsc;
    use turborepo_types::StopExecution;

    use super::*;
    use crate::{Building, TaskInfo};

    fn chain_engine() -> Arc<Engine<Built, TaskInfo>> {
        let mut engine: Engine<Building, TaskInfo> = Engine::new();
        let lib = TaskId::new("lib", "build");
        let app = TaskId::new("app", "build");

        let lib_idx = engine.get_index(&lib);
        let app_idx = engine.get_index(&app);

        engine.add_definition(lib.clone(), TaskInfo::default());
        engine.add_definition(app.clone(), TaskInfo::default());

        engine.task_graph_mut().add_edge(app_idx, lib_idx, ());
        engine.connect_to_root(&lib);

        Arc::new(engine.seal())
    }

    fn branch_engine() -> Arc<Engine<Built, TaskInfo>> {
        let mut engine: Engine<Building, TaskInfo> = Engine::new();
        let lib = TaskId::new("lib", "build");
        let app = TaskId::new("app", "build");
        let other = TaskId::new("other", "build");

        let lib_idx = engine.get_index(&lib);
        let app_idx = engine.get_index(&app);

        engine.add_definition(lib.clone(), TaskInfo::default());
        engine.add_definition(app.clone(), TaskInfo::default());
        engine.add_definition(other.clone(), TaskInfo::default());

        engine.task_graph_mut().add_edge(app_idx, lib_idx, ());
        engine.connect_to_root(&lib);
        engine.connect_to_root(&other);

        Arc::new(engine.seal())
    }

    async fn execute_with_results(
        engine: Arc<Engine<Built, TaskInfo>>,
        results: HashMap<TaskId<'static>, Result<(), StopExecution>>,
    ) -> Vec<TaskId<'static>> {
        let (tx, mut rx) = mpsc::channel(16);
        let execution = tokio::spawn(engine.execute(ExecutionOptions::new(false, 1), tx));
        let mut visited = Vec::new();

        while let Some(message) = rx.recv().await {
            visited.push(message.info.clone());
            let result = results.get(&message.info).copied().unwrap_or(Ok(()));
            message.callback.send(result).ok();
        }

        execution.await.unwrap().unwrap();
        visited
    }

    #[tokio::test]
    async fn execute_continues_dependents_after_success() {
        let visited = execute_with_results(chain_engine(), HashMap::new()).await;

        assert!(visited.contains(&TaskId::new("lib", "build")));
        assert!(visited.contains(&TaskId::new("app", "build")));
    }

    #[tokio::test]
    async fn execute_skips_dependents_after_dependent_tasks_stop() {
        let visited = execute_with_results(
            branch_engine(),
            HashMap::from([(
                TaskId::new("lib", "build"),
                Err(StopExecution::DependentTasks),
            )]),
        )
        .await;

        assert!(visited.contains(&TaskId::new("lib", "build")));
        assert!(visited.contains(&TaskId::new("other", "build")));
        assert!(!visited.contains(&TaskId::new("app", "build")));
    }

    #[tokio::test]
    async fn execute_skips_dependents_after_all_tasks_stop() {
        let visited = execute_with_results(
            chain_engine(),
            HashMap::from([(TaskId::new("lib", "build"), Err(StopExecution::AllTasks))]),
        )
        .await;

        assert!(visited.contains(&TaskId::new("lib", "build")));
        assert!(!visited.contains(&TaskId::new("app", "build")));
    }
}
