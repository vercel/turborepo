use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    event::{Level, LogEvent, Source, Value},
    sink::LogSink,
};

static GLOBAL_LOGGER: OnceLock<Arc<Logger>> = OnceLock::new();

// Compile-time proof that Logger is Send + Sync (required by
// OnceLock<Arc<Logger>>).
#[allow(dead_code)]
const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}
    fn check() {
        assert_send_sync::<Logger>();
    }
};

/// Process-wide logger that dispatches events to registered sinks.
///
/// Create via [`Logger::new`], then either register globally with [`init`]
/// or use directly via [`Logger::handle`] for testing.
pub struct Logger {
    sinks: Vec<Box<dyn LogSink>>,
}

impl fmt::Debug for Logger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logger")
            .field("sinks", &self.sinks.len())
            .finish()
    }
}

impl Logger {
    /// Create a logger that dispatches to the given sinks.
    ///
    /// An empty `sinks` list is valid — events will be silently dropped.
    /// Sinks are called sequentially in registration order.
    #[must_use]
    pub fn new(sinks: Vec<Box<dyn LogSink>>) -> Self {
        Self { sinks }
    }

    /// Dispatch an event to all registered sinks.
    ///
    /// Each sink's [`LogSink::enabled`] method is checked before dispatch.
    pub fn emit(&self, event: &LogEvent) {
        for sink in &self.sinks {
            if sink.enabled(event.level) {
                sink.emit(event);
            }
        }
    }

    /// Flush all sinks. Call during graceful shutdown.
    pub fn flush(&self) {
        for sink in &self.sinks {
            sink.flush();
        }
    }

    /// Create a source-scoped log handle bound to this logger.
    ///
    /// Use this in tests and when you need to bypass the global logger.
    pub fn handle(self: &Arc<Self>, source: Source) -> LogHandle {
        LogHandle {
            source,
            logger: LoggerRef::Direct(Arc::clone(self)),
        }
    }
}

/// Error returned by [`init`] when the global logger has already been set.
#[derive(Debug, Clone, Copy)]
pub struct InitError;

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("global logger already initialized")
    }
}

impl std::error::Error for InitError {}

/// Set the global logger. Returns `Err` if already initialized.
///
/// Call this once during process startup, after sinks are configured.
/// Events emitted before `init` are silently dropped.
///
/// # Errors
///
/// Returns [`InitError`] if the global logger was already set. The
/// provided `Logger` (and its sinks) is dropped in that case — the
/// existing global logger remains active.
#[must_use = "returns Err(InitError) if the global logger was already initialized"]
pub fn init(logger: Logger) -> Result<(), InitError> {
    GLOBAL_LOGGER.set(Arc::new(logger)).map_err(|_| InitError)
}

/// Flush all sinks on the global logger. Call during graceful shutdown.
pub fn flush() {
    if let Some(logger) = GLOBAL_LOGGER.get() {
        logger.flush();
    }
}

/// How a [`LogHandle`] resolves its logger.
#[derive(Clone)]
enum LoggerRef {
    /// Resolve the global logger at emit time (lazy). Handles created
    /// before `init()` will start working once `init()` is called.
    Global,
    /// Bound to a specific logger (eager). Used by `Logger::handle()`.
    Direct(Arc<Logger>),
}

/// A source-scoped handle for emitting user-facing log events.
///
/// Created via [`log()`] (global logger) or [`Logger::handle()`]
/// (specific logger). Cheap to clone — carries the source and either
/// a reference-counted logger pointer or a marker to use the global.
///
/// **Important**: Handles created via [`log()`] resolve the global
/// logger at `.emit()` time, not at handle or builder creation time.
/// This means handles — and builders created from them — will work
/// once [`init()`] is called, even if both were created before
/// initialization. Handles created via [`Logger::handle()`] are
/// permanently bound to their logger.
///
/// ```no_run
/// use turborepo_log::{log, Source, Subsystem};
///
/// let handle = log(Source::turbo(Subsystem::Cache));
/// handle.warn("'daemon' config option is deprecated").emit();
/// handle.warn("deprecated field").field("name", "daemon").emit();
/// ```
#[derive(Clone)]
pub struct LogHandle {
    source: Source,
    logger: LoggerRef,
}

impl fmt::Debug for LogHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogHandle")
            .field("source", &self.source)
            .field(
                "logger",
                &match &self.logger {
                    LoggerRef::Global => "Global",
                    LoggerRef::Direct(_) => "Direct",
                },
            )
            .finish()
    }
}

impl LogHandle {
    fn build<'a>(&'a self, level: Level, message: impl Into<String>) -> LogEventBuilder<'a> {
        let resolver = match &self.logger {
            LoggerRef::Global => LogResolver::Global,
            LoggerRef::Direct(arc) => LogResolver::Direct(arc.as_ref()),
        };
        LogEventBuilder {
            event: LogEvent::new(level, self.source.clone(), message),
            resolver,
        }
    }

    /// Create a warning-level event builder.
    pub fn warn(&self, message: impl Into<String>) -> LogEventBuilder<'_> {
        self.build(Level::Warn, message)
    }

    /// Create an info-level event builder.
    pub fn info(&self, message: impl Into<String>) -> LogEventBuilder<'_> {
        self.build(Level::Info, message)
    }

    /// Create an error-level event builder.
    pub fn error(&self, message: impl Into<String>) -> LogEventBuilder<'_> {
        self.build(Level::Error, message)
    }
}

