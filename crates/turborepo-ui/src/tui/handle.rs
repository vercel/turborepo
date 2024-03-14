use std::{
    sync::{mpsc, Arc, Mutex},
    time::Instant,
};

use super::Event;

/// Struct for sending app events to TUI rendering
#[derive(Debug, Clone)]
pub struct AppSender {
    primary: mpsc::Sender<Event>,
    priority: mpsc::Sender<Event>,
}

/// Struct for receiving app events
pub struct AppReceiver {
    primary: mpsc::Receiver<Event>,
    priority: mpsc::Receiver<Event>,
}

/// Struct for sending events related to a specific task
#[derive(Debug, Clone)]
pub struct TuiTask {
    name: String,
    handle: AppSender,
    logs: Arc<Mutex<Vec<u8>>>,
}

/// Writer that will correctly render writes to the persisted part of the screen
#[derive(Debug, Clone)]
pub struct PersistedWriter {
    handle: AppSender,
}

impl AppSender {
    /// Create a new channel for sending app events.
    ///
    /// AppSender is meant to be held by the actual task runner
    /// AppReceiver should be passed to `crate::tui::run_app`
    pub fn new() -> (Self, AppReceiver) {
        let (primary_tx, primary_rx) = mpsc::channel();
        let (priority_tx, priority_rx) = mpsc::channel();
        (
            Self {
                primary: primary_tx,
                priority: priority_tx,
            },
            AppReceiver {
                primary: primary_rx,
                priority: priority_rx,
            },
        )
    }

    /// Construct a sender configured for a specific task
    pub fn task(&self, task: String) -> TuiTask {
        TuiTask {
            name: task,
            handle: self.clone(),
            logs: Default::default(),
        }
    }

    /// Stop rendering TUI and restore terminal to default configuration
    pub fn stop(&self) {
        // Send stop events in both channels, if receiver has dropped ignore error as
        // it'll be a no-op.
        self.priority.send(Event::Stop).ok();
        self.primary.send(Event::Stop).ok();
    }
}

impl AppReceiver {
    /// Receive an event, producing a tick event if no events are received by
    /// the deadline.
    pub fn recv(&self, deadline: Instant) -> Result<Event, mpsc::RecvError> {
        // If there's an event in the priority queue take from that first
        if let Ok(event) = self.priority.try_recv() {
            Ok(event)
        } else {
            match self.primary.recv_deadline(deadline) {
                Ok(event) => Ok(event),
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(Event::Tick),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
            }
        }
    }
}

impl TuiTask {
    /// Access the underlying AppSender
    pub fn as_app(&self) -> &AppSender {
        &self.handle
    }

    /// Mark the task as started
    pub fn start(&self) {
        self.handle
            .primary
            .send(Event::StartTask {
                task: self.name.clone(),
            })
            .ok();
    }

    /// Mark the task as finished
    pub fn finish(&self) -> Vec<u8> {
        self.handle
            .primary
            .send(Event::EndTask {
                task: self.name.clone(),
            })
            .ok();
        self.logs.lock().expect("logs lock poisoned").clone()
    }

    /// Return a `PersistedWriter` which will properly write provided bytes to
    /// a persisted section of the terminal.
    ///
    /// Designed to be a drop in replacement for `io::stdout()`,
    /// all calls such as `writeln!(io::stdout(), "hello")` should
    /// pass in a PersistedWriter instead.
    pub fn stdout(&self) -> PersistedWriter {
        PersistedWriter {
            handle: self.as_app().clone(),
        }
    }
}

impl std::io::Write for TuiTask {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let task = self.name.clone();
        {
            self.logs
                .lock()
                .expect("log lock poisoned")
                .extend_from_slice(buf);
        }
        self.handle
            .primary
            .send(Event::TaskOutput {
                task,
                output: buf.to_vec(),
            })
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "receiver dropped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Write for PersistedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let bytes = buf.to_vec();
        self.handle
            .primary
            .send(Event::Log { message: bytes })
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "receiver dropped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
