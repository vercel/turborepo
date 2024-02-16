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
mod command;

use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

pub use command::Command;
use futures::Future;
use tokio::task::JoinSet;
use tracing::{debug, trace};

pub use self::child::{Child, ChildExit};

/// A process manager that is responsible for spawning and managing child
/// processes. When the manager is Open, new child processes can be spawned
/// using `spawn`. When the manager is Closed, all currently-running children
/// will be closed, and no new children can be spawned.
#[derive(Debug, Clone)]
pub struct ProcessManager {
    state: Arc<Mutex<ProcessManagerInner>>,
    use_pty: bool,
}

#[derive(Debug)]
struct ProcessManagerInner {
    is_closing: bool,
    children: Vec<child::Child>,
}

impl ProcessManager {
    pub fn new(use_pty: bool) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessManagerInner {
                is_closing: false,
                children: Vec::new(),
            })),
            use_pty,
        }
    }

    /// Construct a process manager and infer if pty should be used
    pub fn infer() -> Self {
        // Only use PTY if we're not on windows and we're currently hooked up to a
        // in a TTY
        let use_pty = !cfg!(windows) && atty::is(atty::Stream::Stdout);
        Self::new(use_pty)
    }
}

impl ProcessManager {
    /// Spawn a new child process to run the given command.
    ///
    /// The handle of the child can be either waited or stopped by the caller,
    /// as well as the entire process manager.
    ///
    /// If spawn returns None, the process manager is closed and the child
    /// process was not spawned. If spawn returns Some(Err), the process
    /// manager is open, but the child process failed to spawn.
    pub fn spawn(
        &self,
        command: Command,
        stop_timeout: Duration,
    ) -> Option<io::Result<child::Child>> {
        let mut lock = self.state.lock().unwrap();
        if lock.is_closing {
            return None;
        }
        let child = child::Child::spawn(
            command,
            child::ShutdownStyle::Graceful(stop_timeout),
            self.use_pty,
        );
        if let Ok(child) = &child {
            lock.children.push(child.clone());
        }
        Some(child)
    }

    /// Stop the process manager, closing all child processes. On posix
    /// systems this will send a SIGINT, and on windows it will just kill
    /// the process immediately.
    pub async fn stop(&self) {
        self.close(|mut c| async move { c.stop().await }).await
    }

    /// Stop the process manager, waiting for all child processes to exit.
    ///
    /// If you want to set a timeout, use `tokio::time::timeout` and
    /// `Self::stop` if the timeout elapses.
    pub async fn wait(&self) {
        self.close(|mut c| async move { c.wait().await }).await
    }

    /// Close the process manager, running the given callback on each child
    ///
    /// note: this is designed to be called multiple times, ie calling close
    /// with two different strategies will propagate both signals to the child
    /// processes. clearing the task queue and re-enabling spawning are both
    /// idempotent operations
    async fn close<F, C>(&self, callback: F)
    where
        F: Fn(Child) -> C + Sync + Send + Copy + 'static,
        C: Future<Output = Option<ChildExit>> + Sync + Send + 'static,
    {
        let mut set = JoinSet::new();

        {
            let mut lock = self.state.lock().expect("not poisoned");
            lock.is_closing = true;
            for child in lock.children.iter() {
                let child = child.clone();
                set.spawn(async move { callback(child).await });
            }
        }

        debug!("waiting for {} processes to exit", set.len());

        while let Some(out) = set.join_next().await {
            trace!("process exited: {:?}", out);
        }

        {
            let mut lock = self.state.lock().expect("not poisoned");

            // just allocate a new vec rather than clearing the old one
            lock.children = vec![];
        }
    }
}

#[cfg(test)]
mod test {
    use futures::{stream::FuturesUnordered, StreamExt};
    use test_case::test_case;
    use time::Instant;
    use tokio::{join, time::sleep};
    use tracing_test::traced_test;

    use super::*;

    fn get_command() -> Command {
        get_script_command("sleep_5_interruptable.js")
    }

    fn get_script_command(script_name: &str) -> Command {
        let mut cmd = Command::new("node");
        cmd.args([format!("./test/scripts/{script_name}")]);
        cmd
    }

