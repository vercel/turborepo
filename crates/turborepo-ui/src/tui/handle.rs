use tokio::sync::{mpsc, oneshot};

use super::{
    Error, Event, TaskResult,
    app::FRAMERATE,
    event::{CacheResult, OutputLogs, PaneSize},
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
        Self::with_framerate(FRAMERATE)
    }

    fn with_framerate(framerate: std::time::Duration) -> (Self, AppReceiver) {
        let (primary_tx, primary_rx) = mpsc::unbounded_channel();
        let tick_sender = primary_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(framerate);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
    /// Test-only constructor that wraps an existing channel sender
    /// without spawning a tick task.
    #[cfg(test)]
    pub(crate) fn new_for_test(sender: mpsc::UnboundedSender<Event>) -> Self {
        Self { primary: sender }
    }

    pub fn log_event(&self, event: turborepo_log::LogEvent) {
        self.primary.send(Event::LogEvent(event)).ok();
    }

    pub fn start_task(&self, task: String, output_logs: OutputLogs) {
        self.primary
            .send(Event::StartTask { task, output_logs })
            .ok();
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        self.primary.send(Event::EndTask { task, result }).ok();
    }

    pub fn status(
        &self,
        task: String,
        status: String,
        result: CacheResult,
        output_logs: OutputLogs,
    ) {
        self.primary
            .send(Event::Status {
                task,
                status,
                result,
                output_logs,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn ticks_per_second(framerate: Duration) -> usize {
        let (_sender, mut receiver) = TuiSender::with_framerate(framerate);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut ticks = 0;
        while matches!(receiver.primary.try_recv(), Ok(Event::Tick)) {
            ticks += 1;
        }
        ticks
    }

    #[tokio::test]
    #[ignore = "benchmark; run with cargo test -p turborepo-ui --features tui \
                tick_cadence_is_bounded_to_60hz -- --ignored --nocapture"]
    async fn tick_cadence_is_bounded_to_60hz() {
        let old_ticks = ticks_per_second(Duration::from_millis(3)).await;
        let ticks = ticks_per_second(FRAMERATE).await;
        println!(
            "3 ms cadence: {old_ticks} ticks/s; 16 ms cadence: {ticks} ticks/s; {:.1}x fewer",
            old_ticks as f64 / ticks as f64
        );
        assert!(
            old_ticks >= 300,
            "3 ms cadence unexpectedly emitted {old_ticks} ticks"
        );
        assert!(ticks <= 64, "16 ms cadence emitted {ticks} ticks");
    }
}

impl AppReceiver {
    pub fn close(&mut self) {
        self.primary.close();
    }

    /// Receive an event, producing a tick event if no events are rec eived by
    /// the deadline.
    pub async fn recv(&mut self) -> Option<Event> {
        self.primary.recv().await
    }
}
