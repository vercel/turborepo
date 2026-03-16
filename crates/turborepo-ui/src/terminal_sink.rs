use std::{
    io::{self, Write},
    sync::atomic::{AtomicU8, Ordering},
};

use turborepo_log::{Level, LogEvent, LogSink};

use crate::ColorConfig;

const MODE_DISABLED: u8 = 0;
const MODE_STDERR_ONLY: u8 = 1;
const MODE_ACTIVE: u8 = 2;

/// Routes [`LogEvent`]s to the appropriate file descriptor with color
/// styling:
///
/// | Level | Destination | Style |
/// |-------|-------------|-------|
/// | Info  | stdout      | grey, no badge |
/// | Warn  | stderr      | yellow, `WARNING` badge |
/// | Error | stderr      | red, `ERROR` badge |
///
/// Operates in three modes controlled by an [`AtomicU8`]:
///
/// - **Active** — all levels emit (stream mode, the default)
/// - **StderrOnly** — Info suppressed, Warn/Error still reach stderr (for
///   `--graph` / `--dry=json` where stdout carries structured data)
/// - **Disabled** — nothing emits (TUI owns the terminal)
///
/// On GitHub Actions, `Error` events also emit a `::error::` annotation
/// line before the formatted output so the runner can parse it.
pub struct TerminalSink {
    color_config: ColorConfig,
    mode: AtomicU8,
    ci_annotations: bool,
}

impl TerminalSink {
    pub fn new(color_config: ColorConfig) -> Self {
        let ci_annotations = std::env::var("GITHUB_ACTIONS")
            .ok()
            .filter(|v| !v.is_empty())
            .is_some();

        Self {
            color_config,
            mode: AtomicU8::new(MODE_ACTIVE),
            ci_annotations,
        }
    }

    /// Suppress all output. Called before the TUI takes ownership of
    /// the terminal.
    pub fn disable(&self) {
        self.mode.store(MODE_DISABLED, Ordering::Relaxed);
    }

    /// Resume all output (Info→stdout, Warn/Error→stderr). Called when
    /// the TUI didn't start and stream mode is active.
    pub fn enable(&self) {
        self.mode.store(MODE_ACTIVE, Ordering::Relaxed);
    }

    /// Suppress Info→stdout while keeping Warn/Error→stderr. Used for
    /// structured output modes (`--graph`, `--dry=json`) where stdout
    /// carries machine-readable data.
    pub fn suppress_stdout(&self) {
        self.mode.store(MODE_STDERR_ONLY, Ordering::Relaxed);
    }
}

impl LogSink for TerminalSink {
    fn enabled(&self, level: Level) -> bool {
        let mode = self.mode.load(Ordering::Relaxed);
        match mode {
            MODE_DISABLED => false,
            MODE_STDERR_ONLY => matches!(level, Level::Warn | Level::Error),
            MODE_ACTIVE => true,
            _ => false,
        }
    }

    fn emit(&self, event: &LogEvent) {
        let mode = self.mode.load(Ordering::Relaxed);
        if mode == MODE_DISABLED {
            return;
        }

        match event.level() {
            Level::Info => {
                if mode == MODE_STDERR_ONLY {
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
            Level::Warn | Level::Error => {
                self.emit_stderr(event);
            }
            _ => {}
        }
    }
}

impl TerminalSink {
    fn emit_stderr(&self, event: &LogEvent) {
        let stderr = io::stderr();
        let mut handle = stderr.lock();

        // GitHub Actions annotation — must start the line for
        // the runner to parse it as a workflow command.
        if self.ci_annotations && event.level() == Level::Error {
            let _ = writeln!(handle, "::error::{}", event.message());
        }

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
    fn emits_all_levels_without_panic() {
        let sink = TerminalSink::new(ColorConfig::new(true));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.info("test info").emit();
        handle.warn("test warning").emit();
        handle.error("test error").field("code", 1).emit();

        assert_eq!(collector.events().len(), 3);
    }

    #[test]
    fn active_mode_enables_all_levels() {
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));

        assert!(sink.enabled(Level::Info));
        assert!(sink.enabled(Level::Warn));
        assert!(sink.enabled(Level::Error));
    }

    #[test]
    fn stderr_only_mode_suppresses_info() {
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));
        sink.suppress_stdout();

        assert!(!sink.enabled(Level::Info));
        assert!(sink.enabled(Level::Warn));
        assert!(sink.enabled(Level::Error));
    }

    #[test]
    fn disabled_mode_suppresses_all() {
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));
        sink.disable();

        assert!(!sink.enabled(Level::Info));
        assert!(!sink.enabled(Level::Warn));
        assert!(!sink.enabled(Level::Error));
    }

    #[test]
    fn enable_restores_from_disabled() {
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));

        sink.disable();
        assert!(!sink.enabled(Level::Info));

        sink.enable();
        assert!(sink.enabled(Level::Info));
        assert!(sink.enabled(Level::Warn));
        assert!(sink.enabled(Level::Error));
    }

    #[test]
    fn disable_suppresses_emit() {
        let sink = Arc::new(TerminalSink::new(ColorConfig::new(true)));
        let collector = Arc::new(CollectorSink::new());
        let logger = Arc::new(Logger::new(vec![
            Box::new(sink.clone()),
            Box::new(collector.clone()),
        ]));

        let handle = logger.handle(Source::turbo("test"));
        handle.warn("before disable").emit();

        sink.disable();
        handle.warn("after disable").emit();

        // Both events reached the collector. TerminalSink's disable
        // only affects its own output.
        assert_eq!(collector.events().len(), 2);
    }
}
