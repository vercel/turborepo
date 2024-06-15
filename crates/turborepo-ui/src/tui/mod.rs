mod app;
pub mod event;
mod handle;
mod input;
mod pane;
mod spinner;
mod table;
mod task;

pub use app::{run_app, terminal_big_enough};
use event::{Event, TaskResult};
pub use handle::{AppReceiver, AppSender, TuiTask};
use input::{input, InputOptions};
pub use pane::TerminalPane;
pub use table::TaskTable;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No task found with name '{name}'")]
    TaskNotFound { name: String },
    #[error("Unable to write to stdin for '{name}': {e}")]
    Stdin { name: String, e: std::io::Error },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
