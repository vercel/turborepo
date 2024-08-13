mod app;
mod clipboard;
pub mod event;
mod handle;
mod input;
mod pane;
mod spinner;
mod table;
mod task;
mod term_output;

pub use app::{restore_default_terminal, run_app, terminal_big_enough};
use clipboard::copy_to_clipboard;
use event::{Event, TaskResult};
pub use handle::{AppReceiver, AppSender, TuiTask};
use input::{input, InputOptions};
pub use pane::TerminalPane;
pub use table::TaskTable;
pub use term_output::TerminalOutput;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("No task found with name '{name}'")]
    TaskNotFound { name: String },
    #[error("Unable to write to stdin for '{name}': {e}")]
    Stdin { name: String, e: std::io::Error },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
