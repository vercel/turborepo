//! `process`
//!
//! This module contains the code that is responsible for running the commands
//! that are queued by run. It consists of a set of child processes that are
//! spawned and managed by the manager. The manager is responsible for
//! running these processes to completion, forwarding signals, and closing
//! them when the manager is closed.
//!
//! As of now, the manager will execute futures in a random order, and
//! must be either `wait`ed on or `stop`ped to drive state.

mod child;

use std::time::Duration;

use tokio::task::JoinSet;
use tracing::{debug, trace};

pub struct Open(Vec<child::Child>);
pub struct Closed;

pub struct ProcessManager<T> {
    state: T,
}

impl ProcessManager<Closed> {
    pub fn new() -> Self {
        ProcessManager { state: Closed }
    }

    pub fn start(self) -> ProcessManager<Open> {
        ProcessManager {
            state: Open(Default::default()),
        }
    }
}

impl ProcessManager<Open> {
    /// Spawn a new child process to run the given command.
    ///
    /// Returns a oneshot receiver that will receive the exit code of the
    /// process when it exits. You do not need to wait for this receiver to
    /// complete, as the process is already being managed by the manager.
    pub fn spawn(&mut self, command: child::Command, timeout: Duration) -> child::Child {
        let mut child = child::Child::spawn(command, child::ShutdownStyle::Graceful(timeout));
        self.state.0.push(child.clone());
        child
    }

    /// Stop the process manager, closing all child processes. On posix
    /// systems this will send a SIGINT, and on windows it will send a
    /// CTRL_BREAK_EVENT.
    pub async fn stop(self) -> ProcessManager<Closed> {
        let mut set = JoinSet::new();

        for mut child in self.state.0.into_iter() {
            set.spawn(async move { child.stop().await });
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }

        ProcessManager { state: Closed }
    }

    /// Stop the process manager, waiting for all child processes to exit.
    ///
    /// If you want to set a timeout, use `tokio::time::timeout` and
    /// `Self::stop` if the timeout elapses.
    pub async fn wait(self) -> ProcessManager<Closed> {
        let mut set = JoinSet::new();

        for mut child in self.state.0.into_iter() {
            set.spawn(async move { child.wait().await });
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }

        ProcessManager { state: Closed }
    }
}

#[cfg(test)]
mod test {
    use tokio::process::Command;

    use super::*;

    fn get_command() -> Command {
        let mut cmd = Command::new("sleep");
        cmd.arg("1");
        cmd
    }

    #[tokio::test]
    async fn test_basic() {
        let mut manager = ProcessManager::new().start();
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_multiple() {
        let mut manager = ProcessManager::new().start();

        manager.spawn(get_command(), Duration::from_secs(2));
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.spawn(get_command(), Duration::from_secs(2));

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_closed() {
        let mut manager = ProcessManager::new().start();
        manager.spawn(get_command(), Duration::from_secs(2));

        let mut manager = manager.stop().await.start();
        manager.spawn(get_command(), Duration::from_secs(2));

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_exit_code() {
        let mut manager = ProcessManager::new().start();
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.stop().await;
    }
}
