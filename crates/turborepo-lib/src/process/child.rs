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
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use command_group::AsyncCommandGroup;
use itertools::Itertools;
pub use tokio::process::Command;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    join,
    sync::{mpsc, watch, RwLock},
};
use tracing::{debug, info};

#[derive(Debug)]
pub enum ChildState {
    Running(ChildCommandChannel),
    Exited(ChildExit),
}

impl ChildState {
    pub fn command_channel(&self) -> Option<&ChildCommandChannel> {
        match self {
            ChildState::Running(c) => Some(c),
            ChildState::Exited(_) => None,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ChildExit {
    Finished(Option<i32>),
    Killed,
    /// The child process was killed by someone else. Note that on
    /// windows, it is not possible to distinguish between whether
    /// the process exited normally or was killed
    KilledExternal,
    Failed,
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
pub struct ShutdownFailed;

impl From<std::io::Error> for ShutdownFailed {
    fn from(_: std::io::Error) -> Self {
        ShutdownFailed
    }
}

impl ShutdownStyle {
    /// Process the shutdown style for the given child process.
    ///
    /// If an exit channel is provided, the exit code will be sent to the
    /// channel when the child process exits.
    async fn process(&self, child: &mut tokio::process::Child) -> ChildState {
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
                            child.wait().await.map(|es| es.code())
                        } else {
                            // if there is no pid, then just report successful with no exit code
                            Ok(None)
                        }
                    };

                    info!("starting shutdown");

                    let result = tokio::time::timeout(*timeout, fut).await;
                    match result {
                        Ok(Ok(result)) => ChildState::Exited(ChildExit::Finished(result)),
                        Ok(Err(_)) => ChildState::Exited(ChildExit::Failed),
                        Err(_) => {
                            info!("graceful shutdown timed out, killing child");
                            match child.kill().await {
                                Ok(_) => ChildState::Exited(ChildExit::Killed),
                                Err(_) => ChildState::Exited(ChildExit::Failed),
                            }
                        }
                    }
                }

                #[cfg(windows)]
                {
                    debug!("timeout not supported on windows, killing");
                    match child.kill().await {
                        Ok(_) => ChildState::Exited(ChildExit::Killed),
                        Err(_) => ChildState::Exited(ChildExit::Failed),
                    }
                }
            }
            ShutdownStyle::Kill => match child.kill().await {
                Ok(_) => ChildState::Exited(ChildExit::Killed),
                Err(_) => ChildState::Exited(ChildExit::Failed),
            },
        }
    }
}

/// A child process that can be interacted with asynchronously.
///
/// This is a wrapper around the `tokio::process::Child` struct, which provides
/// a cross platform interface for spawning and managing child processes.
#[derive(Clone, Debug)]
pub struct Child {
    pid: Option<u32>,
    gid: Option<u32>,
    state: Arc<RwLock<ChildState>>,
    exit_channel: watch::Receiver<Option<ChildExit>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<Mutex<Option<tokio::process::ChildStdout>>>,
    stderr: Arc<Mutex<Option<tokio::process::ChildStderr>>>,
    label: String,
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
    pub fn spawn(mut command: Command, shutdown_style: ShutdownStyle) -> io::Result<Self> {
        let label = {
            let cmd = command.as_std();
            format!(
                "({}) {} {}",
                cmd.get_current_dir()
                    .map(|dir| dir.to_string_lossy())
                    .unwrap_or_default(),
                cmd.get_program().to_string_lossy(),
                cmd.get_args().map(|s| s.to_string_lossy()).join(" ")
            )
        };

        let group = command.group().spawn()?;

        let gid = group.id();
        let mut child = group.into_inner();
        let pid = child.id();

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let (command_tx, mut command_rx) = ChildCommandChannel::new();

        // we use a watch channel to communicate the exit code back to the
        // caller. we are interested in three cases:
        // - the child process exits
        // - the child process is killed (and doesn't have an exit code)
        // - the child process fails somehow (some syscall fails)
        let (exit_tx, exit_rx) = watch::channel(None);

        let state = Arc::new(RwLock::new(ChildState::Running(command_tx)));
        let task_state = state.clone();

        let _task = tokio::spawn(async move {
            info!("waiting for task");
            tokio::select! {
                command = command_rx.recv() => {
                    let state = match command {
                        // we received a command to stop the child process, or the channel was closed.
                        // in theory this happens when the last child is dropped, however in practice
                        // we will always get a `Permit` from the recv call before the channel can be
                        // dropped, and the cnannel is not closed while there are still permits
                        Some(ChildCommand::Stop) | None => {
                            debug!("stopping child process");
                            shutdown_style.process(&mut child).await
                        }
                        // we received a command to kill the child process
                        Some(ChildCommand::Kill) => {
                            debug!("killing child process");
                            ShutdownStyle::Kill.process(&mut child).await
                        }
                    };

                    match state {
                        ChildState::Exited(exit) => {
                            // ignore the send error, failure means the channel is dropped
                            exit_tx.send(Some(exit)).ok();
                        }
                        ChildState::Running(_) => {
                            debug_assert!(false, "child state should not be running after shutdown");
                        }
                    }

                    {
                        let mut task_state = task_state.write().await;
                        *task_state = state;
                    }
                }
                status = child.wait() => {
                    debug!("child process exited normally");
                    // the child process exited
                    let child_exit = match status.map(|s| s.code()) {
                        Ok(Some(c)) => ChildExit::Finished(Some(c)),
                        // if we hit this case, it means that the child process was killed
                        // by someone else, and we should report that it was killed
                        Ok(None) => ChildExit::KilledExternal,
                        Err(_e) => ChildExit::Failed,
                    };
                    {
                        let mut task_state = task_state.write().await;
                        *task_state = ChildState::Exited(child_exit);
                    }

                    // ignore the send error, the channel is dropped anyways
                    exit_tx.send(Some(child_exit)).ok();

                }
            }

            debug!("child process stopped");
        });

        Ok(Self {
            pid,
            gid,
            state,
            exit_channel: exit_rx,
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
            stderr: Arc::new(Mutex::new(stderr)),
            label,
        })
    }

