use std::sync::Arc;

use crate::{LogEvent, event::Level};

/// A destination for user-facing log events.
///
/// Sinks receive structured events and decide how to present or store them.
/// Multiple sinks can be active simultaneously (e.g., terminal output +
/// collector for post-run summary).
///
/// # Threading contract
///
/// `emit` is called synchronously on the thread that triggered the log
/// event (typically a task execution thread). Implementations **must not**
/// perform unbounded blocking — a slow sink delays all subsequent sinks
/// and the calling task. If your sink performs I/O that may block
/// (network, unbuffered disk), buffer internally and flush asynchronously,
/// or accept bounded latency.
///
/// Multiple threads may call `emit` concurrently on the same sink
/// instance. The `Send + Sync` bound is required.
///
/// # Error handling
///
/// Sink implementations should be best-effort: failures to write or
/// serialize an event should be handled silently. Logging must never
/// cause the host process to panic.
pub trait LogSink: Send + Sync + 'static {
    /// Process a log event. Must not panic.
    ///
    /// Called inline on the emitting thread. Keep this fast — see the
    /// threading contract above.
    fn emit(&self, event: &LogEvent);

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

    fn flush(&self) {
        (**self).flush()
    }

    fn enabled(&self, level: Level) -> bool {
        (**self).enabled(level)
    }
}
