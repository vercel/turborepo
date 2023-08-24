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

use std::{
    future::ready,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::TryFutureExt;
pub use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, watch, RwLock};
use tracing::debug;

/// Represents all the information needed to run a child process.
///
/// We use this over the `Command` struct from `std::process` the builtin
/// struct for better control.
// #[derive(Builder)]
// struct Command {
//     program: CString,
//     #[builder(default, setter(into))]
//     args: Vec<CString>,
// }

// impl CommandBuilder {
//     pub fn new(program: impl Into<CString>) -> Self {
//         // let c = tokio::process::Command::new(program);
//         // c.args(args)

//         *CommandBuilder::default().program(program.into())
//     }
// }

#[derive(Debug)]
pub enum ChildState {
    Running(ChildCommandChannel),
    Killed,
    /// The child process has exited, and the exit code is provided.
    /// On unix, termination via a signal will not yield an exit code.
    Finished(Option<i32>),
}

#[derive(Clone)]
pub enum ShutdownStyle {
    /// On windows this will send a CTRL_BREAK_EVENT, and on posix systems it
    /// will send a SIGINT. If `Duration` elapses, we then follow up with a
    /// `Kill`.
    Graceful(Duration),

    Kill,
}

#[derive(Debug)]
pub struct KillFailed;

impl From<std::io::Error> for KillFailed {
    fn from(_: std::io::Error) -> Self {
        KillFailed
    }
}

impl ShutdownStyle {
    /// Process the shutdown style for the given child process.
    ///
    /// If an exit channel is provided, the exit code will be sent to the
    /// channel when the child process exits.
    async fn process(
        &self,
        child: &mut tokio::process::Child,
        exit_channel: Option<watch::Sender<Option<i32>>>,
    ) -> Result<ChildState, KillFailed> {
        match self {
            ShutdownStyle::Graceful(timeout) => {
                // try ro run the command for the given timeout
                let fut = async {
                    #[cfg(unix)]
                    {
                        if let Some(pid) = child.id() {
                            debug!("sending SIGINT to child {}", pid);
                            unsafe {
                                libc::kill(pid as i32, libc::SIGINT);
                            }
                            child.wait().await.ok()
                        } else {
                            None
                        }
                    }

                    #[cfg(windows)]
                    {
                        // send the CTRL_BREAK_EVENT signal to the child process
                        if let Some(pid) = child.id() {
                            debug!("sending CTRL_BREAK_EVENT to child {}", pid);
                            unsafe {
                                winapi::um::wincon::GenerateConsoleCtrlEvent(
                                    winapi::um::wincon::CTRL_BREAK_EVENT,
                                    pid,
                                );
                            }
                            child.wait().await.ok()
                        } else {
                            None
                        }
                    }
                };

                let result = tokio::time::timeout(*timeout, fut).await;
                match result {
                    Ok(x) => {
                        let exit_code = x.and_then(|es| es.code());
                        if let Some(channel) = exit_channel {
                            channel.send(exit_code).ok();
                        }
                        Ok(ChildState::Finished(exit_code))
                    }
                    Err(_) => {
                        debug!("graceful shutdown timed out, killing child");
                        child.kill().await?;
                        Ok(ChildState::Killed)
                    }
                }
            }
            ShutdownStyle::Kill => {
                child.kill().await?;
                Ok(ChildState::Killed)
            }
        }
    }
}

#[derive(Clone)]
pub struct Child {
    pid: Option<u32>,
    state: Arc<RwLock<ChildState>>,
    exit_channel: watch::Receiver<Option<i32>>,
}

#[derive(Debug)]
pub struct ChildCommandChannel(mpsc::Sender<ChildCommand>);

impl ChildCommandChannel {
    pub fn new() -> (Self, mpsc::Receiver<ChildCommand>) {
        let (tx, rx) = mpsc::channel(1);
        (ChildCommandChannel(tx), rx)
    }

    pub async fn kill(&self) {
        self.0.send(ChildCommand::Kill).await;
    }

    pub async fn stop(&self) {
        self.0.send(ChildCommand::Stop).await;
    }
}

pub enum ChildCommand {
    Stop,
    Kill,
}

