//! `child`
//!
//! This module contains the code for spawning a child process and managing it.
//! It is responsible for forwarding signals to the child process, and closing
//! the child process when the manager is closed.
//!
//! The child process is spawned using the `shared_child` crate, which provides
//! a cross platform interface for spawning and managing child processes.
//!
//! Children can be closed in a few ways, either through killing, or more
//! gracefully by coupling a signal and a timeout.
//!
//! This loosely follows the actor model, where the child process is an actor
//! that is spawned and managed by the manager. The manager is responsible for
//! running these processes to completion, forwarding signals, and closing
//! them when the manager is closed.

#[path = "child/io.rs"]
mod child_io;
mod handle;
mod shutdown;
mod state;
#[cfg(test)]
mod test;
#[cfg(test)]
mod test_guard;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::watch;
use tracing::{debug, trace};

#[cfg(unix)]
use self::handle::{TargetIdentity, process_group_matches_identity, signal_process_group};
#[cfg(test)]
use self::test_guard::PtyTestGuard;
pub use self::{
    child_io::ChildStdin,
    shutdown::{ChildExit, ShutdownStyle},
};
use self::{
    child_io::{ChildIO, ChildInput, ChildOutput},
    handle::{ChildHandle, SpawnResult},
    state::{ChildCommand, ChildCommandChannel, ChildStateManager},
};
use super::{Command, PtySize};

/// A child process that can be interacted with asynchronously.
///
/// This is a wrapper around the `tokio::process::Child` struct, which provides
/// a cross platform interface for spawning and managing child processes.
#[derive(Clone, Debug)]
pub struct Child {
    pid: Option<u32>,
    #[cfg(unix)]
    target_identity: Option<TargetIdentity>,
    command_channel: ChildCommandChannel,
    exit_channel: watch::Receiver<Option<ChildExit>>,
    stdin: Arc<Mutex<Option<ChildInput>>>,
    output: Arc<Mutex<Option<ChildOutput>>>,
    label: String,
    shutdown_style: ShutdownStyle,
    /// Flag indicating this child is being stopped as part of a shutdown of the
    /// ProcessManager, rather than individually stopped.
    closing: Arc<AtomicBool>,
    #[cfg(test)]
    _pty_test_guard: Option<Arc<PtyTestGuard>>,
}

impl Child {
    /// Start a child process, returning a handle that can be used to interact
    /// with it. The command will be started immediately.
    #[tracing::instrument(skip(command), fields(command = command.label()))]
    pub fn spawn(
        command: Command,
        shutdown_style: ShutdownStyle,
        pty_size: Option<PtySize>,
    ) -> std::io::Result<Self> {
        let label = command.label();
        #[cfg(test)]
        let pty_test_guard = pty_size.map(|_| Arc::new(PtyTestGuard::acquire()));
        let SpawnResult {
            handle: mut child,
            io: ChildIO { stdin, output },
            controller,
        } = if let Some(size) = pty_size {
            ChildHandle::spawn_pty(command, size)
        } else {
            ChildHandle::spawn_normal(command)
        }?;

        let pid = child.pid();
        #[cfg(unix)]
        let target_identity = child.target_identity;

        let (command_tx, mut command_rx) = ChildCommandChannel::new();

        // we use a watch channel to communicate the exit code back to the
        // caller. we are interested in three cases:
        // - the child process exits
        // - the child process is killed (and doesn't have an exit code)
        // - the child process fails somehow (some syscall fails)
        let (exit_tx, exit_rx) = watch::channel(None);

        let _task = tokio::spawn(async move {
            // On Windows it is important that this gets dropped once the child process
            // exits
            let controller = controller;
            debug!("waiting for task: {pid:?}");
            let mut manager = ChildStateManager {
                shutdown_style,
                exit_tx,
                shutdown_initiated: false,
            };
            tokio::select! {
                biased;
                command = command_rx.recv() => {
                    manager.shutdown_initiated = true;
                    manager.handle_child_command(command, &mut command_rx, &mut child, controller).await;
                }
                status = child.wait() => {
                    drop(controller);
                    manager.handle_child_exit(status).await;
                }
            }

            debug!("child process stopped");
        });

        Ok(Self {
            pid,
            #[cfg(unix)]
            target_identity,
            command_channel: command_tx,
            exit_channel: exit_rx,
            stdin: Arc::new(Mutex::new(stdin)),
            output: Arc::new(Mutex::new(output)),
            label,
            shutdown_style,
            closing: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            _pty_test_guard: pty_test_guard,
        })
    }

    /// Wait for the `Child` to exit, returning the exit code.
    pub async fn wait(&mut self) -> Option<ChildExit> {
        trace!("watching exit channel of {}", self.label);
        // If sending end of exit channel closed, then return last value in the channel
        match self.exit_channel.changed().await {
            Ok(()) => trace!("exit channel was updated"),
            Err(_) => trace!("exit channel sender was dropped"),
        }
        *self.exit_channel.borrow()
    }

    /// Perform a graceful shutdown of the `Child` process.
    pub async fn stop(&mut self) -> Option<ChildExit> {
        self.shutdown(self.shutdown_style).await
    }

    pub async fn shutdown(&mut self, shutdown_style: ShutdownStyle) -> Option<ChildExit> {
        // if this fails, it's because the channel is dropped (toctou)
        // we can just ignore it
        self.command_channel.shutdown(shutdown_style).await.ok();
        self.wait().await
    }

    /// Kill the `Child` process immediately.
    pub async fn kill(&mut self) -> Option<ChildExit> {
        // if this fails, it's because the channel is dropped (toctou)
        // we can just ignore it
        self.command_channel.kill().await.ok();
        self.wait().await
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    #[cfg(unix)]
    fn cleanup_process_scope_after_success(&self) {
        let Some(identity) = self.target_identity else {
            return;
        };

        let Some(pid) = self.pid else {
            return;
        };

        if process_group_matches_identity(pid as libc::pid_t, identity) {
            debug!(
                "cleaning up remaining process group after successful task: {}",
                identity.process_group_id
            );
            signal_process_group(identity.process_group_id, libc::SIGKILL);
        }
    }

    #[cfg(windows)]
    fn cleanup_process_scope_after_success(&self) {
        let Some(pid) = self.pid else {
            return;
        };

        if let Err(err) = super::job_object::terminate_descendant_processes(pid) {
            debug!("failed to clean up descendants after successful task {pid}: {err}");
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn cleanup_process_scope_after_success(&self) {}

    fn cleanup_if_successful(&self, status: Option<ChildExit>) {
        if status == Some(ChildExit::Finished(Some(0))) {
            self.cleanup_process_scope_after_success();
        }
    }

    pub(crate) fn has_exited(&self) -> bool {
        self.exit_channel.borrow().is_some()
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Mark this child as being stopped as part of a ProcessManager shutdown
    pub fn set_closing(&self) {
        self.closing.store(true, Ordering::Release);
    }

    /// Check if this child was stopped as part of a ProcessManager shutdown
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }
}

#[cfg(test)]
impl Child {
    // Helper method for checking if child is running
    fn is_running(&self) -> bool {
        !self.command_channel.0.is_closed()
    }
}