    /// Wait for the `Child` to exit, returning the exit code.
    pub async fn wait(&mut self) -> Option<ChildExit> {
        self.exit_channel.changed().await.ok()?;
        *self.exit_channel.borrow()
    }

    /// Perform a graceful shutdown of the `Child` process.
    pub async fn stop(&mut self) -> Option<ChildExit> {
        let mut watch = self.exit_channel.clone();

        let fut = async {
            let state = self.state.read().await;
            let child = match state.command_channel() {
                Some(child) => child,
                None => return,
            };

            // if this fails, it's because the channel is dropped (toctou)
            // we can just ignore it
            child.stop().await.ok();
        };

        let (_, code) = join! {
            fut,
            async {
                watch.changed().await.ok()?;
                *watch.borrow()
            }
        };

        code
    }

    /// Kill the `Child` process immediately.
    pub async fn kill(&mut self) -> Option<ChildExit> {
        let mut watch = self.exit_channel.clone();

        let fut = async {
            let rw_lock_read_guard = self.state.read().await;
            let child = match rw_lock_read_guard.command_channel() {
                Some(child) => child,
                None => return,
            };

            // if this fails, it's because the channel is dropped (toctou)
            // we can just ignore it
            child.kill().await.ok();
        };

        let (_, code) = join! {
            fut,
            async {
                // if this fails, it is because the watch receiver is dropped. just ignore it do a best-effort
                watch.changed().await.ok();
                *watch.borrow()
            }
        };

        code
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

    /// Wait for the `Child` to exit and pipe any stdout and stderr to the
    /// provided writers.
    /// If `None` is passed for stderr then all output produced will be piped
    /// to stdout
    pub async fn wait_with_piped_outputs<W: Write>(
        &mut self,
        mut stdout_pipe: W,
        mut stderr_pipe: Option<W>,
    ) -> Result<Option<ChildExit>, std::io::Error> {
        async fn next_line<R: AsyncBufRead + Unpin>(
            stream: &mut Option<R>,
            buffer: &mut Vec<u8>,
        ) -> Option<Result<(), io::Error>> {
            match stream {
                Some(stream) => match stream.read_until(b'\n', buffer).await {
                    Ok(0) => None,
                    Ok(_) => Some(Ok(())),
                    Err(e) => Some(Err(e)),
                },
                None => None,
            }
        }

        let mut stdout_lines = self.stdout().map(BufReader::new);
        let mut stderr_lines = self.stderr().map(BufReader::new);

        let mut stdout_buffer = Vec::new();
        let mut stderr_buffer = Vec::new();

        loop {
            tokio::select! {
                Some(result) = next_line(&mut stdout_lines, &mut stdout_buffer) => {
                    result?;
                    stdout_pipe.write_all(&stdout_buffer)?;
                    stdout_buffer.clear();
                }
                Some(result) = next_line(&mut stderr_lines, &mut stderr_buffer) => {
                    result?;
                    stderr_pipe.as_mut().unwrap_or(&mut stdout_pipe).write_all(&stderr_buffer)?;
                    stderr_buffer.clear();
                }
                else => {
                    // In the case that both futures read a complete line
                    // the future not chosen in the select will return None if it's at EOF
                    // as the number of bytes read will be 0.
                    // We check and flush the buffers to avoid missing the last line of output.
                    if !stdout_buffer.is_empty() {
                        stdout_pipe.write_all(&stdout_buffer)?;
                        stdout_buffer.clear();
                    }
                    if !stderr_buffer.is_empty() {
                        stderr_pipe.as_mut().unwrap_or(&mut stdout_pipe).write_all(&stderr_buffer)?;
                        stderr_buffer.clear();
                    }
                    break;
                }
            }
        }
        debug_assert!(stdout_buffer.is_empty(), "buffer should be empty");
        debug_assert!(stderr_buffer.is_empty(), "buffer should be empty");

        Ok(self.wait().await)
    }

    pub fn label(&self) -> &str {
        &self.label
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
    use turbopath::AbsoluteSystemPathBuf;

    use super::{Child, ChildState};
    use crate::process::child::{ChildExit, ShutdownStyle};

    const STARTUP_DELAY: Duration = Duration::from_millis(500);

    fn find_script_dir() -> AbsoluteSystemPathBuf {
        let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
        let mut root = cwd;
        while !root.join_component(".git").exists() {
            root = root.parent().unwrap().to_owned();
        }
        root.join_components(&["crates", "turborepo-lib", "test", "scripts"])
    }

    #[tokio::test]
    async fn test_pid() {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        assert_matches!(child.pid(), Some(_));
        child.stop().await;

        let state = child.state.read().await;
        assert_matches!(&*state, ChildState::Exited(ChildExit::Killed));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_spawn() {
        let cmd = {
            let script = find_script_dir().join_component("hello_world.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd.stdout(Stdio::piped());
            cmd
        };

        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        {
            let state = child.state.read().await;
            assert_matches!(&*state, ChildState::Running(_));
        }

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));

        {
            let state = child.state.read().await;
            assert_matches!(&*state, ChildState::Exited(ChildExit::Finished(Some(0))));
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_stdout() {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

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

        assert_matches!(&*state, ChildState::Exited(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_stdio() {
        let script = find_script_dir().join_component("stdin_stdout.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        cmd.stdin(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

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

        assert_matches!(&*state, ChildState::Exited(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown_timeout() {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_ignore.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd.stdout(Stdio::piped());
            cmd
        };

        let mut child =
            Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500))).unwrap();

        let mut stdout = child.stdout().unwrap();
        let mut buf = vec![0; 4];
        // wait for the process to print "here"
        stdout.read_exact(&mut buf).await.unwrap();
        child.stop().await;

        let state = child.state.read().await;

        // this should time out and be killed
        assert_matches!(&*state, ChildState::Exited(ChildExit::Killed));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown() {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_interruptable.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child =
            Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500))).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await;
        child.wait().await;

        let state = child.state.read().await;

        // process exits with no code when interrupted
        #[cfg(unix)]
        assert_matches!(&*state, &ChildState::Exited(ChildExit::Finished(None)));

        #[cfg(not(unix))]
        assert_matches!(&*state, &ChildState::Exited(ChildExit::Killed));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_detect_killed_someone_else() {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_interruptable.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child =
            Child::spawn(cmd, ShutdownStyle::Graceful(Duration::from_millis(500))).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        #[cfg(unix)]
        if let Some(pid) = child.pid() {
            unsafe {
                libc::kill(pid as i32, libc::SIGINT);
            }
        }
        #[cfg(windows)]
        if let Some(pid) = child.pid() {
            unsafe {
                println!("killing");
                winapi::um::processthreadsapi::TerminateProcess(
                    winapi::um::processthreadsapi::OpenProcess(
                        winapi::um::winnt::PROCESS_TERMINATE,
                        0,
                        pid,
                    ),
                    3,
                );
            }
        }

        child.wait().await;

        let state = child.state.read().await;

        let _expected = if cfg!(unix) {
            ChildExit::KilledExternal
        } else {
            ChildExit::Finished(Some(1))
        };

        assert_matches!(&*state, ChildState::Exited(_expected));
    }

    #[tokio::test]
    async fn test_wait_with_output() {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();

        let exit = child
            .wait_with_piped_outputs(&mut out, Some(&mut err))
            .await
            .unwrap();

        assert_eq!(out, b"hello world\n");
        assert!(err.is_empty());
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_wait_with_single_output() {
        let script = find_script_dir().join_component("hello_world_hello_moon.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        let mut buffer = Vec::new();

        let exit = child
            .wait_with_piped_outputs(&mut buffer, None)
            .await
            .unwrap();

        // There are no ordering guarantees so we accept either order of the logs
        assert!(buffer == b"hello world\nhello moon\n" || buffer == b"hello moon\nhello world\n");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_wait_with_with_non_utf8_output() {
        let script = find_script_dir().join_component("hello_non_utf8.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        let mut out = Vec::new();
        let mut err = Vec::new();

        let exit = child
            .wait_with_piped_outputs(&mut out, Some(&mut err))
            .await
            .unwrap();

        assert_eq!(out, &[0, 159, 146, 150, b'\n']);
        assert!(err.is_empty());
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[tokio::test]
    async fn test_wait_with_non_utf8_single_output() {
        let script = find_script_dir().join_component("hello_non_utf8.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill).unwrap();

        let mut buffer = Vec::new();

        let exit = child
            .wait_with_piped_outputs(&mut buffer, None)
            .await
            .unwrap();

        assert_eq!(buffer, &[0, 159, 146, 150, b'\n']);
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }
}
