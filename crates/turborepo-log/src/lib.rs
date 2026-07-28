#![warn(missing_docs)]
//! User-facing logging interface for Turborepo.
//!
//! This crate provides a structured event system for messages intended for
//! Turborepo's end users: warnings, errors, and informational output. It is
//! separate from Rust's `tracing` crate, which remains for developer
//! diagnostics.
//!
//! # When to use `turborepo-log` vs `tracing`
//!
//! | Audience     | Crate            | Example                              |
//! |--------------|------------------|--------------------------------------|
//! | End user     | `turborepo-log`  | `"cache miss for web#build"`         |
//! | Developer    | `tracing`        | `"resolving lockfile at path={path}"`|
//!
//! **Rule of thumb**: If the message should appear in `turbo run` output
//! that an end user sees, use `turborepo-log`. If it's for debug output
//! or internal diagnostics, use `tracing`.
//!
//! # When NOT to use this crate
//!
//! - **Program data output** (e.g., `turbo ls` listings, `--dry=json`, `turbo
//!   info`): These are program output consumed by scripts or users. Use
//!   `println!` for these — they go to stdout and may be piped.
//! - **Internal diagnostics**: Use `tracing::{debug,trace,info,warn}!`. These
//!   are filtered by `TURBO_LOG_VERBOSITY` and are not user-facing.
//!
//! # Architecture
//!
//! - **Handle**: [`LogHandle`] provides a source-scoped API for emitting
//!   events. Create one via [`log()`] (global) or [`Logger::handle()`]
//!   (specific logger).
//! - **Sinks**: Implement [`LogSink`] to route events to different destinations
//!   (terminal, TUI, file, collector, etc.).
//! - **Logger**: [`Logger`] dispatches events to all registered sinks. Set a
//!   global logger via [`init()`], or use a `Logger` directly for testing.
//!
//! # Usage
//!
//! ```no_run
//! use std::sync::Arc;
//! use turborepo_log::{init, log, Logger, Source, Subsystem};
//! use turborepo_log::sinks::collector::CollectorSink;
//!
//! // Initialize the global logger (once, at startup).
//! let collector = Arc::new(CollectorSink::new());
//! init(Logger::new(vec![Box::new(collector.clone())])).ok();
//!
//! // Create a source-scoped handle and emit events.
//! let handle = log(Source::turbo(Subsystem::Cache));
//! handle.warn("'daemon' config option is deprecated").emit();
//!
//! // With structured fields:
//! handle.warn("deprecated field")
//!     .field("name", "daemon")
//!     .emit();
//!
//! // Task-scoped:
//! let task_handle = log(Source::task("web#build"));
//! task_handle.error("exited with non-zero code")
//!     .field("code", 137)
//!     .emit();
//! ```
//!
//! # Testing
//!
//! The global logger can only be set once per process. For unit tests,
//! create a [`Logger`] directly and use [`Logger::handle()`]:
//!
//! ```
//! use std::sync::Arc;
//! use turborepo_log::{Logger, Source};
//! use turborepo_log::sinks::collector::CollectorSink;
//!
//! let (collector, logger) = CollectorSink::with_logger();
//!
//! let handle = logger.handle(Source::turbo(Subsystem::Cache));
//! handle.warn("test warning").emit();
//!
//! assert_eq!(collector.events().len(), 1);
//! ```
//!
//! # Relationship to `turborepo-ui`
//!
//! `turborepo-ui` handles terminal rendering (TUI, console formatting,
//! progress output). `turborepo-log` handles structured event capture
//! and dispatch. They are complementary: a terminal sink in
//! `turborepo-ui` can implement [`LogSink`] to bridge structured
//! events into the existing rendering pipeline. This crate
//! intentionally has no dependency on `turborepo-ui`.
//!
//! # Limitations
//!
//! The global logger uses [`OnceLock`](std::sync::OnceLock) and cannot
//! be reconfigured after initialization. For long-running processes
//! (e.g., `turbo daemon`), sink-level reconfiguration (such as file
//! rotation) should be handled within the sink implementation rather
//! than by replacing the logger.

mod event;
pub mod grouping;
mod logger;
mod sink;
pub mod sinks;

pub use event::{
    Level, LogEvent, OutputChannel, SanitizedString, Scalar, Source, Subsystem, Value,
};
pub use logger::{
    InitError, LogEventBuilder, LogHandle, Logger, error, flush, global_logger, info, init, log,
    warn,
};
pub use sink::LogSink;
pub use sinks::structured::{StructuredLogSink, StructuredTaskWriter, TeeWriter};
