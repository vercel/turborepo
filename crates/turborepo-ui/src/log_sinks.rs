use std::sync::Arc;

use turborepo_log::{Logger, sinks::collector::CollectorSink};

use crate::{ColorConfig, StdoutSink, TerminalSink, TuiSink};

/// Owns the shared sink handles and coordinates their lifecycle.
///
/// Both `turbo run` and `turbo watch` need the same set of sinks wired
/// into the global logger. This struct is the single source of truth
/// for that setup so the init ceremony and enable/disable protocol
/// don't drift between call sites.
///
/// # Lifecycle
///
/// 1. `LogSinks::new()` — all sinks start enabled
/// 2. `init_logger()` — registers sinks with the global `turborepo_log` logger
/// 3. `disable_for_tui()` — suppress terminal + stdout before TUI startup
/// 4. If TUI starts: call `tui.connect(sender)` to forward buffered events If
///    TUI doesn't start: `enable_for_stream()` to re-enable output
/// 5. For `--dry=json` / `--graph`: call `stdout.disable()` directly (stdout is
///    the structured data channel)
pub struct LogSinks {
    pub terminal: Arc<TerminalSink>,
    pub stdout: Arc<StdoutSink>,
    pub tui: Arc<TuiSink>,
}

impl LogSinks {
    pub fn new(color_config: ColorConfig) -> Self {
        Self {
            terminal: Arc::new(TerminalSink::new(color_config)),
            stdout: Arc::new(StdoutSink::new(color_config)),
            tui: Arc::new(TuiSink::new()),
        }
    }

    /// Initialize the global logger with these sinks plus a fresh
    /// `CollectorSink`. Safe to call more than once — subsequent calls
    /// return `Err` but the original sinks remain active via `Arc`.
    pub fn init_logger(&self) {
        let collector = Arc::new(CollectorSink::new());
        // OnceLock-backed: first call wins, later calls are no-ops.
        // The Arc'd sinks survive regardless.
        let _ = turborepo_log::init(Logger::new(vec![
            Box::new(collector),
            Box::new(self.terminal.clone()),
            Box::new(self.stdout.clone()),
            Box::new(self.tui.clone()),
        ]));
    }

    /// Disable terminal and stdout sinks before TUI startup takes
    /// ownership of the terminal.
    pub fn disable_for_tui(&self) {
        self.terminal.disable();
        self.stdout.disable();
    }

    /// Re-enable terminal and stdout sinks when the TUI didn't start
    /// (stream mode, web mode, terminal too small).
    pub fn enable_for_stream(&self) {
        self.terminal.enable();
        self.stdout.enable();
    }
}
