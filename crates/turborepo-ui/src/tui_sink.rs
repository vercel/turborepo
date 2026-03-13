use std::sync::Mutex;

use turborepo_log::{LogEvent, LogSink};

use crate::tui::TuiSender;

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
    Buffering(Vec<LogEvent>),
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
            state: Mutex::new(SinkState::Buffering(Vec::new())),
        }
    }

    /// Transition from buffering to connected. Drains all buffered
    /// events through the sender, then forwards directly from here on.
    pub fn connect(&self, sender: TuiSender) {
        let mut state = self.state.lock().unwrap();
        if let SinkState::Buffering(buffer) = &mut *state {
            for event in buffer.drain(..) {
                sender.log_event(event);
            }
        }
        *state = SinkState::Connected(sender);
    }
}

impl LogSink for TuiSink {
    fn emit(&self, event: &LogEvent) {
        let mut state = self.state.lock().unwrap();
        match &mut *state {
            SinkState::Buffering(buffer) => buffer.push(event.clone()),
            SinkState::Connected(sender) => {
                sender.log_event(event.clone());
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
        LogEvent::new(Level::Warn, Source::turbo("test"), msg)
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
            SinkState::Buffering(buf) => {
                assert_eq!(buf.len(), 2);
                assert_eq!(buf[0].message(), "first");
                assert_eq!(buf[1].message(), "second");
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
