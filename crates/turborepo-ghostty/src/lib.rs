//! Ghostty-backed virtual terminal support for Turborepo's TUI.
//!
//! This crate vendors safe Rust wrappers around [`libghostty-vt`] (adapted from
//! [libghostty-rs](https://github.com/uzaaft/libghostty-rs)) and links against
//! Ghostty via [`turborepo-ghostty-sys`].
//!
//! [`libghostty-vt`]: https://ghostty.org

#![allow(clippy::expect_used)]
#![allow(
    clippy::all,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::pedantic,
    clippy::doc_markdown,
    missing_docs,
    unexpected_cfgs,
    dead_code,
    reason = "vendored libghostty-vt bindings"
)]

pub use turborepo_ghostty_sys as ffi;

pub mod alloc;
pub mod error;
pub mod fmt;
pub mod key;
pub mod render;
pub mod screen;
pub mod selection;
pub mod style;
pub mod terminal;

mod convert;
mod parser;
mod widget;

pub use error::Error as GhosttyError;
pub use parser::Parser;
pub use render::RenderState;
pub use terminal::{Options as TerminalOptions, Terminal};
pub use widget::{CursorState, CursorStyle, TerminalWidget};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Ghostty(#[from] GhosttyError),
}

pub type Result<T> = std::result::Result<T, Error>;
