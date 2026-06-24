//! Ghostty-backed virtual terminal support for Turborepo's TUI.
//!
//! This crate wraps [`libghostty-vt`] and provides a ratatui widget for
//! rendering parsed task output in the TUI pane.

#![allow(clippy::expect_used)]

mod convert;
mod parser;
mod widget;

pub use libghostty_vt::{Error as GhosttyError, RenderState, Terminal, TerminalOptions};
pub use parser::Parser;
pub use widget::{CursorState, CursorStyle, TerminalWidget};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Ghostty(#[from] GhosttyError),
}

pub type Result<T> = std::result::Result<T, Error>;
