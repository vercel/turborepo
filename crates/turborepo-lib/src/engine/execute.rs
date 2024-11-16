use std::sync::{Arc, Mutex};

use futures::{stream::FuturesUnordered, StreamExt};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tracing::log::debug;
use turborepo_graph_utils::Walker;

use super::{Engine, TaskNode};
use crate::run::task_id::TaskId;

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
    #[error("Engine visitor closed channel before walk finished")]
    Visitor,
}

impl From<mpsc::error::SendError<Message<VisitorData, VisitorResult>>> for ExecuteError {
    fn from(
        _: mpsc::error::SendError<Message<TaskId<'static>, Result<(), StopExecution>>>,
    ) -> Self {
        ExecuteError::Visitor
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StopExecution;

impl Engine {
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
                    .expect("node id should be present")
                else {
                    // Root task has nothing to do so we don't emit any event for it
                    if done.send(()).is_err() {
                        debug!(
                            "Graph walker done callback receiver was closed before done signal \
                             could be sent"
                        );
                    }
                    return Ok(());
                };

                // Acquire the semaphore unless parallel
                let _permit = match parallel {
                    false => Some(sema.acquire().await.expect(
                        "Graph concurrency semaphore closed while tasks are still attempting to \
                         acquire permits",
                    )),
                    true => None,
                };

                let (message, result) = Message::new(task_id.clone());
                visitor.send(message).await?;

                if let Err(StopExecution) = result.await.unwrap_or_else(|_| {
                    // If the visitor doesn't send a callback, then we assume the task finished
                    tracing::trace!(
                        "Engine visitor dropped callback sender without sending result"
                    );
                    Ok(())
                }) {
                    if walker
                        .lock()
                        .expect("Walker mutex poisoned")
                        .cancel()
                        .is_err()
                    {
                        debug!("Unable to cancel graph walk");
                    }
                }
                if done.send(()).is_err() {
                    debug!("Graph walk done receiver closed before node was finished processing");
                }
                Ok(())
            }));
        }

        while let Some(res) = tasks.next().await {
            res.expect("unable to join task")?;
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
