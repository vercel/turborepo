use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use async_graphql::{Enum, SimpleObject};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    tui::event::{CacheResult, TaskResult},
    wui::event::WebUIEvent,
};

/// Subscribes to the Web UI events and updates the state
pub struct Subscriber {
    rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>,
}

impl Subscriber {
    pub fn new(rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>) -> Self {
        Self { rx }
    }

    pub fn watch(
        self,
        // We use a tokio::sync::Mutex here because we want this future to be Send.
        #[allow(clippy::type_complexity)] state: Arc<Mutex<RefCell<WebUIState>>>,
    ) {
        tokio::spawn(async move {
            let mut rx = self.rx;
            while let Some(event) = rx.recv().await {
                Self::add_message(&state, event).await;
            }
        });
    }

    async fn add_message(state: &Arc<Mutex<RefCell<WebUIState>>>, event: WebUIEvent) {
        let state = state.lock().await;

        match event {
            WebUIEvent::StartTask {
                task,
                output_logs: _,
            } => {
                state.borrow_mut().tasks.insert(
                    task,
                    TaskState {
                        output: Vec::new(),
                        status: TaskStatus::Running,
                        cache_result: None,
                        cache_message: None,
                    },
                );
            }
            WebUIEvent::TaskOutput { task, output } => {
                state
                    .borrow_mut()
                    .tasks
                    .get_mut(&task)
                    .unwrap()
                    .output
                    .extend(output);
            }
            WebUIEvent::EndTask { task, result } => {
                state.borrow_mut().tasks.get_mut(&task).unwrap().status = TaskStatus::from(result);
            }
            WebUIEvent::CacheStatus {
                task,
                result,
                message,
            } => {
                let mut state_ref = state.borrow_mut();

                state_ref.tasks.get_mut(&task).unwrap().cache_result = Some(result);

                state_ref.tasks.get_mut(&task).unwrap().cache_message = Some(message);
            }
            WebUIEvent::Stop => {
                // TODO: stop watching
            }
            WebUIEvent::UpdateTasks { tasks } => {
                state.borrow_mut().tasks = tasks
                    .into_iter()
                    .map(|task| {
                        (
                            task,
                            TaskState {
                                output: Vec::new(),
                                status: TaskStatus::Pending,
                                cache_result: None,
                                cache_message: None,
                            },
                        )
                    })
                    .collect();
            }
            WebUIEvent::RestartTasks { tasks } => {
                state.borrow_mut().tasks = tasks
                    .into_iter()
                    .map(|task| {
                        (
                            task,
                            TaskState {
                                output: Vec::new(),
                                status: TaskStatus::Running,
                                cache_result: None,
                                cache_message: None,
                            },
                        )
                    })
                    .collect();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Enum)]
pub enum TaskStatus {
    Pending,
    Running,
    Cached,
    Failed,
    Success,
}

impl From<TaskResult> for TaskStatus {
    fn from(result: TaskResult) -> Self {
        match result {
            TaskResult::Success => Self::Success,
            TaskResult::CacheHit => Self::Cached,
            TaskResult::Failure => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, SimpleObject)]
pub struct TaskState {
    output: Vec<u8>,
    status: TaskStatus,
    cache_result: Option<CacheResult>,
    /// The message for the cache status, i.e. `cache hit, replaying logs`
    cache_message: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct WebUIState {
    tasks: BTreeMap<String, TaskState>,
}

impl WebUIState {
    pub fn tasks(&self) -> &BTreeMap<String, TaskState> {
        &self.tasks
    }
}
