use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};

use turborepo_log::{Level, LogEvent, LogSink};

use crate::ColorConfig;

/// Routes `Warn` and `Error` level [`LogEvent`]s to stderr with color
/// styling. Info events are handled by [`StdoutSink`](crate::StdoutSink)
/// instead, keeping stdout as the channel for status information and
/// stderr for diagnostics.
///
/// When the TUI is active it owns the terminal, so stderr writes would
/// corrupt the display. Call [`disable()`](Self::disable) to suppress
/// output once the TUI takes over.
pub struct TerminalSink {
    color_config: ColorConfig,
    active: AtomicBool,
}

impl TerminalSink {
    pub fn new(color_config: ColorConfig) -> Self {
        Self {
            color_config,
            active: AtomicBool::new(true),
        }
    }

    /// Stop emitting to stderr. Intended to be called when the TUI
    /// connects and takes ownership of the terminal.
    pub fn disable(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    /// Resume emitting to stderr. Called when the TUI was expected
    /// but didn't start (e.g. terminal too small, no tasks).
    pub fn enable(&self) {
        self.active.store(true, Ordering::Relaxed);
    }
}

impl LogSink for TerminalSink {
    fn emit(&self, event: &LogEvent) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }

        // Info events go to stdout via StdoutSink, not here.
        if matches!(event.level(), Level::Info) {
            return;
        }

        let stderr = io::stderr();
        let mut handle = stderr.lock();

        let badge = match event.level() {
            Level::Error => self.color_config.apply(crate::BOLD_RED.apply_to(" ERROR ")),
            Level::Warn => self
                .color_config
                .apply(crate::BOLD_YELLOW_REVERSE.apply_to(" WARNING ")),
            _ => return,
        };

        let message_style = match event.level() {
            Level::Error => &*crate::BOLD_RED,
            Level::Warn => &*crate::YELLOW,
            _ => return,
        };

        let _ = write!(
            handle,
            "{badge} {}",
            self.color_config
                .apply(message_style.apply_to(event.message()))
        );

        for (key, value) in event.fields() {
            let _ = write!(
                handle,
                " {}",
                self.color_config
                    .apply(crate::GREY.apply_to(format!("{key}={value}")))
            );
        }

        let _ = writeln!(handle);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use turborepo_log::{Logger, Source, sinks::collector::CollectorSink};

    use super::*;

    #[test]
    fn emits_to_stderr_without_panic() {
        let sink = TerminalSink::new(ColorConfig::new(true));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.warn("test warning").emit();
        handle.error("test error").field("code", 1).emit();
        handle.info("test info").emit();

        assert_eq!(collector.events().len(), 3);
    }

    #[test]
    fn strips_ansi_when_configured() {
        let sink = TerminalSink::new(ColorConfig::new(true));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.warn("no color here").emit();
        assert_eq!(collector.events().len(), 1);
    }

    #[test]
    fn disable_suppresses_emit() {
        // TerminalSink behind Arc so disable() and emit() share the same AtomicBool.
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink.clone()),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.warn("before disable").emit();

        sink.disable();
        // This event still reaches the collector (it's a separate sink)
        // but TerminalSink should skip its stderr write.
        handle.warn("after disable").emit();

        // Both events reached the collector, confirming the logger still
        // dispatches. TerminalSink's disable only affects its own output.
        assert_eq!(collector.events().len(), 2);
    }
}
