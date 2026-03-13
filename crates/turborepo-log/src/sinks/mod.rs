//! Built-in [`LogSink`](crate::LogSink) implementations.
//!
//! - [`collector::CollectorSink`] — In-memory event buffer for post-run
//!   summaries and testing
//! - [`file::FileSink`] — Newline-delimited JSON file output with optional size
//!   limiting
//!
//! To implement a custom sink, see the [`LogSink`](crate::LogSink) trait.

/// In-memory event buffer for post-run summaries and testing.
pub mod collector;
/// Newline-delimited JSON file output with optional size limiting.
pub mod file;
