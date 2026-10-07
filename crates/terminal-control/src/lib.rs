//! Control, model, render, and record terminal applications.
//!
//! The command-line interface is built on this library. A [`frame::Frame`] is the stable,
//! structured representation of one visible terminal shot, while a `.termctrl` recording is a
//! JSON Lines stream of [`recording::Entry`] values.
//!
//! ```
//! let shot = terminal_control::shot::from_ansi(b"ready".to_vec(), 1, 20, 1024).unwrap();
//! assert_eq!(shot.frame.text(), "ready");
//! ```

pub mod frame;
pub mod recording;
pub mod render;
mod runtime;
mod semantic;
pub mod session;
pub mod shot;
mod terminal_core;
mod terminal_theme;
mod workspace;
