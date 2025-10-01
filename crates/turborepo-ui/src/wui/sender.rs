use std::io::Write;

use tracing::log::warn;

use crate::{
    sender::{TaskSender, UISender},
    tui::event::{CacheResult, OutputLogs, TaskResult},
    wui::{Error, event::WebUIEvent},
};

#[derive(Debug, Clone)]
pub struct WebUISender {
    pub tx: tokio::sync::mpsc::UnboundedSender<WebUIEvent>,
}

impl WebUISender {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<WebUIEvent>) -> Self {
        Self { tx }
    }
    pub fn start_task(&self, task: String, output_logs: OutputLogs) {
        self.tx
            .send(WebUIEvent::StartTask { task, output_logs })
            .ok();
    }

    pub fn restart_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::RestartTasks { tasks })
            .map_err(Error::Broadcast)?;
        Ok(())
    }

    pub fn end_task(&self, task: String, result: TaskResult) {
        self.tx.send(WebUIEvent::EndTask { task, result }).ok();
    }

    pub fn status(&self, task: String, message: String, result: CacheResult) {
        self.tx
            .send(WebUIEvent::CacheStatus {
                task,
                message,
                result,
            })
            .ok();
    }

    pub fn set_stdin(&self, _: String, _: Box<dyn Write + Send>) {
        warn!("stdin is not supported (yet) in web ui");
    }

    pub fn task(&self, task: String) -> TaskSender {
        TaskSender {
            name: task,
            handle: UISender::Wui(self.clone()),
            logs: Default::default(),
        }
    }

    pub fn stop(&self) {
        self.tx.send(WebUIEvent::Stop).ok();
    }

    pub fn update_tasks(&self, tasks: Vec<String>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::UpdateTasks { tasks })
            .map_err(Error::Broadcast)?;

        Ok(())
    }

    pub fn output(&self, task: String, output: Vec<u8>) -> Result<(), crate::Error> {
        self.tx
            .send(WebUIEvent::TaskOutput { task, output })
            .map_err(Error::Broadcast)?;

        Ok(())
    }
}
