//! Output handling for task execution.
//!
//! This module provides types for handling task output to both the terminal
//! and the TUI.

use turborepo_ui::sender::TaskSender;

/// Wrapper for TUI lifecycle signaling.
///
/// In stream mode (no TUI), this is `None` — lifecycle signals are not
/// needed since `TerminalSink` handles all rendering.
///
/// In TUI mode, this holds a `TaskSender` for start/succeeded/failed
/// lifecycle events that the TUI needs to manage task panes.
pub struct TaskOutput(Option<TaskSender>);

impl TaskOutput {
    pub fn stream() -> Self {
        Self(None)
    }

    pub fn tui(sender: TaskSender) -> Self {
        Self(Some(sender))
    }

    pub fn sender(&self) -> Option<&TaskSender> {
        self.0.as_ref()
    }

    /// Signal task completion to the TUI (if active).
    pub fn finish(self, is_error: bool, is_cache_hit: bool) {
        if let Some(sender) = self.0 {
            if is_error {
                sender.failed();
            } else {
                sender.succeeded(is_cache_hit);
            }
        }
    }

    /// Signal task start to the TUI (if active).
    pub fn start(&self, output_logs: turborepo_ui::tui::event::OutputLogs) {
        if let Some(sender) = &self.0 {
            sender.start(output_logs);
        }
    }

    /// Set stdin for interactive tasks (TUI only).
    pub fn set_stdin(&self, stdin: Box<dyn std::io::Write + Send>) {
        if let Some(sender) = &self.0 {
            sender.set_stdin(stdin);
        }
    }
}

// A tiny enum that allows us to use the same type for stdout and stderr without
// the use of Box<dyn Write>
pub enum StdWriter {
    Out(std::io::Stdout),
    Err(std::io::Stderr),
    Null(std::io::Sink),
}

impl StdWriter {
    fn writer(&mut self) -> &mut dyn std::io::Write {
        match self {
            StdWriter::Out(out) => out,
            StdWriter::Err(err) => err,
            StdWriter::Null(null) => null,
        }
    }
}

impl From<std::io::Stdout> for StdWriter {
    fn from(value: std::io::Stdout) -> Self {
        Self::Out(value)
    }
}

impl From<std::io::Stderr> for StdWriter {
    fn from(value: std::io::Stderr) -> Self {
        Self::Err(value)
    }
}

impl From<std::io::Sink> for StdWriter {
    fn from(value: std::io::Sink) -> Self {
        Self::Null(value)
    }
}

impl std::io::Write for StdWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer().flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    /// A writer that tracks whether it has been dropped. Used to verify
    /// that `set_stdin` in stream mode drops the stdin handle.
    struct DropTracker {
        dropped: Arc<AtomicBool>,
    }

    impl DropTracker {
        fn new() -> (Self, Arc<AtomicBool>) {
            let dropped = Arc::new(AtomicBool::new(false));
            (
                Self {
                    dropped: dropped.clone(),
                },
                dropped,
            )
        }
    }

    impl std::io::Write for DropTracker {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Drop for DropTracker {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn stream_mode_has_no_sender() {
        let output = TaskOutput::stream();
        assert!(output.sender().is_none());
    }

    /// Regression test for #12393: in stream mode, `set_stdin` has no TUI
    /// sender to forward stdin to, so the stdin handle is dropped. This is
    /// why the task executor must NOT pass stdin through `set_stdin` in
    /// stream mode — it must hold it in a guard instead.
    #[test]
    fn stream_mode_set_stdin_drops_handle() {
        let output = TaskOutput::stream();
        let (tracker, dropped) = DropTracker::new();

        assert!(!dropped.load(Ordering::SeqCst));
        output.set_stdin(Box::new(tracker));
        assert!(
            dropped.load(Ordering::SeqCst),
            "stdin should be dropped when set_stdin is called in stream mode"
        );
    }
}