/// Create a source-scoped log handle using the global logger.
///
/// The handle resolves the global logger at emit time, not at creation
/// time. This means handles created before [`init()`] will work once
/// the global logger is set.
///
/// Prefer creating a handle and reusing it when emitting multiple events
/// from the same source. For one-off events, the free functions
/// [`warn()`], [`info()`], and [`error()`] are more concise.
pub fn log(source: Source) -> LogHandle {
    LogHandle {
        source,
        logger: LoggerRef::Global,
    }
}

/// How a [`LogEventBuilder`] resolves its logger at emit time.
enum LogResolver<'a> {
    /// Resolve the global logger at emit time (lazy).
    Global,
    /// Bound to a specific logger (eager). Used by `Logger::handle()`.
    Direct(&'a Logger),
}

/// Builder for a log event. Call `.emit()` to dispatch.
///
/// Chain `.field()` calls to attach structured metadata:
///
/// ```
/// use std::sync::Arc;
/// use turborepo_log::{Logger, Source, Subsystem};
/// use turborepo_log::sinks::collector::CollectorSink;
///
/// let collector = Arc::new(CollectorSink::new());
/// let logger = Arc::new(Logger::new(vec![Box::new(collector.clone())]));
///
/// logger.handle(Source::turbo(Subsystem::Cache))
///     .warn("cache miss")
///     .field("task", "web#build")
///     .field("hash", "abc123")
///     .emit();
///
/// assert_eq!(collector.events().len(), 1);
/// ```
///
/// Events are **not** emitted automatically. You must call `.emit()`.
///
/// The global logger is resolved at `.emit()` time, not when the
/// builder is created. This means a builder created before [`init()`]
/// will dispatch correctly if `init()` is called before `.emit()`.
#[must_use = "log events are not emitted until .emit() is called"]
pub struct LogEventBuilder<'a> {
    event: LogEvent,
    resolver: LogResolver<'a>,
}

impl<'a> LogEventBuilder<'a> {
    /// Attach a structured field to this event.
    pub fn field(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.event.push_field(key, value.into());
        self
    }

    /// Attach a redacted field. The value is recorded as `[REDACTED]`
    /// and will never appear in log output.
    pub fn field_redacted(mut self, key: &'static str) -> Self {
        self.event.push_field(key, Value::Redacted);
        self
    }

    /// Dispatch the event to all registered sinks.
    ///
    /// For global-logger builders, resolution happens now — not when
    /// the builder was created. This is the lazy-resolution guarantee.
    pub fn emit(self) {
        let logger = match self.resolver {
            LogResolver::Global => GLOBAL_LOGGER.get().map(|arc| arc.as_ref()),
            LogResolver::Direct(l) => Some(l),
        };
        if let Some(logger) = logger {
            logger.emit(&self.event);
        }
    }
}

/// Emit a warning without creating a handle.
///
/// Returns a [`LogEventBuilder`] — chain `.field()` calls to attach
/// metadata, then call `.emit()`. The global logger is resolved at
/// `.emit()` time; if it has not been initialized, `.emit()` is a
/// no-op.
///
/// Prefer [`log()`] to create a reusable [`LogHandle`] when emitting
/// multiple events from the same source.
pub fn warn(source: Source, message: impl Into<String>) -> LogEventBuilder<'static> {
    LogEventBuilder {
        event: LogEvent::new(Level::Warn, source, message),
        resolver: LogResolver::Global,
    }
}

/// Emit an info message without creating a handle.
///
/// See [`warn()`] for full semantics — this function behaves
/// identically except it creates an `Info`-level event.
pub fn info(source: Source, message: impl Into<String>) -> LogEventBuilder<'static> {
    LogEventBuilder {
        event: LogEvent::new(Level::Info, source, message),
        resolver: LogResolver::Global,
    }
}

