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
    sync::{Arc, Mutex},
    time::Duration,
};

use command_group::AsyncCommandGroup;
pub use tokio::process::Command;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info};

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
    /// On windows this will immediately kill, and on posix systems it
    /// will send a SIGINT. If `Duration` elapses, we then follow up with a
    /// `Kill`.
    Graceful(Duration),

    Kill,
}

/// Child process stopped.
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
                #[cfg(unix)]
                {
                    let fut = async {
                        if let Some(pid) = child.id() {
                            debug!("sending SIGINT to child {}", pid);
                            unsafe {
                                libc::kill(pid as i32, libc::SIGINT);
                            }
                            debug!("waiting for child {}", pid);
                            child.wait().await.ok()
                        } else {
                            None
                        }
                    };

                    info!("starting shutdown");

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
                            info!("graceful shutdown timed out, killing child");
                            child.kill().await?;
                            Ok(ChildState::Killed)
                        }
                    }
                }

                #[cfg(windows)]
                {
                    debug!("timeout not supported on windows, killing");
                    child.kill().await?;
                    Ok(ChildState::Killed)
                }
            }
            ShutdownStyle::Kill => {
                child.kill().await?;
                Ok(ChildState::Killed)
            }
        }
    }
}

/// A child process that can be interacted with asynchronously.
///
/// This is a wrapper around the `tokio::process::Child` struct, which provides
/// a cross platform interface for spawning and managing child processes.
#[derive(Clone)]
pub struct Child {
    pid: Option<u32>,
    gid: Option<u32>,
    state: Arc<RwLock<ChildState>>,
    exit_channel: watch::Receiver<Option<i32>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<tokio::process::ChildStdout>>>,
    stderr: Arc<Mutex<Option<tokio::process::ChildStderr>>>,
}

#[derive(Debug)]
pub struct ChildCommandChannel(mpsc::Sender<ChildCommand>);

impl ChildCommandChannel {
    pub fn new() -> (Self, mpsc::Receiver<ChildCommand>) {
        let (tx, rx) = mpsc::channel(1);
        (ChildCommandChannel(tx), rx)
    }

    pub async fn kill(&self) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Kill).await
    }

    pub async fn stop(&self) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Stop).await
    }
}

pub enum ChildCommand {
    Stop,
    Kill,
}

