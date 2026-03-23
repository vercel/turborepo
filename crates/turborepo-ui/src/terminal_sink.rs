use std::{
    collections::HashMap,
    io::{self, Write},
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use turborepo_log::{Level, LogEvent, LogSink, OutputChannel, Source};

use crate::{ColorConfig, ColorSelector};

const MODE_DISABLED: u8 = 0;
const MODE_STDERR_ONLY: u8 = 1;
const MODE_ACTIVE: u8 = 2;

/// Per-task rendering state held by `TerminalSink`.
struct TaskRenderState {
    /// Pre-formatted ANSI-colored prefix (e.g., "\x1b[36mmy-app:build:
    /// \x1b[0m").
    prefix: String,
    /// Partial line buffer — bytes accumulate until `\n` before being written
    /// with the prefix prepended.
    line_buffer: Vec<u8>,
}

/// Routes [`LogEvent`]s and task output to the appropriate file
/// descriptor with color styling:
///
/// | Level | Destination | Style |
/// |-------|-------------|-------|
/// | Info  | stdout      | grey, no badge |
/// | Warn  | stderr      | yellow, `WARNING` badge |
/// | Error | stderr      | red, `ERROR` badge |
///
/// Task output bytes are rendered with a per-task colored prefix.
/// Call [`register_task`](LogSink::register_task) before a task
/// starts producing output so the sink can assign a color.
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
    color_selector: ColorSelector,
    tasks: Mutex<HashMap<String, TaskRenderState>>,
    include_timestamps: bool,
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
            color_selector: ColorSelector::default(),
            tasks: Mutex::new(HashMap::new()),
            include_timestamps: false,
        }
    }

    /// Enable timestamp prefixes on task output lines.
    pub fn with_timestamps(mut self, include: bool) -> Self {
        self.include_timestamps = include;
        self
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

    /// Generate the current prefix string for a task, optionally with
    /// timestamp.
    fn task_prefix(&self, base_prefix: &str) -> String {
        if self.include_timestamps {
            let timestamp = chrono::Local::now().format("%H:%M:%S%.3f");
            let grey_timestamp = self
                .color_config
                .apply(crate::GREY.apply_to(format!("[{timestamp}]")));
            format!("{grey_timestamp} {base_prefix}")
        } else {
            base_prefix.to_string()
        }
    }

    /// Write task output bytes to stdout with per-line prefix.
    ///
    /// Buffers partial lines. On each complete line (ending with `\n`),
    /// writes `prefix + line` to stdout. Handles `\r` for progress
    /// bars by rewriting the prefix at the start of the line.
    fn write_task_output(&self, task: &str, bytes: &[u8]) {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(state) = tasks.get_mut(task) else {
            // Task not registered — write raw bytes as fallback
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            let _ = handle.write_all(bytes);
            return;
        };

        let stdout = io::stdout();
        let mut handle = stdout.lock();

        // Split on newlines. For each complete line, write prefix + line.
        for line in bytes.split_inclusive(|c| *c == b'\n') {
            if line.ends_with(b"\n") {
                if state.line_buffer.is_empty() {
                    self.write_prefixed_line(&mut handle, &state.prefix, line);
                } else {
                    state.line_buffer.extend_from_slice(line);
                    let buffered = std::mem::take(&mut state.line_buffer);
                    self.write_prefixed_line(&mut handle, &state.prefix, &buffered);
                }
            } else {
                state.line_buffer.extend_from_slice(line);
            }
        }
    }

    /// Write a complete line with per-chunk prefix handling for `\r`.
    fn write_prefixed_line(&self, handle: &mut io::StdoutLock<'_>, base_prefix: &str, line: &[u8]) {
        let mut is_first = true;
        for chunk in line.split_inclusive(|c| *c == b'\r') {
            if is_first || chunk != b"\n" {
                let prefix = self.task_prefix(base_prefix);
                let _ = handle.write_all(prefix.as_bytes());
            }
            let _ = handle.write_all(chunk);
            is_first = false;
        }
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
                // Print the raw message — it may carry its own ANSI
                // formatting (e.g., summary colors). Don't wrap in GREY.
                let _ = writeln!(handle, "{}", event.message());
            }
            Level::Warn | Level::Error => {
                self.emit_stderr(event);
            }
            _ => {}
        }
    }

    fn task_output(&self, task: &str, _channel: OutputChannel, bytes: &[u8]) {
        if self.mode.load(Ordering::Relaxed) == MODE_DISABLED {
            return;
        }
        self.write_task_output(task, bytes);
    }

    fn register_task(&self, task: &str, prefix: &str) {
        let styled = self.color_selector.prefix_with_color(task, prefix);
        let formatted = self.color_config.apply(styled).to_string();
        let mut tasks = self.tasks.lock().unwrap();
        tasks.entry(task.to_string()).or_insert(TaskRenderState {
            prefix: formatted,
            line_buffer: Vec::with_capacity(512),
        });
    }

    fn begin_task_group(&self, task: &str, is_error: bool) {
        if self.mode.load(Ordering::Relaxed) == MODE_DISABLED {
            return;
        }
        if self.ci_annotations {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            if is_error {
                let _ = writeln!(handle, "\x1b[;31m{task}\x1b[;0m");
            } else {
                let _ = writeln!(handle, "::group::{task}");
            }
        }
    }

    fn end_task_group(&self, _task: &str, is_error: bool) {
        if self.mode.load(Ordering::Relaxed) == MODE_DISABLED {
            return;
        }
        // Error tasks use a red header instead of ::group::, so
        // emitting ::endgroup:: would be unpaired.
        if self.ci_annotations && !is_error {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            let _ = writeln!(handle, "::endgroup::");
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

        // Task-scoped events get the task ID prefix so the user
        // knows which task produced the warning/error.
        if let Source::Task(id) = event.source() {
            let _ = write!(
                handle,
                "{}",
                self.color_config
                    .apply(crate::BOLD.apply_to(format!("{id}: ")))
            );
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

        let handle = logger.handle(Source::turbo(turborepo_log::Subsystem::Cache));
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

        let handle = logger.handle(Source::turbo(turborepo_log::Subsystem::Cache));
        handle.warn("before disable").emit();

        sink.disable();
        handle.warn("after disable").emit();

        // Both events reached the collector. TerminalSink's disable
        // only affects its own output.
        assert_eq!(collector.events().len(), 2);
    }
}
