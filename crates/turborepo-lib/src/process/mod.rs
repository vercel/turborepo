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
mod shared_adaptor;

use std::time::Duration;

pub use child::Command;
pub use shared_adaptor::SharedProcessManager;
use tokio::task::JoinSet;
use tracing::{debug, trace};

/// A process manager that is responsible for spawning and managing child
/// processes. When the manager is Open, new child processes can be spawned
/// using `spawn`. When the manager is Closed, all currently-running children
/// will be closed, and no new children can be spawned.
#[derive(Debug)]
pub struct ProcessManager<T> {
    state: T,
}

#[derive(Debug)]
pub struct Open(Vec<child::Child>);
#[derive(Debug)]
pub struct Closed;

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
    /// The handle of the child can be either waited or stopped by the caller,
    /// as well as the entire process manager.
    pub fn spawn(&mut self, command: child::Command, stop_timeout: Duration) -> child::Child {
        let child = child::Child::spawn(command, child::ShutdownStyle::Graceful(stop_timeout));
        self.state.0.push(child.clone());
        child
    }

    /// Stop the process manager, closing all child processes. On posix
    /// systems this will send a SIGINT, and on windows it will just kill
    /// the process immediately.
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
    use tokio::{process::Command, time::sleep};
    use tracing_test::traced_test;

    use super::*;

    fn get_command() -> Command {
        let mut cmd = Command::new("node");
        cmd.arg("./test/scripts/sleep_5_interruptable.js");
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

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_closed() {
        let mut manager = ProcessManager::new().start();
        manager.spawn(get_command(), Duration::from_secs(2));

        let mut manager = manager.stop().await.start();
        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_exit_code() {
        let mut manager = ProcessManager::new().start();
        let mut child = manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        let code = child.wait().await;
        assert_eq!(code, Some(0));

        manager.stop().await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_after_stop() {
        let mut manager = ProcessManager::new().start();
        let mut child = manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        let code = child.wait().await;
        assert_eq!(code, Some(0));

        manager.stop().await;

        // this is idempotent, so calling it after the manager is stopped is ok
        child.kill().await;

        let code = child.wait().await;
        assert_eq!(code, None);
    }

    #[tokio::test]
    async fn test_reuse_manager() {
        let mut manager = ProcessManager::new().start();
        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        let mut manager = manager.stop().await.start();

        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;
    }
}
