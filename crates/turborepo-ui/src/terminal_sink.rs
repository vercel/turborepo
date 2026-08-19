use std::{
    collections::HashMap,
    io::{self, Write},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use turborepo_log::{Level, LogEvent, LogSink, OutputChannel, Source};

use crate::{ColorConfig, ColorSelector};

/// Normalize lone `\n` to `\r\n`.
///
/// Already-correct `\r\n` sequences are left as-is. Used by [`TerminalSink`]
/// when streaming under raw mode (where a lone `\n` would staircase the
/// output) and by the TUI's virtual terminal emulator.
pub(crate) fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut prev_cr = false;
    for &b in bytes {
        if b == b'\n' && !prev_cr {
            result.push(b'\r');
        }
        result.push(b);
        prev_cr = b == b'\r';
    }
    result
}

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

/// How the single-task stream filter treats a task's output.
#[derive(Debug, PartialEq, Eq)]
enum StreamDisposition {
    /// Another task is being streamed; drop this output.
    Suppress,
    /// This is the single streamed task; write its bytes exactly as the TUI
    /// pane would show them, with no per-line prefix.
    Verbatim,
    /// No filter is active; write with the usual per-task colored prefix.
    Prefixed,
}

/// Decide how to render `task`'s output given the active stream filter.
fn stream_disposition(filter: Option<String>, task: &str) -> StreamDisposition {
    match filter.as_deref() {
        Some(selected) if selected == task => StreamDisposition::Verbatim,
        Some(_) => StreamDisposition::Suppress,
        None => StreamDisposition::Prefixed,
    }
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
    /// When `true`, the terminal is in raw mode (the TUI owns input while
    /// the user has switched to streamed logs). Output must then convert
    /// lone `\n` to `\r\n` to avoid staircased text.
    raw_terminal: AtomicBool,
    /// When `Some(task)`, only output for that task is emitted, verbatim
    /// (no per-task prefix), exactly as the TUI pane shows it. Used when
    /// the user streams a single selected task's logs. `None` streams all
    /// tasks with prefixes.
    stream_filter: Mutex<Option<String>>,
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
            raw_terminal: AtomicBool::new(false),
            stream_filter: Mutex::new(None),
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

    /// Toggle raw-mode output handling. While the TUI owns terminal input
    /// (raw mode), lone `\n` is converted to `\r\n` so streamed output
    /// doesn't staircase.
    pub fn set_raw_terminal(&self, raw: bool) {
        self.raw_terminal.store(raw, Ordering::Relaxed);
    }

    /// Restrict streamed output to a single task (`Some`) or all tasks
    /// (`None`). Used when the user streams only the selected task's logs.
    pub fn set_stream_filter(&self, task: Option<String>) {
        *self
            .stream_filter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = task;
    }

    /// Returns `true` if output for `task` should be suppressed by the
    /// current single-task stream filter.
    fn filtered_out(&self, task: &str) -> bool {
        stream_disposition(self.stream_filter(), task) == StreamDisposition::Suppress
    }

    fn stream_filter(&self) -> Option<String> {
        self.stream_filter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Write `bytes` to `handle`, converting lone `\n` to `\r\n` when the
    /// terminal is in raw mode.
    fn write_bytes(&self, handle: &mut impl Write, bytes: &[u8]) {
        if self.raw_terminal.load(Ordering::Relaxed) {
            let _ = handle.write_all(&normalize_newlines(bytes));
        } else {
            let _ = handle.write_all(bytes);
        }
    }

    /// Generate the current prefix string for a task.
    fn task_prefix(&self, base_prefix: &str) -> String {
        base_prefix.to_string()
    }

    /// Write task output bytes to stdout with per-line prefix.
    ///
    /// Buffers partial lines. On each complete line (ending with `\n`),
    /// writes `prefix + line` to stdout. Handles `\r` for progress
    /// bars by rewriting the prefix at the start of the line.
    fn write_task_output(&self, task: &str, bytes: &[u8]) {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = tasks.get_mut(task) else {
            // Task not registered — write raw bytes as fallback
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            self.write_bytes(&mut handle, bytes);
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
            self.write_bytes(handle, chunk);
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

        // When a single-task stream filter is active, suppress task-scoped
        // events from other tasks. Non-task (turbo-level) events still pass.
        if let Source::Task(id) = event.source()
            && self.filtered_out(id)
        {
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
                self.write_bytes(&mut handle, format!("{}\n", event.message()).as_bytes());
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
        match stream_disposition(self.stream_filter(), task) {
            StreamDisposition::Suppress => {}
            // Single-task streaming shows the task's output verbatim —
            // no per-line prefix — matching what the TUI pane displays.
            StreamDisposition::Verbatim => {
                let stdout = io::stdout();
                let mut handle = stdout.lock();
                self.write_bytes(&mut handle, bytes);
            }
            StreamDisposition::Prefixed => self.write_task_output(task, bytes),
        }
    }

    fn register_task(&self, task: &str, prefix: &str) {
        let styled = self.color_selector.prefix_with_color(task, prefix);
        let formatted = self.color_config.apply(styled).to_string();
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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

        // Build the output up front so it can be newline-normalized in one
        // pass before writing (important under raw mode).
        let mut line = String::new();

        // GitHub Actions annotation — must start the line for
        // the runner to parse it as a workflow command.
        if self.ci_annotations && event.level() == Level::Error {
            line.push_str(&format!("::error::{}\n", event.message()));
        }

        // Task-scoped events get the task ID prefix so the user
        // knows which task produced the warning/error.
        if let Source::Task(id) = event.source() {
            line.push_str(
                &self
                    .color_config
                    .apply(crate::BOLD.apply_to(format!("{id}: ")))
                    .to_string(),
            );
        }

        line.push_str(&format!(
            "{badge} {}",
            self.color_config
                .apply(message_style.apply_to(event.message()))
        ));

        for (key, value) in event.fields() {
            line.push_str(&format!(
                " {}",
                self.color_config
                    .apply(crate::GREY.apply_to(format!("{key}={value}")))
            ));
        }

        line.push('\n');

        let stderr = io::stderr();
        let mut handle = stderr.lock();
        self.write_bytes(&mut handle, line.as_bytes());
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
    fn stream_filter_suppresses_other_tasks() {
        let sink = TerminalSink::new(ColorConfig::new(true));
        assert!(!sink.filtered_out("a"), "no filter passes all tasks");

        sink.set_stream_filter(Some("a".to_string()));
        assert!(!sink.filtered_out("a"), "selected task passes");
        assert!(sink.filtered_out("b"), "other tasks are suppressed");

        sink.set_stream_filter(None);
        assert!(!sink.filtered_out("b"), "clearing the filter restores all");
    }

    #[test]
    fn single_task_stream_is_verbatim_and_all_tasks_are_prefixed() {
        assert_eq!(
            stream_disposition(None, "a"),
            StreamDisposition::Prefixed,
            "no filter streams every task with its prefix"
        );
        assert_eq!(
            stream_disposition(Some("a".to_string()), "a"),
            StreamDisposition::Verbatim,
            "the selected task's output is shown exactly as in the TUI"
        );
        assert_eq!(
            stream_disposition(Some("a".to_string()), "b"),
            StreamDisposition::Suppress,
            "other tasks are dropped while a single task is streamed"
        );
    }

    #[test]
    fn write_bytes_normalizes_newlines_under_raw_mode() {
        let sink = TerminalSink::new(ColorConfig::new(true));

        let mut buf = Vec::new();
        sink.write_bytes(&mut buf, b"a\nb\n");
        assert_eq!(buf, b"a\nb\n", "no normalization when not in raw mode");

        sink.set_raw_terminal(true);
        let mut buf = Vec::new();
        sink.write_bytes(&mut buf, b"a\nb\r\n");
        assert_eq!(
            buf, b"a\r\nb\r\n",
            "lone \\n becomes \\r\\n, existing \\r\\n untouched"
        );
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
