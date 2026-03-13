use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use crate::{
    Logger,
    event::{Level, LogEvent},
    sink::LogSink,
};

const DEFAULT_MAX_EVENTS: usize = 10_000;

/// Collects log events in memory for retrieval after a run.
///
/// Used for post-run warning/error summaries. Thread-safe via internal
/// `Mutex`. Share across threads with `Arc<CollectorSink>`.
///
/// Events beyond the capacity limit are silently dropped and counted
/// via [`dropped_count()`](Self::dropped_count). Use [`drain()`](Self::drain)
/// to clear the buffer periodically for long-running processes.
pub struct CollectorSink {
    events: Mutex<Vec<LogEvent>>,
    max_events: usize,
    dropped: AtomicU64,
}

impl CollectorSink {
    /// Create a collector with the default capacity (10,000 events).
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            max_events: DEFAULT_MAX_EVENTS,
            dropped: AtomicU64::new(0),
        }
    }

    /// Create a collector with a specific capacity limit.
    ///
    /// Passing `0` creates a sink that drops all events (useful for
    /// disabling collection without changing the sink plumbing).
    /// Events beyond the capacity are silently dropped and counted.
    #[must_use]
    pub fn with_capacity(max_events: usize) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            max_events,
            dropped: AtomicU64::new(0),
        }
    }

    /// Create a collector and a logger wired to it.
    ///
    /// Convenience for tests — avoids repeating the `Arc::new` +
    /// `Box::new` boilerplate.
    pub fn with_logger() -> (Arc<Self>, Arc<Logger>) {
        let collector = Arc::new(Self::new());
        let logger = Arc::new(Logger::new(vec![Box::new(collector.clone())]));
        (collector, logger)
    }

    /// Return a clone of all collected events.
    ///
    /// This clones every event while holding the internal lock.
    /// For large buffers, prefer [`with_events`](Self::with_events)
    /// or [`drain`](Self::drain).
    pub fn events(&self) -> Vec<LogEvent> {
        self.lock().clone()
    }

    /// Access collected events without cloning.
    pub fn with_events<R>(&self, f: impl FnOnce(&[LogEvent]) -> R) -> R {
        f(&self.lock())
    }

    /// Return events at or above the given severity.
    ///
    /// For example, `events_at_severity(Level::Warn)` returns `Warn` and
    /// `Error` events, excluding `Info`.
    pub fn events_at_severity(&self, min_severity: Level) -> Vec<LogEvent> {
        self.lock()
            .iter()
            .filter(|e| e.level >= min_severity)
            .cloned()
            .collect()
    }

    /// Drain all collected events, clearing the internal buffer.
    pub fn drain(&self) -> Vec<LogEvent> {
        std::mem::take(&mut *self.lock())
    }

    /// Number of events that were dropped because the buffer was full.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    // Recover from poisoned mutex — logging must not propagate panics
    // from other threads.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<LogEvent>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for CollectorSink {
    fn default() -> Self {
        Self::new()
    }
}

impl LogSink for CollectorSink {
    fn emit(&self, event: &LogEvent) {
        let mut events = self.lock();
        if events.len() < self.max_events {
            events.push(event.clone());
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Source;

    #[test]
    fn stores_events() {
        let collector = CollectorSink::new();
        collector.emit(&LogEvent::new(
            Level::Warn,
            Source::turbo("test"),
            "warning 1",
        ));
        collector.emit(&LogEvent::new(
            Level::Error,
            Source::turbo("test"),
            "error 1",
        ));
        assert_eq!(collector.events().len(), 2);
    }

    #[test]
    fn filter_by_severity() {
        let collector = CollectorSink::new();
        collector.emit(&LogEvent::new(Level::Info, Source::turbo("t"), "info"));
        collector.emit(&LogEvent::new(Level::Warn, Source::turbo("t"), "warn"));
        collector.emit(&LogEvent::new(Level::Error, Source::turbo("t"), "error"));

        let warnings_and_above = collector.events_at_severity(Level::Warn);
        assert_eq!(warnings_and_above.len(), 2);
        assert!(warnings_and_above.iter().all(|e| e.level >= Level::Warn));
        assert!(warnings_and_above.iter().any(|e| e.level == Level::Warn));
        assert!(warnings_and_above.iter().any(|e| e.level == Level::Error));

        let errors_only = collector.events_at_severity(Level::Error);
        assert_eq!(errors_only.len(), 1);
        assert_eq!(errors_only[0].level, Level::Error);

        let all = collector.events_at_severity(Level::Info);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn drain_clears_buffer() {
        let collector = CollectorSink::new();
        collector.emit(&LogEvent::new(Level::Warn, Source::turbo("test"), "msg"));
        let drained = collector.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(collector.events().len(), 0);
    }

    #[test]
    fn respects_capacity_limit() {
        let collector = CollectorSink::with_capacity(3);
        for i in 0..5 {
            collector.emit(&LogEvent::new(
                Level::Warn,
                Source::turbo("test"),
                format!("event {i}"),
            ));
        }
        assert_eq!(collector.events().len(), 3);
        assert_eq!(collector.dropped_count(), 2);
    }

    #[test]
    fn with_capacity_zero_drops_all_events() {
        let collector = CollectorSink::with_capacity(0);
        collector.emit(&LogEvent::new(Level::Warn, Source::turbo("t"), "msg"));
        assert_eq!(collector.events().len(), 0);
        assert_eq!(collector.dropped_count(), 1);
    }

    #[test]
    fn with_events_borrows_without_cloning() {
        let collector = CollectorSink::new();
        collector.emit(&LogEvent::new(Level::Info, Source::turbo("test"), "msg"));
        collector.with_events(|events| {
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].message, "msg");
        });
    }

    #[test]
    fn default_is_empty() {
        let collector = CollectorSink::default();
        assert_eq!(collector.events().len(), 0);
        assert_eq!(collector.dropped_count(), 0);
    }

    #[test]
    fn with_logger_creates_wired_pair() {
        let (collector, logger) = CollectorSink::with_logger();
        let handle = logger.handle(Source::turbo("test"));
        handle.warn("via helper").emit();
        assert_eq!(collector.events().len(), 1);
    }

    #[test]
    fn concurrent_access() {
        let collector = std::sync::Arc::new(CollectorSink::new());
        let mut handles = vec![];
        for i in 0..10 {
            let c = std::sync::Arc::clone(&collector);
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    c.emit(&LogEvent::new(
                        Level::Warn,
                        Source::turbo("test"),
                        format!("thread {i} event {j}"),
                    ));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(collector.events().len(), 1000);
    }
}