/// Emit an error without creating a handle.
///
/// See [`warn()`] for full semantics — this function behaves
/// identically except it creates an `Error`-level event.
pub fn error(source: Source, message: impl Into<String>) -> LogEventBuilder<'static> {
    LogEventBuilder {
        event: LogEvent::new(Level::Error, source, message),
        resolver: LogResolver::Global,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Subsystem, sinks::collector::CollectorSink};

    #[test]
    fn logger_dispatches_to_sinks() {
        let (collector, logger) = CollectorSink::with_logger();

        let event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "test warning");
        logger.emit(&event);

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "test warning");
        assert_eq!(events[0].level, Level::Warn);
    }

    #[test]
    fn logger_dispatches_to_multiple_sinks() {
        let c1 = Arc::new(CollectorSink::new());
        let c2 = Arc::new(CollectorSink::new());
        let logger = Logger::new(vec![Box::new(c1.clone()), Box::new(c2.clone())]);

        let event = LogEvent::new(Level::Error, Source::turbo(Subsystem::Cache), "broadcast");
        logger.emit(&event);

        assert_eq!(c1.events().len(), 1);
        assert_eq!(c2.events().len(), 1);
        assert_eq!(c1.events()[0].message, "broadcast");
    }

    #[test]
    fn log_handle_emits_via_builder() {
        let (collector, logger) = CollectorSink::with_logger();

        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        handle
            .warn("deprecated field")
            .field("name", "daemon")
            .emit();

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "deprecated field");
        assert_eq!(events[0].level, Level::Warn);
        assert_eq!(events[0].fields.len(), 1);
        assert_eq!(events[0].fields[0].0, "name");
    }

    #[test]
    fn builder_without_emit_does_not_dispatch() {
        let (collector, logger) = CollectorSink::with_logger();

        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        let _builder = handle.warn("should not appear");
        drop(_builder);
        assert_eq!(collector.events().len(), 0);
    }

    #[test]
    fn builder_field_chaining() {
        let (collector, logger) = CollectorSink::with_logger();

        logger
            .handle(Source::turbo(Subsystem::Cache))
            .warn("cache miss")
            .field("hash", "abc123")
            .field("task", "web#build")
            .field_redacted("token")
            .emit();

        let events = collector.events();
        assert_eq!(events[0].fields.len(), 3);
        assert_eq!(events[0].fields[0].0, "hash");
        assert_eq!(events[0].fields[2].1, Value::Redacted);
    }

    #[test]
    fn logger_with_no_sinks_does_not_panic() {
        let logger = Logger::new(vec![]);
        let event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "ignored");
        logger.emit(&event);
        logger.flush();
    }

    #[test]
    fn log_event_with_fields() {
        let (collector, logger) = CollectorSink::with_logger();

        let mut event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "cache miss");
        event.fields.push(("hash", Value::from("abc123")));
        event.fields.push(("task", Value::from("web#build")));
        logger.emit(&event);

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fields.len(), 2);
        assert_eq!(events[0].fields[0].0, "hash");
        assert!(matches!(&events[0].fields[0].1, Value::String(s) if s == "abc123"));
    }

    #[test]
    fn log_handle_all_levels() {
        let (collector, logger) = CollectorSink::with_logger();

        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        handle.info("info msg").emit();
        handle.warn("warn msg").emit();
        handle.error("error msg").emit();

        let events = collector.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].level, Level::Info);
        assert_eq!(events[1].level, Level::Warn);
        assert_eq!(events[2].level, Level::Error);
    }

    #[test]
    fn flush_propagates_to_sinks() {
        let (_, logger) = CollectorSink::with_logger();
        logger.flush();
    }

    #[test]
    fn cloned_handle_emits_to_same_sinks() {
        let (collector, logger) = CollectorSink::with_logger();
        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        let cloned = handle.clone();

        handle.warn("from original").emit();
        cloned.warn("from clone").emit();

        assert_eq!(collector.events().len(), 2);
    }

    #[test]
    fn logger_respects_sink_enabled_filter() {
        use crate::sink::LogSink;

        struct WarnAndAbove(Arc<CollectorSink>);

        impl LogSink for WarnAndAbove {
            fn emit(&self, event: &LogEvent) {
                self.0.emit(event);
            }

            fn enabled(&self, level: Level) -> bool {
                level >= Level::Warn
            }
        }

        let inner = Arc::new(CollectorSink::new());
        let sink = WarnAndAbove(inner.clone());
        let logger = Arc::new(Logger::new(vec![Box::new(sink)]));

        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        handle.info("should be filtered").emit();
        handle.warn("should pass").emit();
        handle.error("should pass").emit();

        assert_eq!(inner.events().len(), 2);
        assert_eq!(inner.events()[0].level, Level::Warn);
        assert_eq!(inner.events()[1].level, Level::Error);
    }

    #[test]
    fn logger_selective_dispatch_with_multiple_sinks() {
        use crate::sink::LogSink;

        struct ErrorOnly(Arc<CollectorSink>);

        impl LogSink for ErrorOnly {
            fn emit(&self, event: &LogEvent) {
                self.0.emit(event);
            }

            fn enabled(&self, level: Level) -> bool {
                level >= Level::Error
            }
        }

        let all_events = Arc::new(CollectorSink::new());
        let errors_only_inner = Arc::new(CollectorSink::new());
        let errors_only = ErrorOnly(errors_only_inner.clone());

        let logger = Arc::new(Logger::new(vec![
            Box::new(all_events.clone()),
            Box::new(errors_only),
        ]));

        let handle = logger.handle(Source::turbo(Subsystem::Cache));
        handle.info("info").emit();
        handle.warn("warn").emit();
        handle.error("error").emit();

        assert_eq!(all_events.events().len(), 3);
        assert_eq!(errors_only_inner.events().len(), 1);
        assert_eq!(errors_only_inner.events()[0].level, Level::Error);
    }

    #[test]
    fn logger_debug_shows_sink_count() {
        let logger = Logger::new(vec![]);
        let debug = format!("{logger:?}");
        assert!(debug.contains("sinks: 0"));
    }
}
