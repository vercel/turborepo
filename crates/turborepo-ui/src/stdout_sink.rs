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
/// Disabled in TUI mode (the TUI owns the terminal) and during
/// `--dry=json` (stdout is the structured data channel).
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

    pub fn disable(&self) {
        self.active.store(false, Ordering::Relaxed);
    }

    pub fn enable(&self) {
        self.active.store(true, Ordering::Relaxed);
    }
}

impl LogSink for StdoutSink {
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
