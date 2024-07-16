use std::{sync::mpsc, time::Instant};

use super::{
    event::{CacheResult, OutputLogs, PaneSize},
    Error, Event, TaskResult,
};
use crate::sender::{TaskSender, UISender};

/// Struct for sending app events to TUI rendering
#[derive(Debug, Clone)]
pub struct TuiSender {
    primary: mpsc::Sender<Event>,
}

/// Struct for receiving app events
pub struct AppReceiver {
    primary: mpsc::Receiver<Event>,
}

impl TuiSender {
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
}

impl TuiSender {
    pub fn start_task(&self, task: String, output_logs: OutputLogs) {
        self.primary
            .send(Event::StartTask { task, output_logs })
            .ok();
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        self.primary.send(Event::EndTask { task, result }).ok();
    }

    pub fn status(&self, task: String, status: String, result: CacheResult) {
        self.primary
            .send(Event::Status {
                task,
                status,
                result,
            })
            .ok();
    }

    pub fn set_stdin(&self, task: String, stdin: Box<dyn std::io::Write + Send>) {
        self.primary.send(Event::SetStdin { task, stdin }).ok();
    }

    /// Construct a sender configured for a specific task
    pub fn task(&self, task: String) -> TaskSender {
        TaskSender {
            name: task,
            handle: UISender::Tui(self.clone()),
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
    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        Ok(self
            .primary
            .send(Event::UpdateTasks { tasks })
            .map_err(|err| Error::Mpsc(err.to_string()))?)
    }

    pub fn output(&self, task: String, output: Vec<u8>) -> Result<(), crate::Error> {
        Ok(self
            .primary
            .send(Event::TaskOutput { task, output })
            .map_err(|err| Error::Mpsc(err.to_string()))?)
    }

    /// Restart the list of tasks displayed in the TUI
    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        Ok(self
            .primary
            .send(Event::RestartTasks { tasks })
            .map_err(|err| Error::Mpsc(err.to_string()))?)
    }

    /// Fetches the size of the terminal pane
    pub fn pane_size(&self) -> Option<PaneSize> {
        let (callback_tx, callback_rx) = mpsc::sync_channel(1);
        // Send query, if no receiver to handle the request return None
        self.primary.send(Event::PaneSizeQuery(callback_tx)).ok()?;
        // Wait for callback to be sent
        callback_rx.recv().ok()
    }
}

impl AppReceiver {
    /// Receive an event, producing a tick event if no events are rec eived by
    /// the deadline.
    pub fn recv(&self, deadline: Instant) -> Result<Event, mpsc::RecvError> {
        match self.primary.recv_deadline(deadline) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(Event::Tick),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
        }
    }
}
