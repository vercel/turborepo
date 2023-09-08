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

use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Duration,
};

pub use child::Command;
use tokio::task::JoinSet;
use tracing::{debug, trace};

/// A process manager that is responsible for spawning and managing child
/// processes. When the manager is Open, new child processes can be spawned
/// using `spawn`. When the manager is Closed, all currently-running children
/// will be closed, and no new children can be spawned.
#[derive(Debug, Clone)]
pub struct ProcessManager {
    is_closing: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<child::Child>>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        ProcessManager {
            is_closing: Arc::new(AtomicBool::new(false)),
            children: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ProcessManager {
    /// Spawn a new child process to run the given command.
    ///
    /// The handle of the child can be either waited or stopped by the caller,
    /// as well as the entire process manager.
    ///
    /// If spawn returns None,
    pub fn spawn(&self, command: child::Command, stop_timeout: Duration) -> Option<child::Child> {
        if self.is_closing.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let child = child::Child::spawn(command, child::ShutdownStyle::Graceful(stop_timeout));
        {
            let mut lock = self.children.lock().unwrap();
            lock.push(child.clone());
        }
        Some(child)
    }

    /// Stop the process manager, closing all child processes. On posix
    /// systems this will send a SIGINT, and on windows it will just kill
    /// the process immediately.
    pub async fn stop(&self) {
        self.is_closing
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let mut set = JoinSet::new();

        {
            let lock = self.children.lock().unwrap();
            for child in lock.iter() {
                let mut child = child.clone();
                set.spawn(async move { child.stop().await });
            }
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }
    }

    /// Stop the process manager, waiting for all child processes to exit.
    ///
    /// If you want to set a timeout, use `tokio::time::timeout` and
    /// `Self::stop` if the timeout elapses.
    pub async fn wait(&self) {
        self.is_closing
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let mut set = JoinSet::new();

        {
            let lock = self.children.lock().unwrap();
            for child in lock.iter() {
                let mut child = child.clone();
                set.spawn(async move { child.wait().await });
            }
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }

        {
            // clean up the vec and re-open
            let mut lock = self.children.lock().unwrap();
            std::mem::replace(vec![], lock);
        }
    }
}

#[cfg(test)]
mod test {
    use std::assert_matches::assert_matches;

    use futures::{stream::FuturesUnordered, StreamExt};
    use test_case::test_case;
    use time::Instant;
    use tokio::{join, process::Command, time::sleep};
    use tracing_test::traced_test;

    use super::*;

    fn get_command() -> Command {
        let mut cmd = Command::new("node");
        cmd.arg("./test/scripts/sleep_5_interruptable.js");
        cmd
    }

    #[tokio::test]
    async fn test_basic() {
        let mut manager = ProcessManager::new();
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.stop().await;
    }

    #[tokio::test]
    async fn test_multiple() {
        let mut manager = ProcessManager::new();

        manager.spawn(get_command(), Duration::from_secs(2));
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_closed() {
        let mut manager = ProcessManager::new();
        manager.spawn(get_command(), Duration::from_secs(2));
        manager.stop().await;

        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_exit_code() {
        let mut manager = ProcessManager::new();
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(2))
            .expect("running");

        sleep(Duration::from_millis(100)).await;

        let code = child.wait().await;
        assert_eq!(code, Some(0));

        manager.stop().await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_after_stop() {
        let mut manager = ProcessManager::new();
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(2))
            .expect("running");

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
        let mut manager = ProcessManager::new();
        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;

        assert_matches!(manager.spawn(get_command(), Duration::from_secs(2)), None);

        sleep(Duration::from_millis(100)).await;

        // idempotent
        manager.stop().await;
    }

    #[test_case("stop", None)]
    #[test_case("wait", Some(0))]
    #[tokio::test]
    async fn test_stop_multiple_tasks_shared(strat: &str, expected: Option<i32>) {
        let manager = ProcessManager::new();
        let tasks = FuturesUnordered::new();

        for _ in 0..10 {
            let manager = manager.clone();
            tasks.push(tokio::spawn(async move {
                let mut command = super::child::Command::new("sleep");
                command.arg("1");

                let mut child = manager.spawn(command, Duration::from_secs(1)).unwrap();
                let exit = child.wait().await;

                return exit;
            }));
        }

        // wait for tasks to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        match strat {
            "stop" => manager.stop().await,
            "wait" => manager.wait().await,
            _ => panic!("unknown strat"),
        }

        // tasks return proper exit code
        assert!(
            tasks.all(|v| async { v.unwrap() == expected }).await,
            "not all tasks returned the correct code: {:?}",
            expected
        );
    }

    #[tokio::test]
    async fn test_wait_multiple_tasks() {
        let manager = ProcessManager::new();

        let mut command = super::child::Command::new("sleep");
        command.arg("1");

        manager.spawn(command, Duration::from_secs(1));

        // let the task start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let start_time = Instant::now();

        // we support 'close escalation'; someone can call
        // stop even if others are waiting
        let _ = join! {
            manager.wait(),
            manager.wait(),
            manager.stop(),
        };

        let finish_time = Instant::now();

        assert!((finish_time - start_time).lt(&Duration::from_secs(2)));
    }
}
