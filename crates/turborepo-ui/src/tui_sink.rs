use std::sync::Mutex;

use turborepo_log::{Level, LogEvent, LogSink, OutputChannel, Source};

use crate::tui::TuiSender;

/// Normalize lone `\n` to `\r\n` for the TUI's VT100 terminal emulator.
///
/// Already-correct `\r\n` sequences are left as-is.
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
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

/// Format a task-scoped log event as a string for the task output pane.
///
/// Produces output like `ERROR: command finished with error: exit code 1\r\n`
/// that will be rendered by the TUI's VT100 parser in the task pane.
fn format_task_event(event: &LogEvent) -> String {
    let badge = match event.level() {
        Level::Error => "ERROR: ",
        Level::Warn => "WARNING: ",
        Level::Info => "",
        _ => "",
    };
    format!("{badge}{}\r\n", event.message())
}

/// Routes [`LogEvent`]s into the TUI's event channel.
///
/// Created before the TUI starts, so it buffers events until
/// [`connect()`](Self::connect) is called with a live [`TuiSender`].
/// Buffered events are drained through the sender on connect.
///
/// If the TUI never starts (stream mode, terminal too small), the
/// buffer is simply unused.
pub struct TuiSink {
    state: Mutex<SinkState>,
}

enum SinkState {
    Buffering {
        events: Vec<LogEvent>,
        task_output: Vec<(String, Vec<u8>)>,
    },
    Connected(TuiSender),
}

impl Default for TuiSink {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiSink {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SinkState::Buffering {
                events: Vec::new(),
                task_output: Vec::new(),
            }),
        }
    }

    /// Transition from buffering to connected. Drains all buffered
    /// events and task output through the sender, then forwards
    /// directly from here on.
    pub fn connect(&self, sender: TuiSender) {
        let mut state = self.state.lock().unwrap();
        if let SinkState::Buffering {
            events,
            task_output,
        } = &mut *state
        {
            for event in events.drain(..) {
                sender.log_event(event);
            }
            for (task, bytes) in task_output.drain(..) {
                let _ = sender.output(task, bytes);
            }
        }
        *state = SinkState::Connected(sender);
    }
}

impl LogSink for TuiSink {
    fn emit(&self, event: &LogEvent) {
        let mut state = self.state.lock().unwrap();
        match &mut *state {
            SinkState::Buffering { events, .. } => events.push(event.clone()),
            SinkState::Connected(sender) => {
                // Task-scoped events go to the task's output pane so
                // they appear inline with the task's process output.
                // Non-task events go to the global log panel.
                if let Source::Task(id) = event.source() {
                    let formatted = format_task_event(event);
                    let _ = sender.output(id.to_string(), formatted.into_bytes());
                } else {
                    sender.log_event(event.clone());
                }
            }
        }
    }

    fn task_output(&self, task: &str, _channel: OutputChannel, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap();
        let normalized = normalize_newlines(bytes);
        match &mut *state {
            SinkState::Buffering { task_output, .. } => {
                task_output.push((task.to_string(), normalized));
            }
            SinkState::Connected(sender) => {
                let _ = sender.output(task.to_string(), normalized);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use turborepo_log::{Level, LogEvent, Source};

    use super::*;
    use crate::tui::event::Event;

    fn make_event(msg: &str) -> LogEvent {
        LogEvent::new(
            Level::Warn,
            Source::turbo(turborepo_log::Subsystem::Cache),
            msg,
        )
    }

    fn make_tui_sender() -> (TuiSender, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (TuiSender::new_for_test(tx), rx)
    }

    #[test]
    fn buffers_events_before_connect() {
        let sink = TuiSink::new();
        sink.emit(&make_event("first"));
        sink.emit(&make_event("second"));

        let state = sink.state.lock().unwrap();
        match &*state {
            SinkState::Buffering { events, .. } => {
                assert_eq!(events.len(), 2);
                assert_eq!(events[0].message(), "first");
                assert_eq!(events[1].message(), "second");
            }
            SinkState::Connected(_) => panic!("expected Buffering state"),
        }
    }

    #[test]
    fn connect_drains_buffer() {
        let sink = TuiSink::new();
        sink.emit(&make_event("buffered"));

        let (sender, mut rx) = make_tui_sender();
        sink.connect(sender);

        let event = rx.try_recv().expect("should have drained buffered event");
        match event {
            Event::LogEvent(e) => assert_eq!(e.message(), "buffered"),
            _ => panic!("expected LogEvent"),
        }
    }

    #[test]
    fn forwards_after_connect() {
        let sink = TuiSink::new();
        let (sender, mut rx) = make_tui_sender();
        sink.connect(sender);

        sink.emit(&make_event("live"));

        let event = rx.try_recv().expect("should have forwarded event");
        match event {
            Event::LogEvent(e) => assert_eq!(e.message(), "live"),
            _ => panic!("expected LogEvent"),
        }
    }

    #[test]
    fn works_behind_arc() {
        let sink = Arc::new(TuiSink::new());
        sink.emit(&make_event("arc event"));

        let (sender, mut rx) = make_tui_sender();
        sink.connect(sender);

        let event = rx.try_recv().expect("should have drained");
        match event {
            Event::LogEvent(e) => assert_eq!(e.message(), "arc event"),
            _ => panic!("expected LogEvent"),
        }
    }
}