impl Child {
    /// Start a child process, returning a oneshot channel that will receive
    /// the exit code of the process when it exits.
    ///
    /// This spawns a task that will wait for the child process to exit, and
    /// send the exit code to the channel.
    pub fn spawn(mut command: Command, shutdown_style: ShutdownStyle) -> Self {
        let mut child = command.spawn().expect("failed to start child");

        let (command_tx, mut command_rx) = ChildCommandChannel::new();
        let (exit_tx, exit_rx) = watch::channel(None);

        let state = Arc::new(RwLock::new(ChildState::Running(command_tx)));
        let task_state = state.clone();
        let pid = child.id();

        let task = tokio::spawn(async move {
            tokio::select! {
                command = command_rx.recv() => {
                    let state = match command {
                        Some(ChildCommand::Stop) | None => {
                            // we received a command to stop the child process
                            shutdown_style.process(&mut child, Some(exit_tx)).await.unwrap()
                        }
                        Some(ChildCommand::Kill) => {
                            // we received a command to kill the child process
                            ShutdownStyle::Kill.process(&mut child, Some(exit_tx)).await.unwrap()

                        }
                    };

                    {
                        let mut task_state = task_state.write().await;
                        *task_state = state;
                    }
                }
                status = child.wait() => {
                    // the child process exited
                    let exit_code = status.ok().and_then(|s| s.code());
                        {
                            let mut task_state = task_state.write().await;
                            *task_state = ChildState::Finished(exit_code);
                        }
                        exit_tx.send(exit_code).ok();

                }
            }
        });

        Self {
            pid,
            state,
            exit_channel: exit_rx,
        }
    }

    pub async fn wait(&mut self) -> Option<i32> {
        self.exit_channel.changed().await.ok()?;
        *self.exit_channel.borrow()
    }

    /// Perform a graceful shutdown of the child process.
    pub async fn stop(&mut self) -> Result<(), KillFailed> {
        {
            let state = self.state.read().await;
            let child = match Self::child(&*state) {
                Some(child) => child,
                None => return Ok(()),
            };
            child.stop().await;
        }

        self.wait().await;
        Ok(())
    }

    /// Kill the child process immediately.
    pub async fn kill(&mut self) -> Result<(), KillFailed> {
        let next_state = {
            let rw_lock_read_guard = self.state.read().await;
            let child = match Self::child(&*rw_lock_read_guard) {
                Some(child) => child,
                None => return Ok(()),
            };

            child.kill().await;
        };

        let mut state = self.state.write().await;
        // *state = next_state;

        Ok(())
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn stdout(&mut self) -> Option<&mut tokio::process::ChildStdout> {
        todo!()
        // Self::child(&mut self.state).and_then(|c| c.stdout.as_mut())
    }

    fn stderr(&mut self) -> Option<&mut tokio::process::ChildStderr> {
        todo!()
        // Self::child(&mut self.state).and_then(|c| c.stderr.as_mut())
    }

    fn child(state: &ChildState) -> Option<&ChildCommandChannel> {
        match state {
            ChildState::Running(child) => Some(child),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, process::Stdio, time::Duration};

    use tokio::{io::AsyncReadExt, process::Command};
    use tracing_test::traced_test;

    use super::{Child, ChildState};
    use crate::process::child::ShutdownStyle;

    const STARTUP_DELAY: Duration = Duration::from_millis(500);

    #[tokio::test]
    async fn test_pid() {
        let mut cmd = Command::new("echo");
        cmd.args(&["hello", "world"]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        assert_matches!(child.pid(), Some(_));
        child.stop().await.unwrap();

        let state = child.state.read().await;
        assert_matches!(&*state, ChildState::Running(_));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_await_start() {
        let mut cmd = Command::new("echo");
        cmd.args(&["hello", "world"]);
        cmd.stdout(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        {
            let state = child.state.read().await;
            assert_matches!(&*state, ChildState::Running(_));
        }

        child.wait().await;

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Finished(Some(0)));
    }

    #[tokio::test]
    async fn test_start() {
        let mut cmd = Command::new("env");
        cmd.envs([("a", "b"), ("c", "d")].iter().copied());
        cmd.stdout(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        tokio::time::sleep(STARTUP_DELAY).await;

        {
            let mut output = Vec::new();
            child
                .stdout()
                .unwrap()
                .read_to_end(&mut output)
                .await
                .expect("Failed to read stdout");

            let output_str = String::from_utf8(output).expect("Failed to parse stdout");

            for &env_var in &["a=b", "c=d"] {
                assert!(output_str.contains(env_var));
            }
        }

        child.stop().await;

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Killed);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown_timeout() {
        let mut cmd = Command::new("sh");
        cmd.args(&["-c", "trap '' SIGINT INT; while true; do sleep 0.1; done"]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500)));

        // give it a moment to register the signal handler
        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await.unwrap();

        let state = child.state.read().await;

        // this should time out and be killed
        assert_matches!(&*state, ChildState::Killed);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown() {
        let mut cmd = Command::new("sh");
        cmd.args(&["-c", "while true; do sleep 0.2; done"]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500)));

        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await.unwrap();

        let state = child.state.read().await;

        // process exits with 1 when interrupted
        assert_matches!(&*state, &ChildState::Finished(None));
    }
}
