mod pane;
mod table;
mod task;
mod task_duration;

pub use pane::TerminalPane;
pub use table::TaskTable;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No task found with name '{name}'")]
    TaskNotFound { name: String },
    #[error("Unable to write to stdin for '{name}': {e}")]
    Stdin { name: String, e: std::io::Error },
}
