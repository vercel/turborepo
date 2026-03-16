use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use turborepo_log::{Level, LogEvent, LogSink};

use crate::ColorConfig;

/// Routes [`LogEvent`]s at `Info` level to stdout with color styling.
///
/// This sink handles informational status output (like the run prelude)
/// that belongs on stdout. Warn/Error events are ignored — those go
/// through [`TerminalSink`](crate::TerminalSink) to stderr.
///
/// Disabled in TUI mode (the TUI owns the terminal) and by the caller
/// for structured output modes (`--dry=json`, `--graph`) where stdout
/// is reserved for machine-readable data.
pub struct StdoutSink {
    color_config: ColorConfig,
    active: AtomicBool,
}

impl StdoutSink {
    pub fn new(color_config: ColorConfig) -> Self {
        Self {
            color_config,
            active: AtomicBool::new(true),
        }
    }

    /// Stop emitting to stdout. Called before TUI startup takes ownership
    /// of the terminal, and for structured output modes (`--dry=json`,
    /// `--graph`) where stdout carries machine-readable data.
    pub fn disable(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    /// Resume emitting to stdout. Called when the TUI didn't start
    /// (stream/web mode) and stdout is available for status output.
    pub fn enable(&self) {
        self.active.store(true, Ordering::Relaxed);
    }
}

impl LogSink for StdoutSink {
    fn enabled(&self, level: Level) -> bool {
        level == Level::Info && self.active.load(Ordering::Relaxed)
    }

    fn emit(&self, event: &LogEvent) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }

        if !matches!(event.level(), Level::Info) {
            return;
        }

        let stdout = io::stdout();
        let mut handle = stdout.lock();
        let _ = writeln!(
            handle,
            "{}",
            self.color_config
                .apply(crate::GREY.apply_to(event.message()))
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use turborepo_log::{Logger, Source, sinks::collector::CollectorSink};

    use super::*;

    #[test]
    fn emits_info_without_panic() {
        let sink = StdoutSink::new(ColorConfig::new(true));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.info("test info").emit();
        handle.warn("test warning").emit();
        handle.error("test error").emit();

        // Collector receives all three; StdoutSink only processes Info.
        assert_eq!(collector.events().len(), 3);
    }

    #[test]
    fn filters_non_info_levels() {
        let sink = Arc::new(StdoutSink::new(ColorConfig::new(true)));

        assert!(sink.enabled(Level::Info));
        assert!(!sink.enabled(Level::Warn));
        assert!(!sink.enabled(Level::Error));
    }

    #[test]
    fn disable_suppresses_emit() {
        let sink = Arc::new(StdoutSink::new(ColorConfig::new(true)));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink.clone()),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.info("before disable").emit();

        sink.disable();
        handle.info("after disable").emit();

        // Both events reach the collector, but StdoutSink skips
        // the second one due to the active flag.
        assert_eq!(collector.events().len(), 2);

        // enabled() also reflects the disabled state.
        assert!(!sink.enabled(Level::Info));
    }

    #[test]
    fn enable_resumes_after_disable() {
        let sink = Arc::new(StdoutSink::new(ColorConfig::new(true)));

        sink.disable();
        assert!(!sink.enabled(Level::Info));

        sink.enable();
        assert!(sink.enabled(Level::Info));
    }
}
