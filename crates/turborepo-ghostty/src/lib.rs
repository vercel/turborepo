//! Ghostty-backed virtual terminal support for Turborepo's TUI.
//!
//! This crate wraps [`libghostty-vt`] with Turborepo-specific helpers: a
//! [`Parser`] for task output and a ratatui [`TerminalWidget`].
//!
//! [`libghostty-vt`]: https://github.com/Uzaaft/libghostty-rs

pub use libghostty_vt::{RenderState, Terminal, terminal::Options as TerminalOptions};

mod convert;
mod parser;
mod widget;

pub use parser::Parser;
pub use widget::{CursorState, CursorStyle, TerminalWidget};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Ghostty(#[from] libghostty_vt::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
