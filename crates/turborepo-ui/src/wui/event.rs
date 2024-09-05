use serde::Serialize;

use crate::tui::event::{CacheResult, OutputLogs, TaskResult};

/// Specific events that the GraphQL server can send to the client,
/// not all the `Event` types from the TUI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum WebUIEvent {
    StartTask {
        task: String,
        output_logs: OutputLogs,
    },
    TaskOutput {
        task: String,
        output: Vec<u8>,
    },
    EndTask {
        task: String,
        result: TaskResult,
    },
    CacheStatus {
        task: String,
        message: String,
        result: CacheResult,
    },
    UpdateTasks {
        tasks: Vec<String>,
    },
    RestartTasks {
        tasks: Vec<String>,
    },
    Stop,
}
