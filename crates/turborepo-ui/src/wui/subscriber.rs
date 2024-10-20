use std::{collections::BTreeMap, sync::Arc};

use async_graphql::{Enum, SimpleObject};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::{
    tui::event::{CacheResult, TaskResult},
    wui::{event::WebUIEvent, query::SharedState},
};

/// Subscribes to the Web UI events and updates the state
pub struct Subscriber {
    rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>,
}

impl Subscriber {
    pub fn new(rx: tokio::sync::mpsc::UnboundedReceiver<WebUIEvent>) -> Self {
        Self { rx }
    }

    pub async fn watch(
        self,
        // We use a tokio::sync::Mutex here because we want this future to be Send.
        #[allow(clippy::type_complexity)] state: SharedState,
    ) {
        let mut rx = self.rx;
        while let Some(event) = rx.recv().await {
            Self::add_message(&state, event).await;
        }
    }

    async fn add_message(state: &Arc<Mutex<WebUIState>>, event: WebUIEvent) {
        let mut state = state.lock().await;

        match event {
            WebUIEvent::StartTask {
                task,
                output_logs: _,
            } => {
                state.tasks.insert(
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
                state.tasks.get_mut(&task).unwrap().output.extend(output);
            }
            WebUIEvent::EndTask { task, result } => {
                state.tasks.get_mut(&task).unwrap().status = TaskStatus::from(result);
            }
            WebUIEvent::CacheStatus {
                task,
                result,
                message,
            } => {
                if result == CacheResult::Hit {
                    state.tasks.get_mut(&task).unwrap().status = TaskStatus::Cached;
                }
                state.tasks.get_mut(&task).unwrap().cache_result = Some(result);
                state.tasks.get_mut(&task).unwrap().cache_message = Some(message);
            }
            WebUIEvent::Stop => {
                // TODO: stop watching
            }
            WebUIEvent::UpdateTasks { tasks } => {
                state.tasks = tasks
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
                state.tasks = tasks
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
    Succeeded,
}

impl From<TaskResult> for TaskStatus {
    fn from(result: TaskResult) -> Self {
        match result {
            TaskResult::Success => Self::Succeeded,
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

#[cfg(test)]
mod test {
    use async_graphql::{EmptyMutation, EmptySubscription, Schema};

    use super::*;
    use crate::{
        tui::event::OutputLogs,
        wui::{query::RunQuery, sender::WebUISender},
    };

    #[tokio::test]
    async fn test_web_ui_state() -> Result<(), crate::Error> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(WebUIState::default()));
        let subscriber = Subscriber::new(rx);

        let sender = WebUISender::new(tx);

        // Start a successful task
        sender.start_task("task".to_string(), OutputLogs::Full);
        sender.output("task".to_string(), b"this is my output".to_vec())?;
        sender.end_task("task".to_string(), TaskResult::Success);

        // Start a cached task
        sender.start_task("task2".to_string(), OutputLogs::Full);
        sender.status("task2".to_string(), "status".to_string(), CacheResult::Hit);

        // Start a failing task
        sender.start_task("task3".to_string(), OutputLogs::Full);
        sender.end_task("task3".to_string(), TaskResult::Failure);

        // Drop the sender so the subscriber can terminate
        drop(sender);

        // Run the subscriber blocking
        subscriber.watch(state.clone()).await;

        let state_handle = state.lock().await.clone();
        assert_eq!(state_handle.tasks().len(), 3);
        assert_eq!(
            state_handle.tasks().get("task2").unwrap().status,
            TaskStatus::Cached
        );
        assert_eq!(
            state_handle.tasks().get("task").unwrap().status,
            TaskStatus::Succeeded
        );
        assert_eq!(
            state_handle.tasks().get("task").unwrap().output,
            b"this is my output"
        );
        assert_eq!(
            state_handle.tasks().get("task3").unwrap().status,
            TaskStatus::Failed
        );

        // Now let's check with the GraphQL API
        let schema = Schema::new(RunQuery::new(Some(state)), EmptyMutation, EmptySubscription);
        let result = schema
            .execute("query { currentRun { tasks { name state { status } } } }")
            .await;
        assert!(result.errors.is_empty());
        assert_eq!(
            result.data,
            async_graphql::Value::from_json(serde_json::json!({
                "currentRun": {
                    "tasks": [
                        {
                            "name": "task",
                            "state": {
                                "status": "SUCCEEDED"
                            }
                        },
                        {
                            "name": "task2",
                            "state": {
                                "status": "CACHED"
                            }
                        },
                        {
                            "name": "task3",
                            "state": {
                                "status": "FAILED"
                            }
                        }
                    ]
                }
            }))
            .unwrap()
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_restart_tasks() -> Result<(), crate::Error> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(WebUIState::default()));
        let subscriber = Subscriber::new(rx);

        let sender = WebUISender::new(tx);

        // Start a successful task
        sender.start_task("task".to_string(), OutputLogs::Full);
        sender.output("task".to_string(), b"this is my output".to_vec())?;
        sender.end_task("task".to_string(), TaskResult::Success);

        // Start a cached task
        sender.start_task("task2".to_string(), OutputLogs::Full);
        sender.status("task2".to_string(), "status".to_string(), CacheResult::Hit);

        // Restart a task
        sender.restart_tasks(vec!["task".to_string()])?;

        // Drop the sender so the subscriber can terminate
        drop(sender);

        // Run the subscriber blocking
        subscriber.watch(state.clone()).await;

        let state_handle = state.lock().await.clone();
        assert_eq!(state_handle.tasks().len(), 1);
        assert_eq!(
            state_handle.tasks().get("task").unwrap().status,
            TaskStatus::Running
        );
        assert!(state_handle.tasks().get("task").unwrap().output.is_empty());

        Ok(())
    }
}
