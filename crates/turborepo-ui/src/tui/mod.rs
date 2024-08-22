mod app;
mod clipboard;
mod debouncer;
pub mod event;
mod handle;
mod input;
mod pane;
mod search;
mod size;
mod spinner;
mod table;
mod task;
mod term_output;

pub use app::{run_app, terminal_big_enough};
use clipboard::copy_to_clipboard;
use debouncer::Debouncer;
use event::{Event, TaskResult};
pub use handle::{AppReceiver, AppSender, TuiTask};
use input::{input, InputOptions};
pub use pane::TerminalPane;
use size::SizeInfo;
pub use table::TaskTable;
pub use term_output::TerminalOutput;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No task found with name '{name}'")]
    TaskNotFound { name: String },
    #[error("No task at index {index} (only {len} tasks) ")]
    TaskNotFoundIndex { index: usize, len: usize },
    #[error("Unable to write to stdin for '{name}': {e}")]
    Stdin { name: String, e: std::io::Error },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
