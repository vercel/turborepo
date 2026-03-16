use std::sync::Arc;

use turborepo_log::{Logger, sinks::collector::CollectorSink};

use crate::{ColorConfig, TerminalSink, TuiSink};

/// Owns the shared sink handles and coordinates their lifecycle.
///
/// Both `turbo run` and `turbo watch` need the same set of sinks wired
/// into the global logger. This struct is the single source of truth
/// for that setup so the init ceremony and enable/disable protocol
/// don't drift between call sites.
///
/// # Lifecycle
///
/// 1. `LogSinks::new()` — all sinks start in Active mode
/// 2. `init_logger()` — registers sinks with the global `turborepo_log` logger
/// 3. For `--graph` / `--dry=json`: `suppress_stdout()` before emitting prelude
/// 4. Emit prelude logs (Info→stdout, unless suppressed)
/// 5. `disable_for_tui()` — suppress all terminal output before TUI startup
/// 6. If TUI starts: `tui.connect(sender)` to forward buffered events If TUI
///    doesn't start: `enable_for_stream()` to re-enable output
pub struct LogSinks {
    pub terminal: Arc<TerminalSink>,
    pub tui: Arc<TuiSink>,
}

impl LogSinks {
    pub fn new(color_config: ColorConfig) -> Self {
        Self {
            terminal: Arc::new(TerminalSink::new(color_config)),
            tui: Arc::new(TuiSink::new()),
        }
    }

    /// Initialize the global logger with these sinks plus a fresh
    /// `CollectorSink`. Safe to call more than once — subsequent calls
    /// return `Err` but the original sinks remain active via `Arc`.
    pub fn init_logger(&self) {
        let collector = Arc::new(CollectorSink::new());
        let _ = turborepo_log::init(Logger::new(vec![
            Box::new(collector),
            Box::new(self.terminal.clone()),
            Box::new(self.tui.clone()),
        ]));
    }

    /// Suppress Info→stdout while keeping Warn/Error→stderr. Used for
    /// structured output modes where stdout carries machine-readable data.
    pub fn suppress_stdout(&self) {
        self.terminal.suppress_stdout();
    }

    /// Disable all terminal output before TUI startup takes ownership
    /// of the terminal.
    pub fn disable_for_tui(&self) {
        self.terminal.disable();
    }

    /// Re-enable all terminal output when the TUI didn't start
    /// (stream mode, web mode, terminal too small).
    pub fn enable_for_stream(&self) {
        self.terminal.enable();
    }
}
