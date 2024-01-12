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
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::Future;
use tokio::task::JoinSet;
use tracing::{debug, trace};

pub use self::child::{Child, ChildExit};

/// A process manager that is responsible for spawning and managing child
/// processes. When the manager is Open, new child processes can be spawned
/// using `spawn`. When the manager is Closed, all currently-running children
/// will be closed, and no new children can be spawned.
#[derive(Debug, Clone)]
pub struct ProcessManager(Arc<Mutex<ProcessManagerInner>>);

#[derive(Debug)]
struct ProcessManagerInner {
    is_closing: bool,
    children: Vec<child::Child>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ProcessManagerInner {
            is_closing: false,
            children: Vec::new(),
        })))
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
        pty: portable_pty::PtyPair,
        cmd_builder: portable_pty::CommandBuilder,
        // command: child::Command,
        stop_timeout: Duration,
    ) -> Option<io::Result<child::Child>> {
        let mut lock = self.0.lock().unwrap();
        if lock.is_closing {
            return None;
        }

        let child = child::Child::spawn(
            pty,
            cmd_builder,
            // command,
            child::ShutdownStyle::Graceful(stop_timeout),
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
            let mut lock = self.0.lock().expect("not poisoned");
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
            let mut lock = self.0.lock().expect("not poisoned");

            // just allocate a new vec rather than clearing the old one
            lock.children = vec![];
        }
    }
}
