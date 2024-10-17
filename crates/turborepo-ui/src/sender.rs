use std::sync::{Arc, Mutex};

use crate::{
    tui,
    tui::event::{CacheResult, OutputLogs, PaneSize, TaskResult},
    wui::sender,
};

/// Enum to abstract over sending events to either the Tui or the Web UI
#[derive(Debug, Clone)]
pub enum UISender {
    Tui(tui::TuiSender),
    Wui(sender::WebUISender),
}

impl UISender {
    pub fn start_task(&self, task: String, output_logs: OutputLogs) {
        match self {
            UISender::Tui(sender) => sender.start_task(task, output_logs),
            UISender::Wui(sender) => sender.start_task(task, output_logs),
        }
    }

    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        match self {
            UISender::Tui(sender) => sender.restart_tasks(tasks),
            UISender::Wui(sender) => sender.restart_tasks(tasks),
        }
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        match self {
            UISender::Tui(sender) => sender.end_task(task, result),
            UISender::Wui(sender) => sender.end_task(task, result),
        }
    }

    pub fn status(&self, task: String, status: String, result: CacheResult) {
        match self {
            UISender::Tui(sender) => sender.status(task, status, result),
            UISender::Wui(sender) => sender.status(task, status, result),
        }
    }
    fn set_stdin(&self, task: String, stdin: Box<dyn std::io::Write + Send>) {
        match self {
            UISender::Tui(sender) => sender.set_stdin(task, stdin),
            UISender::Wui(sender) => sender.set_stdin(task, stdin),
        }
    }

    pub fn output(&self, task: String, output: Vec<u8>) -> Result<(), crate::Error> {
        match self {
            UISender::Tui(sender) => sender.output(task, output),
            UISender::Wui(sender) => sender.output(task, output),
        }
    }

    /// Construct a sender configured for a specific task
    pub fn task(&self, task: String) -> TaskSender {
        match self {
            UISender::Tui(sender) => sender.task(task),
            UISender::Wui(sender) => sender.task(task),
        }
    }
    pub async fn stop(&self) {
        match self {
            UISender::Tui(sender) => sender.stop().await,
            UISender::Wui(sender) => sender.stop(),
        }
    }
    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        match self {
            UISender::Tui(sender) => sender.update_tasks(tasks),
            UISender::Wui(sender) => sender.update_tasks(tasks),
        }
    }

    pub async fn pane_size(&self) -> Option<PaneSize> {
        match self {
            UISender::Tui(sender) => sender.pane_size().await,
            // Not applicable to the web UI
            UISender::Wui(_) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskSender {
    pub(crate) name: String,
    pub(crate) handle: UISender,
    pub(crate) logs: Arc<Mutex<Vec<u8>>>,
}

impl TaskSender {
    /// Access the underlying UISender
    pub fn as_app(&self) -> &UISender {
        &self.handle
    }

    /// Mark the task as started
    pub fn start(&self, output_logs: OutputLogs) {
        self.handle.start_task(self.name.clone(), output_logs);
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
        self.handle.end_task(self.name.clone(), result);
        self.logs.lock().expect("logs lock poisoned").clone()
    }

    pub fn set_stdin(&self, stdin: Box<dyn std::io::Write + Send>) {
        self.handle.set_stdin(self.name.clone(), stdin);
    }

    pub fn status(&self, status: &str, result: CacheResult) {
        // Since this will be rendered via ratatui we any ANSI escape codes will not be
        // handled.
        // TODO: prevent the status from having ANSI codes in this scenario
        let status = console::strip_ansi_codes(status).into_owned();
        self.handle.status(self.name.clone(), status, result);
    }
}

impl std::io::Write for TaskSender {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let task = self.name.clone();
        {
            self.logs
                .lock()
                .expect("log lock poisoned")
                .extend_from_slice(buf);
        }

        self.handle
            .output(task, buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "receiver dropped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
