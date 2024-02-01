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

const CHILD_POLL_INTERVAL: Duration = Duration::from_micros(50);

use std::{
    fmt,
    io::{self, BufRead, Read, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use portable_pty::{native_pty_system, Child as PtyChild, MasterPty as PtyController};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    join,
    process::Command as TokioCommand,
    sync::{mpsc, watch, RwLock},
};
use tracing::debug;

use super::Command;

#[derive(Debug)]
pub enum ChildState {
    Running(ChildCommandChannel),
    Exited(ChildExit),
}

impl ChildState {
    pub fn command_channel(&self) -> Option<ChildCommandChannel> {
        match self {
            ChildState::Running(c) => Some(c.clone()),
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

#[derive(Debug, Clone)]
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

struct ChildHandle {
    pid: Option<u32>,
    imp: ChildHandleImpl,
}

enum ChildHandleImpl {
    Tokio(tokio::process::Child),
    Pty(Box<dyn PtyChild + Send + Sync>),
}

impl ChildHandle {
    #[tracing::instrument(skip(command))]
    pub fn spawn_normal(command: Command) -> io::Result<SpawnResult> {
        let mut command = TokioCommand::from(command);

        // Create a process group for the child on unix like systems
        #[cfg(unix)]
        {
            use nix::unistd::setsid;
            unsafe {
                command.pre_exec(|| {
                    setsid()?;
                    Ok(())
                });
            }
        }

        let mut child = command.spawn()?;
        let pid = child.id();

        let stdin = child.stdin.take().map(ChildInput::Std);
        let stdout = child
            .stdout
            .take()
            .expect("child process must be started with piped stdout");
        let stderr = child
            .stderr
            .take()
            .expect("child process must be started with piped stderr");

        Ok(SpawnResult {
            handle: Self {
                pid,
                imp: ChildHandleImpl::Tokio(child),
            },
            io: ChildIO {
                stdin,
                output: Some(ChildOutput::Std { stdout, stderr }),
            },
            controller: None,
        })
    }

    #[tracing::instrument(skip(command))]
    pub fn spawn_pty(command: Command) -> io::Result<SpawnResult> {
        use portable_pty::PtySize;

        let keep_stdin_open = command.will_open_stdin();

        let command = portable_pty::CommandBuilder::from(command);
        let pty_system = native_pty_system();
        let size =
            console::Term::stdout()
                .size_checked()
                .map_or_else(PtySize::default, |(rows, cols)| PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
        let pair = pty_system
            .openpty(size)
            .map_err(|err| match err.downcast() {
                Ok(err) => err,
                Err(err) => io::Error::new(io::ErrorKind::Other, err),
            })?;

        let controller = pair.master;
        let receiver = pair.slave;

        #[cfg(unix)]
        {
            use nix::sys::termios;
            if let Some((file_desc, mut termios)) = controller
                .as_raw_fd()
                .and_then(|fd| Some(fd).zip(termios::tcgetattr(fd).ok()))
            {
                // We unset ECHOCTL to disable rendering of the closing of stdin
                // as ^D
                termios.local_flags &= !nix::sys::termios::LocalFlags::ECHOCTL;
                if let Err(e) = nix::sys::termios::tcsetattr(
                    file_desc,
                    nix::sys::termios::SetArg::TCSANOW,
                    &termios,
                ) {
                    debug!("unable to unset ECHOCTL: {e}");
                }
            }
        }

        let child = receiver
            .spawn_command(command)
            .map_err(|err| match err.downcast() {
                Ok(err) => err,
                Err(err) => io::Error::new(io::ErrorKind::Other, err),
            })?;

        let pid = child.process_id();

        let mut stdin = controller.take_writer().ok();
        let output = controller.try_clone_reader().ok().map(ChildOutput::Pty);

        // If we don't want to keep stdin open we take it here and it is immediately
        // dropped resulting in a EOF being sent to the child process.
        if !keep_stdin_open {
            stdin.take();
        }

        Ok(SpawnResult {
            handle: Self {
                pid,
                imp: ChildHandleImpl::Pty(child),
            },
            io: ChildIO {
                stdin: stdin.map(ChildInput::Pty),
                output,
            },
            controller: Some(controller),
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub async fn wait(&mut self) -> io::Result<Option<i32>> {
        match &mut self.imp {
            ChildHandleImpl::Tokio(child) => child.wait().await.map(|status| status.code()),
            ChildHandleImpl::Pty(child) => {
                // TODO: we currently poll the child to see if it has finished yet which is less
                // than ideal
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // portable_pty maps the status of being killed by a signal to a 1 exit
                            // code. The only way to tell if the task
                            // exited normally with exit code 1 or got killed by a signal is to
                            // display it as the signal will be included
                            // in the message.
                            let exit_code = if status.exit_code() == 1
                                && status.to_string().contains("Terminated by")
                            {
                                None
                            } else {
                                // This is safe as the portable_pty::ExitStatus's exit code is just
                                // converted from a i32 to an u32 before we get it
                                Some(status.exit_code() as i32)
                            };
                            return Ok(exit_code);
                        }
                        Ok(None) => {
                            // child hasn't finished, we sleep for a short time
                            tokio::time::sleep(CHILD_POLL_INTERVAL).await;
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
        }
    }

    pub async fn kill(&mut self) -> io::Result<()> {
        match &mut self.imp {
            ChildHandleImpl::Tokio(child) => child.kill().await,
            ChildHandleImpl::Pty(child) => {
                let mut killer = child.clone_killer();
                tokio::task::spawn_blocking(move || killer.kill())
                    .await
                    .unwrap()
            }
        }
    }
}

struct SpawnResult {
    handle: ChildHandle,
    io: ChildIO,
    controller: Option<Box<dyn PtyController + Send>>,
}

struct ChildIO {
    stdin: Option<ChildInput>,
    output: Option<ChildOutput>,
}

enum ChildInput {
    Std(tokio::process::ChildStdin),
    Pty(Box<dyn Write + Send>),
}
enum ChildOutput {
    Std {
        stdout: tokio::process::ChildStdout,
        stderr: tokio::process::ChildStderr,
    },
    Pty(Box<dyn Read + Send>),
}

impl ChildInput {
    fn concrete(self) -> Option<tokio::process::ChildStdin> {
        match self {
            ChildInput::Std(stdin) => Some(stdin),
            ChildInput::Pty(_) => None,
        }
    }
}

impl fmt::Debug for ChildInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Std(arg0) => f.debug_tuple("Std").field(arg0).finish(),
            Self::Pty(_) => f.debug_tuple("Pty").finish(),
        }
    }
}

impl fmt::Debug for ChildOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Std { stdout, stderr } => f
                .debug_struct("Std")
                .field("stdout", stdout)
                .field("stderr", stderr)
                .finish(),
            Self::Pty(_) => f.debug_tuple("Pty").finish(),
        }
    }
}

impl ShutdownStyle {
    /// Process the shutdown style for the given child process.
    ///
    /// If an exit channel is provided, the exit code will be sent to the
    /// channel when the child process exits.
    async fn process(&self, child: &mut ChildHandle) -> ChildState {
        match self {
            // Windows doesn't give the ability to send a signal to a process so we
            // can't make use of the graceful shutdown timeout.
            #[allow(unused)]
            ShutdownStyle::Graceful(timeout) => {
                // try ro run the command for the given timeout
                #[cfg(unix)]
                {
                    let fut = async {
                        if let Some(pid) = child.pid() {
                            debug!("sending SIGINT to child {}", pid);
                            // kill takes negative pid to indicate that you want to use gpid
                            let pgid = -(pid as i32);
                            unsafe {
                                libc::kill(pgid, libc::SIGINT);
                            }
                            debug!("waiting for child {}", pid);
                            child.wait().await
                        } else {
                            // if there is no pid, then just report successful with no exit code
                            Ok(None)
                        }
                    };

                    debug!("starting shutdown");

                    let result = tokio::time::timeout(*timeout, fut).await;
                    match result {
                        // We ignore the exit code and mark it as killed since we sent a SIGINT
                        // This avoids reliance on an underlying process exiting with
                        // no exit code or a non-zero in order for turbo to operate correctly.
                        Ok(Ok(_exit_code)) => ChildState::Exited(ChildExit::Killed),
                        Ok(Err(_)) => ChildState::Exited(ChildExit::Failed),
                        Err(_) => {
                            debug!("graceful shutdown timed out, killing child");
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

/// The structure that holds logic regarding interacting with the underlying
/// child process
#[derive(Debug)]
struct ChildStateManager {
    shutdown_style: ShutdownStyle,
    task_state: Arc<RwLock<ChildState>>,
    exit_tx: watch::Sender<Option<ChildExit>>,
}

/// A child process that can be interacted with asynchronously.
///
/// This is a wrapper around the `tokio::process::Child` struct, which provides
/// a cross platform interface for spawning and managing child processes.
#[derive(Clone, Debug)]
pub struct Child {
    pid: Option<u32>,
    state: Arc<RwLock<ChildState>>,
    exit_channel: watch::Receiver<Option<ChildExit>>,
    stdin: Arc<Mutex<Option<ChildInput>>>,
    output: Arc<Mutex<Option<ChildOutput>>>,
    label: String,
}

#[derive(Clone, Debug)]
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
    #[tracing::instrument(skip(command), fields(command = command.label()))]
    pub fn spawn(
        command: Command,
        shutdown_style: ShutdownStyle,
        use_pty: bool,
    ) -> io::Result<Self> {
        let label = command.label();
        let SpawnResult {
            handle: mut child,
            io: ChildIO { stdin, output },
            controller,
        } = if use_pty {
            ChildHandle::spawn_pty(command)
        } else {
            ChildHandle::spawn_normal(command)
        }?;

        let pid = child.pid();

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
            // On Windows it is important that this gets dropped once the child process
            // exits
            let controller = controller;
            debug!("waiting for task");
            let manager = ChildStateManager {
                shutdown_style,
                task_state,
                exit_tx,
            };
            tokio::select! {
                command = command_rx.recv() => {
                    manager.handle_child_command(command, &mut child, controller).await;
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
            state,
            exit_channel: exit_rx,
            stdin: Arc::new(Mutex::new(stdin)),
            output: Arc::new(Mutex::new(output)),
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
            let child = {
                let state = self.state.read().await;

                match state.command_channel() {
                    Some(child) => child,
                    None => return,
                }
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

    fn stdin(&mut self) -> Option<ChildInput> {
        self.stdin.lock().unwrap().take()
    }

    fn outputs(&mut self) -> Option<ChildOutput> {
        self.output.lock().unwrap().take()
    }

    /// Wait for the `Child` to exit and pipe any stdout and stderr to the
    /// provided writer.
    #[tracing::instrument(skip_all)]
    pub async fn wait_with_piped_outputs<W: Write>(
        &mut self,
        stdout_pipe: W,
    ) -> Result<Option<ChildExit>, std::io::Error> {
        match self.outputs() {
            Some(ChildOutput::Std { stdout, stderr }) => {
                self.wait_with_piped_async_outputs(
                    stdout_pipe,
                    Some(BufReader::new(stdout)),
                    Some(BufReader::new(stderr)),
                )
                .await
            }
            Some(ChildOutput::Pty(output)) => {
                self.wait_with_piped_sync_output(stdout_pipe, std::io::BufReader::new(output))
                    .await
            }
            None => Ok(self.wait().await),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn wait_with_piped_sync_output<R: BufRead + Send + 'static>(
        &mut self,
        mut stdout_pipe: impl Write,
        mut stdout_lines: R,
    ) -> Result<Option<ChildExit>, std::io::Error> {
        // TODO: in order to not impose that a stdout_pipe is Send we send the bytes
        // across a channel
        let (byte_tx, mut byte_rx) = mpsc::channel(48);
        tokio::task::spawn_blocking(move || {
            let mut buffer = Vec::new();
            loop {
                match stdout_lines.read_until(b'\n', &mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {
                        if byte_tx.blocking_send(buffer.clone()).is_err() {
                            // A dropped receiver indicates that there was an issue writing to the
                            // pipe. We can stop reading output.
                            break;
                        }
                        buffer.clear();
                    }
                    Err(e) => return Err(e),
                }
            }
            if !buffer.is_empty() {
                byte_tx.blocking_send(buffer).ok();
            }
            Ok(())
        });

        let writer_fut = async {
            let mut result = Ok(());
            while let Some(mut bytes) = byte_rx.recv().await {
                add_trailing_newline(&mut bytes);
                if let Err(err) = stdout_pipe.write_all(&bytes) {
                    result = Err(err);
                    break;
                }
            }
            result
        };

        let (status, write_result) = tokio::join!(self.wait(), writer_fut);
        write_result?;

        Ok(status)
    }

    #[tracing::instrument(skip_all)]
    async fn wait_with_piped_async_outputs<R1: AsyncBufRead + Unpin, R2: AsyncBufRead + Unpin>(
        &mut self,
        mut stdout_pipe: impl Write,
        mut stdout_lines: Option<R1>,
        mut stderr_lines: Option<R2>,
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

        let mut stdout_buffer = Vec::new();
        let mut stderr_buffer = Vec::new();

        loop {
            tokio::select! {
                Some(result) = next_line(&mut stdout_lines, &mut stdout_buffer) => {
                    result?;
                    add_trailing_newline(&mut stdout_buffer);
                    stdout_pipe.write_all(&stdout_buffer)?;
                    stdout_buffer.clear();
                }
                Some(result) = next_line(&mut stderr_lines, &mut stderr_buffer) => {
                    result?;
                    add_trailing_newline(&mut stderr_buffer);
                    stdout_pipe.write_all(&stderr_buffer)?;
                    stderr_buffer.clear();
                }
                else => {
                    // In the case that both futures read a complete line
                    // the future not chosen in the select will return None if it's at EOF
                    // as the number of bytes read will be 0.
                    // We check and flush the buffers to avoid missing the last line of output.
                    if !stdout_buffer.is_empty() {
                        add_trailing_newline(&mut stdout_buffer);
                        stdout_pipe.write_all(&stdout_buffer)?;
                        stdout_buffer.clear();
                    }
                    if !stderr_buffer.is_empty() {
                        add_trailing_newline(&mut stderr_buffer);
                        stdout_pipe.write_all(&stderr_buffer)?;
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

// Adds a trailing newline if necessary to the buffer
fn add_trailing_newline(buffer: &mut Vec<u8>) {
    // If the line doesn't end with a newline, that indicates we hit a EOF.
    // We add a newline so output from other tasks doesn't get written to the same
    // line.
    if buffer.last() != Some(&b'\n') {
        buffer.push(b'\n');
    }
}

impl ChildStateManager {
    async fn handle_child_command(
        &self,
        command: Option<ChildCommand>,
        child: &mut ChildHandle,
        controller: Option<Box<dyn PtyController + Send>>,
    ) {
        let state = match command {
            // we received a command to stop the child process, or the channel was closed.
            // in theory this happens when the last child is dropped, however in practice
            // we will always get a `Permit` from the recv call before the channel can be
            // dropped, and the channel is not closed while there are still permits
            Some(ChildCommand::Stop) | None => {
                debug!("stopping child process");
                self.shutdown_style.process(child).await
            }
            // we received a command to kill the child process
            Some(ChildCommand::Kill) => {
                debug!("killing child process");
                ShutdownStyle::Kill.process(child).await
            }
        };
        match state {
            ChildState::Exited(exit) => {
                // ignore the send error, failure means the channel is dropped
                self.exit_tx.send(Some(exit)).ok();
            }
            ChildState::Running(_) => {
                debug_assert!(false, "child state should not be running after shutdown");
            }
        }
        drop(controller);

        {
            let mut task_state = self.task_state.write().await;
            *task_state = state;
        }
    }

    async fn handle_child_exit(&self, status: io::Result<Option<i32>>) {
        debug!("child process exited normally");
        // the child process exited
        let child_exit = match status {
            Ok(Some(c)) => ChildExit::Finished(Some(c)),
            // if we hit this case, it means that the child process was killed
            // by someone else, and we should report that it was killed
            Ok(None) => ChildExit::KilledExternal,
            Err(_e) => ChildExit::Failed,
        };
        {
            let mut task_state = self.task_state.write().await;
            *task_state = ChildState::Exited(child_exit);
        }

        // ignore the send error, the channel is dropped anyways
        self.exit_tx.send(Some(child_exit)).ok();
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, time::Duration};

    use futures::{stream::FuturesUnordered, StreamExt};
    use test_case::test_case;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tracing_test::traced_test;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{Child, ChildInput, ChildOutput, ChildState, Command};
    use crate::process::child::{ChildExit, ShutdownStyle};

    const STARTUP_DELAY: Duration = Duration::from_millis(500);
    // We skip testing PTY usage on Windows
    const TEST_PTY: bool = !cfg!(windows);
    const EOT: char = '\u{4}';

    fn find_script_dir() -> AbsoluteSystemPathBuf {
        let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
        let mut root = cwd;
        while !root.join_component(".git").exists() {
            root = root.parent().unwrap().to_owned();
        }
        root.join_components(&["crates", "turborepo-lib", "test", "scripts"])
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_pid(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        assert_matches!(child.pid(), Some(_));
        child.stop().await;

        let state = child.state.read().await;
        assert_matches!(&*state, ChildState::Exited(ChildExit::Killed));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_spawn(use_pty: bool) {
        let cmd = {
            let script = find_script_dir().join_component("hello_world.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

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

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_stdout(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        {
            let mut output = Vec::new();
            match child.outputs().unwrap() {
                ChildOutput::Std { mut stdout, .. } => {
                    stdout
                        .read_to_end(&mut output)
                        .await
                        .expect("Failed to read stdout");
                }
                ChildOutput::Pty(mut outputs) => {
                    outputs
                        .read_to_end(&mut output)
                        .expect("failed to read stdout");
                }
            };

            let output_str = String::from_utf8(output).expect("Failed to parse stdout");
            let trimmed_output = output_str.trim();
            let trimmed_output = trimmed_output.strip_prefix(EOT).unwrap_or(trimmed_output);

            assert_eq!(trimmed_output, "hello world");
        }

        child.wait().await;

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Exited(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_stdio(use_pty: bool) {
        let script = find_script_dir().join_component("stdin_stdout.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let input = "hello world";
        // drop stdin to close the pipe
        {
            match child.stdin().unwrap() {
                ChildInput::Std(mut stdin) => stdin.write_all(input.as_bytes()).await.unwrap(),
                ChildInput::Pty(mut stdin) => stdin.write_all(input.as_bytes()).unwrap(),
            }
        }

        let mut output = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => stdout.read_to_end(&mut output).await.unwrap(),
            ChildOutput::Pty(mut stdout) => stdout.read_to_end(&mut output).unwrap(),
        };

        let output_str = String::from_utf8(output).expect("Failed to parse stdout");
        let trimmed_out = output_str.trim();
        let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);

        assert!(trimmed_out.contains(input), "got: {}", trimmed_out);

        child.wait().await;

        let state = child.state.read().await;

        assert_matches!(&*state, ChildState::Exited(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown_timeout(use_pty: bool) {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_ignore.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Duration::from_millis(500)),
            use_pty,
        )
        .unwrap();

        let mut buf = vec![0; 4];
        // wait for the process to print "here"
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                stdout.read_exact(&mut buf).await.unwrap();
            }
            ChildOutput::Pty(mut stdout) => {
                stdout.read_exact(&mut buf).unwrap();
            }
        };
        child.stop().await;

        let state = child.state.read().await;

        // this should time out and be killed
        assert_matches!(&*state, ChildState::Exited(ChildExit::Killed));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown(use_pty: bool) {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_interruptable.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Duration::from_millis(500)),
            use_pty,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        child.stop().await;
        child.wait().await;

        let state = child.state.read().await;

        // We should ignore the exit code of the process and always treat it as killed
        assert_matches!(&*state, &ChildState::Exited(ChildExit::Killed));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_detect_killed_someone_else(use_pty: bool) {
        let cmd = {
            let script = find_script_dir().join_component("sleep_5_interruptable.js");
            let mut cmd = Command::new("node");
            cmd.args([script.as_std_path()]);
            cmd
        };

        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Duration::from_millis(500)),
            use_pty,
        )
        .unwrap();

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

        let ChildState::Exited(state) = &*state else {
            panic!("expected child to have exited after wait call");
        };

        #[cfg(unix)]
        assert_matches!(state, ChildExit::KilledExternal);
        #[cfg(not(unix))]
        assert_matches!(state, ChildExit::Finished(Some(3)));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_wait_with_output(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        let mut out = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

        let out = String::from_utf8(out).unwrap();
        let trimmed_out = out.trim();
        let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);

        assert_eq!(trimmed_out, "hello world");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(true)]
    #[tokio::test]
    async fn test_wait_with_single_output(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world_hello_moon.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        let mut buffer = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut buffer).await.unwrap();

        let output = String::from_utf8(buffer).unwrap();

        // There are no ordering guarantees so we just check that both logs made it
        let expected_stdout = "hello world";
        let expected_stderr = "hello moon";
        assert!(output.contains(expected_stdout), "got: {}", output);
        assert!(output.contains(expected_stderr), "got: {}", output);
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_wait_with_with_non_utf8_output(use_pty: bool) {
        let script = find_script_dir().join_component("hello_non_utf8.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        let mut out = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

        let expected = &[0, 159, 146, 150];
        let trimmed_out = out.trim_ascii();
        let trimmed_out = trimmed_out.strip_prefix(&[4]).unwrap_or(trimmed_out);
        assert_eq!(trimmed_out, expected);
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_no_newline(use_pty: bool) {
        let script = find_script_dir().join_component("hello_no_line.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        let mut out = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

        let output = String::from_utf8(out).unwrap();
        let trimmed_out = output.trim();
        let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);
        assert!(
            output.ends_with('\n'),
            "expected newline to be added: {}",
            output
        );
        assert_eq!(trimmed_out, "look ma, no newline!");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[cfg(unix)]
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_kill_process_group(use_pty: bool) {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "while true; do sleep 0.2; done"]);
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Duration::from_millis(100)),
            use_pty,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let exit = child.stop().await;

        assert_matches!(exit, Some(ChildExit::Killed));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_multistop(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty).unwrap();

        let mut stops = FuturesUnordered::new();
        for _ in 1..10 {
            let mut child = child.clone();
            stops.push(async move {
                child.stop().await;
            });
        }

        while tokio::time::timeout(Duration::from_secs(5), stops.next())
            .await
            .expect("timed out")
            .is_some()
        {}
    }
}
