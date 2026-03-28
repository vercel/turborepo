//! Built-in [`LogSink`](crate::LogSink) implementations.
//!
//! - [`collector::CollectorSink`] — In-memory event buffer for post-run
//!   summaries and testing
//! - [`file::FileSink`] — Newline-delimited JSON file output with optional size
//!   limiting
//! - [`structured::StructuredLogSink`] — Machine-readable structured log output
//!   (JSON array file and/or NDJSON terminal)
//!
//! To implement a custom sink, see the [`LogSink`](crate::LogSink) trait.

/// In-memory event buffer for post-run summaries and testing.
pub mod collector;
/// Newline-delimited JSON file output with optional size limiting.
pub mod file;
/// Machine-readable structured log output for observability.
pub mod structured;
