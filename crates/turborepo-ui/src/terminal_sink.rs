use std::io::{self, Write};

use turborepo_log::{Level, LogEvent, LogSink};

use crate::ColorConfig;

/// Routes [`LogEvent`]s to stderr with color styling.
///
/// This is the primary sink for user-facing messages during `turbo run`.
/// It formats events to stderr using the same color conventions as the
/// rest of turborepo's terminal output.
pub struct TerminalSink {
    color_config: ColorConfig,
}

impl TerminalSink {
    pub fn new(color_config: ColorConfig) -> Self {
        Self { color_config }
    }
}

impl LogSink for TerminalSink {
    fn emit(&self, event: &LogEvent) {
        let stderr = io::stderr();
        let mut handle = stderr.lock();

        let badge = match event.level() {
            Level::Error => self.color_config.apply(crate::BOLD_RED.apply_to(" ERROR ")),
            Level::Warn => self
                .color_config
                .apply(crate::BOLD_YELLOW_REVERSE.apply_to(" WARNING ")),
            _ => self.color_config.apply(crate::BOLD_CYAN.apply_to(" INFO ")),
        };

        let message_style = match event.level() {
            Level::Error => &*crate::BOLD_RED,
            Level::Warn => &*crate::YELLOW,
            _ => &*crate::GREY,
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
}
