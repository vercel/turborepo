use std::sync::Arc;

use crate::{
    LogEvent,
    event::{Level, OutputChannel},
};

/// A destination for user-facing log events and task output.
///
/// Sinks receive structured events and streaming task output, then
/// decide how to present or store them. Multiple sinks can be active
/// simultaneously (e.g., terminal output + collector for post-run summary).
///
/// # Threading contract
///
/// Methods are called synchronously on the thread that triggered the
/// event (typically a task execution thread). Implementations **must not**
/// perform unbounded blocking — a slow sink delays all subsequent sinks
/// and the calling task. If your sink performs I/O that may block
/// (network, unbuffered disk), buffer internally and flush asynchronously,
/// or accept bounded latency.
///
/// Multiple threads may call methods concurrently on the same sink
/// instance. The `Send + Sync` bound is required.
///
/// # Error handling
///
/// Sink implementations should be best-effort: failures to write or
/// serialize an event should be handled silently. Logging must never
/// cause the host process to panic.
pub trait LogSink: Send + Sync + 'static {
    /// Process a structured log event. Must not panic.
    ///
    /// Called inline on the emitting thread. Keep this fast — see the
    /// threading contract above.
    fn emit(&self, event: &LogEvent);

    /// Process raw bytes from a task's child process.
    ///
    /// Called for each chunk of stdout/stderr output from a running task.
    /// `channel` indicates whether the bytes came from stdout or stderr.
    ///
    /// Default: no-op. Sinks that don't care about task output can
    /// ignore this.
    fn task_output(&self, _task: &str, _channel: OutputChannel, _bytes: &[u8]) {}

    /// Called before a grouped task flush begins.
    ///
    /// In grouped mode, all output for a task is buffered and flushed
    /// atomically on task completion. This method is called once before
    /// the buffered events and bytes are replayed through `emit` and
    /// `task_output`.
    ///
    /// `is_error` is true when the task failed, allowing sinks to use
    /// different styling (e.g., red CI group headers).
    ///
    /// Default: no-op.
    fn begin_task_group(&self, _task: &str, _is_error: bool) {}

    /// Called after a grouped task flush completes.
    ///
    /// Pairs with [`begin_task_group`](Self::begin_task_group). Sinks
    /// that write CI group markers (e.g., `::endgroup::`) should emit
    /// the closing marker here.
    ///
    /// `is_error` matches the value passed to the corresponding
    /// `begin_task_group` call. On some CI providers, error tasks
    /// skip the opening group marker and must also skip the closing
    /// marker to avoid unpaired annotations.
    ///
    /// Default: no-op.
    fn end_task_group(&self, _task: &str, _is_error: bool) {}

    /// Register a task with this sink.
    ///
    /// Called before a task starts producing output. Sinks that need
    /// per-task render state (e.g., colored prefixes, line buffers)
    /// should initialize it here.
    ///
    /// `task` is the task identifier used in `task_output()` calls.
    /// `prefix` is the display prefix for terminal rendering
    /// (e.g., `"my-app:build"` — the sink appends `": "`).
    ///
    /// Default: no-op.
    fn register_task(&self, _task: &str, _prefix: &str) {}

    /// Flush any buffered output. Called during graceful shutdown.
    fn flush(&self) {}

    /// Whether this sink wants events at the given level.
    /// Return `false` to skip dispatch entirely (avoids serialization cost).
    /// Default: accept all levels.
    fn enabled(&self, _level: Level) -> bool {
        true
    }
}

impl<T: LogSink> LogSink for Arc<T> {
    fn emit(&self, event: &LogEvent) {
        (**self).emit(event)
    }

    fn task_output(&self, task: &str, channel: OutputChannel, bytes: &[u8]) {
        (**self).task_output(task, channel, bytes)
    }

    fn begin_task_group(&self, task: &str, is_error: bool) {
        (**self).begin_task_group(task, is_error)
    }

    fn end_task_group(&self, task: &str, is_error: bool) {
        (**self).end_task_group(task, is_error)
    }

    fn register_task(&self, task: &str, prefix: &str) {
        (**self).register_task(task, prefix)
    }

    fn flush(&self) {
        (**self).flush()
    }

    fn enabled(&self, level: Level) -> bool {
        (**self).enabled(level)
    }
}
