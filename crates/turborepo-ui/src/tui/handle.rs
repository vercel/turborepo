use tokio::sync::{mpsc, oneshot};

use super::{
    app::FRAMERATE,
    event::{CacheResult, OutputLogs, PaneSize},
    Error, Event, TaskResult,
};
use crate::sender::{TaskSender, UISender};

/// Struct for sending app events to TUI rendering
#[derive(Debug, Clone)]
pub struct TuiSender {
    primary: mpsc::UnboundedSender<Event>,
}

/// Struct for receiving app events
pub struct AppReceiver {
    primary: mpsc::UnboundedReceiver<Event>,
}

impl TuiSender {
    /// Create a new channel for sending app events.
    ///
    /// AppSender is meant to be held by the actual task runner
    /// AppReceiver should be passed to `crate::tui::run_app`
    pub fn new() -> (Self, AppReceiver) {
        let (primary_tx, primary_rx) = mpsc::unbounded_channel();
        let tick_sender = primary_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(FRAMERATE);
            loop {
                interval.tick().await;
                if tick_sender.send(Event::Tick).is_err() {
                    break;
                }
            }
        });
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
    pub async fn stop(&self) {
        let (callback_tx, callback_rx) = oneshot::channel();
        // Send stop event, if receiver has dropped ignore error as
        // it'll be a no-op.
        self.primary.send(Event::Stop(callback_tx)).ok();
        // Wait for callback to be sent or the channel closed.
        callback_rx.await.ok();
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
    pub async fn pane_size(&self) -> Option<PaneSize> {
        let (callback_tx, callback_rx) = oneshot::channel();
        // Send query, if no receiver to handle the request return None
        self.primary.send(Event::PaneSizeQuery(callback_tx)).ok()?;
        // Wait for callback to be sent
        callback_rx.await.ok()
    }
}

impl AppReceiver {
    /// Receive an event, producing a tick event if no events are rec eived by
    /// the deadline.
    pub async fn recv(&mut self) -> Option<Event> {
        self.primary.recv().await
    }
}