    const STOPPED_EXIT: Option<ChildExit> = Some(ChildExit::Killed);

    #[tokio::test]
    async fn test_basic() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(get_script_command("hello_world.js"), Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let mut out = Vec::new();
        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();
        assert_eq!(exit, Some(ChildExit::Finished(Some(0))));
        assert_eq!(out, b"hello world\n");
    }

    #[tokio::test]
    async fn test_multiple() {
        let manager = ProcessManager::new(false);

        let children = (0..2)
            .map(|_| {
                manager
                    .spawn(get_command(), Duration::from_secs(2))
                    .unwrap()
                    .unwrap()
            })
            .collect::<Vec<_>>();

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;

        for mut child in children {
            let exit = child.wait().await;
            assert_eq!(exit, STOPPED_EXIT,);
        }
    }

    #[tokio::test]
    async fn test_closed() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let mut out = Vec::new();
        let (exit, _) = join! {
            child.wait_with_piped_outputs(&mut out),
            manager.stop(),
        };
        let exit = exit.unwrap();
        assert_eq!(exit, STOPPED_EXIT,);
        assert_eq!(
            out, b"",
            "child process should exit before output is printed"
        );

        // Verify that we can't start new child processes
        assert!(manager
            .spawn(get_command(), Duration::from_secs(2))
            .is_none());

        manager.stop().await;
    }

    #[tokio::test]
    async fn test_exit_code() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(get_script_command("hello_world.js"), Duration::from_secs(2))
            .unwrap()
            .unwrap();

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));

        // TODO: maybe we should do some assertion that there was nothing to shut down
        // and this is a noop?
        manager.stop().await;
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_after_stop() {
        let manager = ProcessManager::new(false);
        let mut child = manager
            .spawn(get_script_command("hello_world.js"), Duration::from_secs(2))
            .unwrap()
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        let exit = child.wait().await;
        assert_eq!(exit, Some(ChildExit::Finished(Some(0))));

        manager.stop().await;

        // this is idempotent, so calling it after the manager is stopped is ok
        let kill_code = child.kill().await;
        assert_eq!(kill_code, Some(ChildExit::Finished(Some(0))));

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_reuse_manager() {
        let manager = ProcessManager::new(false);
        manager.spawn(get_command(), Duration::from_secs(2));

        sleep(Duration::from_millis(100)).await;

        manager.stop().await;

        assert!(manager.state.lock().unwrap().children.is_empty());

        // TODO: actually do some check that this is idempotent
        // idempotent
        manager.stop().await;
    }

    #[test_case("stop", "sleep_5_interruptable.js", STOPPED_EXIT)]
    #[test_case("wait", "hello_world.js", Some(ChildExit::Finished(Some(0))))]
    #[tokio::test]
    async fn test_stop_multiple_tasks_shared(
        strategy: &str,
        script: &str,
        expected: Option<ChildExit>,
    ) {
        let manager = ProcessManager::new(false);
        let tasks = FuturesUnordered::new();

        for _ in 0..10 {
            let manager = manager.clone();
            let command = get_script_command(script);
            tasks.push(tokio::spawn(async move {
                manager
                    .spawn(command, Duration::from_secs(1))
                    .unwrap()
                    .unwrap()
                    .wait()
                    .await
            }));
        }

        // wait for tasks to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        match strategy {
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
        let manager = ProcessManager::new(false);

        let mut out = Vec::new();
        let mut child = manager
            .spawn(get_command(), Duration::from_secs(1))
            .unwrap()
            .unwrap();

        // let the task start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let start_time = Instant::now();

        // we support 'close escalation'; someone can call
        // stop even if others are waiting
        let (exit, _, _) = join! {
            child.wait_with_piped_outputs(&mut out),
            manager.wait(),
            manager.stop(),
        };

        assert_eq!(exit.unwrap(), STOPPED_EXIT);
        assert_eq!(
            out, b"",
            "child process was stopped before any output was written"
        );

        let finish_time = Instant::now();

        assert!((finish_time - start_time).lt(&Duration::from_secs(2)));
    }
}
