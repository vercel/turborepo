use std::{
    sync::{mpsc, Arc, Mutex},
    time::Instant,
};

use super::{
    event::{CacheResult, OutputLogs},
    Event, TaskResult,
};

/// Struct for sending app events to TUI rendering
#[derive(Debug, Clone)]
pub struct AppSender {
    primary: mpsc::Sender<Event>,
}

/// Struct for receiving app events
pub struct AppReceiver {
    primary: mpsc::Receiver<Event>,
}

/// Struct for sending events related to a specific task
#[derive(Debug, Clone)]
pub struct TuiTask {
    name: String,
    handle: AppSender,
    logs: Arc<Mutex<Vec<u8>>>,
}

impl AppSender {
    /// Create a new channel for sending app events.
    ///
    /// AppSender is meant to be held by the actual task runner
    /// AppReceiver should be passed to `crate::tui::run_app`
    pub fn new() -> (Self, AppReceiver) {
        let (primary_tx, primary_rx) = mpsc::channel();
        (
            Self {
                primary: primary_tx,
            },
            AppReceiver {
                primary: primary_rx,
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
        let (callback_tx, callback_rx) = mpsc::sync_channel(1);
        // Send stop event, if receiver has dropped ignore error as
        // it'll be a no-op.
        self.primary.send(Event::Stop(callback_tx)).ok();
        // Wait for callback to be sent or the channel closed.
        callback_rx.recv().ok();
    }

    /// Update the list of tasks displayed in the TUI
    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), mpsc::SendError<Event>> {
        self.primary.send(Event::UpdateTasks { tasks })
    }

    /// Restart the list of tasks displayed in the TUI
    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), mpsc::SendError<Event>> {
        self.primary.send(Event::RestartTasks { tasks })
    }
}

impl AppReceiver {
    /// Receive an event, producing a tick event if no events are received by
    /// the deadline.
    pub fn recv(&self, deadline: Instant) -> Result<Event, mpsc::RecvError> {
        match self.primary.recv_deadline(deadline) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(Event::Tick),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
        }
    }
}

impl TuiTask {
    /// Access the underlying AppSender
    pub fn as_app(&self) -> &AppSender {
        &self.handle
    }

    /// Mark the task as started
    pub fn start(&self, output_logs: OutputLogs) {
        self.handle
            .primary
            .send(Event::StartTask {
                task: self.name.clone(),
                output_logs,
            })
            .ok();
    }

    /// Mark the task as finished
    pub fn succeeded(&self, is_cache_hit: bool) -> Vec<u8> {
        if is_cache_hit {
            self.finish(TaskResult::CacheHit)
        } else {
            self.finish(TaskResult::Success)
        }
    }

    /// Mark the task as finished
    pub fn failed(&self) -> Vec<u8> {
        self.finish(TaskResult::Failure)
    }

    fn finish(&self, result: TaskResult) -> Vec<u8> {
        self.handle
            .primary
            .send(Event::EndTask {
                task: self.name.clone(),
                result,
            })
            .ok();
        self.logs.lock().expect("logs lock poisoned").clone()
    }

    pub fn set_stdin(&self, stdin: Box<dyn std::io::Write + Send>) {
        self.handle
            .primary
            .send(Event::SetStdin {
                task: self.name.clone(),
                stdin,
            })
            .ok();
    }

    pub fn status(&self, status: &str, result: CacheResult) {
        // Since this will be rendered via ratatui we any ANSI escape codes will not be
        // handled.
        // TODO: prevent the status from having ANSI codes in this scenario
        let status = console::strip_ansi_codes(status).into_owned();
        self.handle
            .primary
            .send(Event::Status {
                task: self.name.clone(),
                status,
                result,
            })
            .ok();
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