impl Child {
    /// Start a child process, returning a handle that can be used to interact
    /// with it. The command will be started immediately.
    pub fn spawn(mut command: Command, shutdown_style: ShutdownStyle) -> Self {
        let mut group = command.group().spawn().expect("failed to start child");

        let stdin = group.inner().stdin.take();
        let stdout = group.inner().stdout.take();
        let stderr = group.inner().stderr.take();

        let (command_tx, mut command_rx) = ChildCommandChannel::new();
        let (exit_tx, exit_rx) = watch::channel(None);

        let state = Arc::new(RwLock::new(ChildState::Running(command_tx)));
        let task_state = state.clone();
        let pid = group.inner().id();
        let gid = group.id();

        let mut child = group.into_inner();

        let _task = tokio::spawn(async move {
            info!("waiting for task");
            tokio::select! {
                command = command_rx.recv() => {
                    let state = match command {
                        Some(ChildCommand::Stop) | None => {
                            // we received a command to stop the child process
                            shutdown_style.process(&mut child, Some(exit_tx)).await.unwrap()
                        }
                        Some(ChildCommand::Kill) => {
                            // we received a command to kill the child process
                            debug!("killing child process");
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

            debug!("child process exited");
        });

        Self {
            pid,
            gid,
            state,
            exit_channel: exit_rx,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
            stderr: Arc::new(Mutex::new(stderr)),
        }
    }

    /// Wait for the `Child` to exit, returning the exit code.
    pub async fn wait(&mut self) -> Option<i32> {
        self.exit_channel.changed().await.ok()?;
        *self.exit_channel.borrow()
    }

    /// Perform a graceful shutdown of the `Child` process.
    pub async fn stop(&mut self) {
        {
            let state = self.state.read().await;
            let child = match Self::child_channel(&state) {
                Some(child) => child,
                None => return,
            };

            // if this fails, it's because the channel is dropped (toctou)
            // we can just ignore it
            child.stop().await.ok();
        }

        self.wait().await;
    }

    /// Kill the `Child` process immediately.
    pub async fn kill(&mut self) {
        {
            let rw_lock_read_guard = self.state.read().await;
            let child = match Self::child_channel(&rw_lock_read_guard) {
                Some(child) => child,
                None => return,
            };

            // if this fails, it's because the channel is dropped (toctou)
            // we can just ignore it
            child.kill().await.ok();
        }

        self.wait().await;
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.stdin.lock().unwrap().take()
    }

    pub fn stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.stdout.lock().unwrap().take()
    }

    pub fn stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.stderr.lock().unwrap().take()
    }

    /// Get a channel for interacting with the child process.
    fn child_channel(state: &ChildState) -> Option<&ChildCommandChannel> {
        match state {
            ChildState::Running(child) => Some(child),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, process::Stdio, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
    };
    use tracing_test::traced_test;

    use super::{Child, ChildState};
    use crate::process::child::ShutdownStyle;

    const STARTUP_DELAY: Duration = Duration::from_millis(500);

    #[tokio::test]
    async fn test_pid() {
        let mut cmd = Command::new("node");
        cmd.args(["./test/scripts/hello_world.js"]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        assert_matches!(child.pid(), Some(_));
        child.stop().await;

        let state = child.state.read().await;
        assert_matches!(&*state, ChildState::Killed);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_spawn() {
        let cmd = {
            let mut cmd = Command::new("node");
            cmd.args(["./test/scripts/hello_world.js"]);
            cmd.stdout(Stdio::piped());
            cmd
        };

        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        {
            let state = child.state.read().await;
            assert_matches!(&*state, ChildState::Running(_));
        }

        let code = child.wait().await;
        assert_eq!(code, Some(0));

        {
            let state = child.state.read().await;
            assert_matches!(&*state, ChildState::Finished(Some(0)));
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stdout() {
        let mut cmd = Command::new("node");
        cmd.args(["./test/scripts/hello_world.js"]);
        cmd.stdout(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        tokio::time::sleep(STARTUP_DELAY).await;

        child.wait().await;

        {
            let mut output = Vec::new();
            child
                .stdout()
                .unwrap()
                .read_to_end(&mut output)
                .await
                .expect("Failed to read stdout");

            let output_str = String::from_utf8(output).expect("Failed to parse stdout");

            assert!(output_str.contains("hello world"));
        }

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Finished(Some(0)));
    }

    #[tokio::test]
    async fn test_stdio() {
        let mut cmd = Command::new("node");
        cmd.args(["./test/scripts/stdin_stdout.js"]);
        cmd.stdout(Stdio::piped());
        cmd.stdin(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill);

        let mut stdout = child.stdout().unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // drop stdin to close the pipe
        {
            let mut stdin = child.stdin().unwrap();
            stdin.write_all(b"hello world").await.unwrap();
        }

        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await.unwrap();

        let output_str = String::from_utf8(output).expect("Failed to parse stdout");

        assert_eq!(output_str, "hello world");

        child.wait().await;

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Finished(Some(0)));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown_timeout() {
        let cmd = {
            let mut cmd = Command::new("node");
            cmd.args(["./test/scripts/sleep_5_ignore.js"]);
            cmd
        };

        let mut child = Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500)));

        // give it a moment to register the signal handler
        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await;

        let state = child.state.read().await;

        // this should time out and be killed
        assert_matches!(&*state, ChildState::Killed);
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown() {
        let cmd = {
            let mut cmd = Command::new("node");
            cmd.args(["./test/scripts/sleep_5_interruptable.js"]);
            cmd
        };

        let mut child = Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500)));

        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await;
        child.wait().await;

        let state = child.state.read().await;

        // process exits with no code when interrupted
        #[cfg(unix)]
        assert_matches!(&*state, &ChildState::Finished(None));

        #[cfg(not(unix))]
        assert_matches!(&*state, &ChildState::Killed);
    }
}
