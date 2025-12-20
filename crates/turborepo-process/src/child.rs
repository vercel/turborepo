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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use portable_pty::{Child as PtyChild, MasterPty as PtyController, native_pty_system};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    process::Command as TokioCommand,
    sync::{mpsc, watch},
};
use tracing::{debug, trace};

use super::{Command, PtySize};

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ChildExit {
    Finished(Option<i32>),
    /// The child process was sent an interrupt and shut down on it's own
    Interrupted,
    /// The child process was killed, it could either be explicitly killed or it
    /// did not respond to an interrupt and was killed as a result
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
#[allow(dead_code)]
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
    pub fn spawn_pty(command: Command, size: PtySize) -> io::Result<SpawnResult> {
        let keep_stdin_open = command.will_open_stdin();

        let command = portable_pty::CommandBuilder::from(command);
        let pty_system = native_pty_system();
        let size = portable_pty::PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system
            .openpty(size)
            .map_err(|err| match err.downcast() {
                Ok(err) => err,
                Err(err) => io::Error::other(err),
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
                Err(err) => io::Error::other(err),
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

    /// Perform a `wait` syscall on the child until it exits
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
    async fn process(&self, child: &mut ChildHandle) -> ChildExit {
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
                            if unsafe { libc::kill(pgid, libc::SIGINT) } == -1 {
                                debug!("failed to send SIGINT to {pgid}");
                            };
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
                        // We ignore the exit code and mark it as interrupted since we sent a SIGINT
                        // This avoids reliance on an underlying process exiting with
                        // no exit code or a non-zero in order for turbo to operate correctly.
                        Ok(Ok(_exit_code)) => ChildExit::Interrupted,
                        Ok(Err(_)) => ChildExit::Failed,
                        Err(_) => {
                            debug!("graceful shutdown timed out, killing child");
                            match child.kill().await {
                                Ok(_) => ChildExit::Killed,
                                Err(_) => ChildExit::Failed,
                            }
                        }
                    }
                }

                #[cfg(windows)]
                {
                    debug!("timeout not supported on windows, killing");
                    match child.kill().await {
                        Ok(_) => ChildExit::Killed,
                        Err(_) => ChildExit::Failed,
                    }
                }
            }
            ShutdownStyle::Kill => match child.kill().await {
                Ok(_) => ChildExit::Killed,
                Err(_) => ChildExit::Failed,
            },
        }
    }
}

/// The structure that holds logic regarding interacting with the underlying
/// child process
#[derive(Debug)]
struct ChildStateManager {
    shutdown_style: ShutdownStyle,
    exit_tx: watch::Sender<Option<ChildExit>>,
    shutdown_initiated: bool,
}

/// A child process that can be interacted with asynchronously.
///
/// This is a wrapper around the `tokio::process::Child` struct, which provides
/// a cross platform interface for spawning and managing child processes.
#[derive(Clone, Debug)]
pub struct Child {
    pid: Option<u32>,
    command_channel: ChildCommandChannel,
    exit_channel: watch::Receiver<Option<ChildExit>>,
    stdin: Arc<Mutex<Option<ChildInput>>>,
    output: Arc<Mutex<Option<ChildOutput>>>,
    label: String,
    /// Flag indicating this child is being stopped as part of a shutdown of the
    /// ProcessManager, rather than individually stopped.
    closing: Arc<AtomicBool>,
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
        pty_size: Option<PtySize>,
    ) -> io::Result<Self> {
        let label = command.label();
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
            command_channel: command_tx,
            exit_channel: exit_rx,
            stdin: Arc::new(Mutex::new(stdin)),
            output: Arc::new(Mutex::new(output)),
            label,
            closing: Arc::new(AtomicBool::new(false)),
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
        // if this fails, it's because the channel is dropped (toctou)
        // we can just ignore it
        self.command_channel.stop().await.ok();
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

    fn stdin_inner(&mut self) -> Option<ChildInput> {
        self.stdin.lock().unwrap().take()
    }

    fn outputs(&self) -> Option<ChildOutput> {
        self.output.lock().unwrap().take()
    }

    pub fn stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        let stdin = self.stdin_inner()?;
        match stdin {
            ChildInput::Std(_) => None,
            ChildInput::Pty(stdin) => Some(stdin),
        }
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
            let mut buffer = [0; 1024];
            let mut last_byte = None;
            loop {
                match stdout_lines.read(&mut buffer) {
                    Ok(0) => {
                        if !matches!(last_byte, Some(b'\n')) {
                            // Ignore if this fails as we already are shutting down
                            byte_tx.blocking_send(vec![b'\n']).ok();
                        }
                        break;
                    }
                    Ok(n) => {
                        let mut bytes = Vec::with_capacity(n);
                        bytes.extend_from_slice(&buffer[..n]);
                        last_byte = bytes.last().copied();
                        if byte_tx.blocking_send(bytes).is_err() {
                            // A dropped receiver indicates that there was an issue writing to the
                            // pipe. We can stop reading output.
                            break;
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        });

        let writer_fut = async {
            let mut result = Ok(());
            while let Some(bytes) = byte_rx.recv().await {
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
                    Ok(0) => {
                        trace!("reached EOF");
                        None
                    }
                    Ok(_) => Some(Ok(())),
                    Err(e) => Some(Err(e)),
                },
                None => None,
            }
        }

        let mut stdout_buffer = Vec::new();
        let mut stderr_buffer = Vec::new();

        let mut is_exited = false;
        loop {
            tokio::select! {
                Some(result) = next_line(&mut stdout_lines, &mut stdout_buffer) => {
                    trace!("processing stdout line");
                    result?;
                    add_trailing_newline(&mut stdout_buffer);
                    stdout_pipe.write_all(&stdout_buffer)?;
                    stdout_buffer.clear();
                }
                Some(result) = next_line(&mut stderr_lines, &mut stderr_buffer) => {
                    trace!("processing stderr line");
                    result?;
                    add_trailing_newline(&mut stderr_buffer);
                    stdout_pipe.write_all(&stderr_buffer)?;
                    stderr_buffer.clear();
                }
                status = self.wait(), if !is_exited => {
                    trace!("child process exited: {}", self.label());
                    is_exited = true;
                    // We don't abort in the cases of a zero exit code as we could be
                    // caching this task and should read all the logs it produces.
                    if status != Some(ChildExit::Finished(Some(0))) {
                        debug!("child process failed, skipping reading stdout/stderr");
                        return Ok(status);
                    }
                }
                else => {
                    trace!("flushing child stdout/stderr buffers");
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

    /// Mark this child as being stopped as part of a ProcessManager shutdown
    pub fn set_closing(&self) {
        self.closing.store(true, Ordering::Release);
    }

    /// Check if this child was stopped as part of a ProcessManager shutdown
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
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
        let exit = match command {
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
        // ignore the send error, failure means the channel is dropped
        trace!("sending child exit after shutdown");
        self.exit_tx.send(Some(exit)).ok();
        drop(controller);
    }

    async fn handle_child_exit(&self, status: io::Result<Option<i32>>) {
        // If a shutdown was initiated we defer to the exit returned by
        // `ShutdownStyle::process` as that will have information if the child
        // responded to a SIGINT or a SIGKILL. The `wait` response this function
        // gets in that scenario would make it appear that the child was killed by an
        // external process.
        if self.shutdown_initiated {
            return;
        }

        debug!("child process exited normally");
        // the child process exited
        let child_exit = match status {
            Ok(Some(c)) => ChildExit::Finished(Some(c)),
            // if we hit this case, it means that the child process was killed
            // by someone else, and we should report that it was killed
            Ok(None) => ChildExit::KilledExternal,
            Err(_e) => ChildExit::Failed,
        };

        // ignore the send error, the channel is dropped anyways
        trace!("sending child exit");
        self.exit_tx.send(Some(child_exit)).ok();
    }
}

#[cfg(test)]
impl Child {
    // Helper method for checking if child is running
    fn is_running(&self) -> bool {
        !self.command_channel.0.is_closed()
    }
}

#[cfg(test)]
mod test {
    use std::{assert_matches::assert_matches, time::Duration};

    use futures::{StreamExt, stream::FuturesUnordered};
    use test_case::test_case;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tracing_test::traced_test;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{Child, ChildInput, ChildOutput, Command};
    use crate::{
        PtySize,
        child::{ChildExit, ShutdownStyle},
    };

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
        root.join_components(&["crates", "turborepo-process", "test", "scripts"])
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_pid(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        assert_matches!(child.pid(), Some(_));
        child.stop().await;

        let exit = child.wait().await;
        assert_matches!(exit, Some(ChildExit::Killed));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tracing_test::traced_test]
    #[tokio::test]
    async fn test_wait(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        let exit1 = child.wait().await;
        let exit2 = child.wait().await;
        assert_matches!(exit1, Some(ChildExit::Finished(Some(0))));
        assert_matches!(exit2, Some(ChildExit::Finished(Some(0))));
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

        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        assert!(child.is_running());

        let code = child.wait().await;
        assert_eq!(code, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_stdout(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

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

        let exit = child.wait().await;

        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_stdio(use_pty: bool) {
        let script = find_script_dir().join_component("stdin_stdout.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let input = "hello world";
        // drop stdin to close the pipe
        {
            match child.stdin_inner().unwrap() {
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

        assert!(trimmed_out.contains(input), "got: {trimmed_out}");

        let exit = child.wait().await;
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
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
            ShutdownStyle::Graceful(Duration::from_millis(1000)),
            use_pty.then(PtySize::default),
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

        let exit = child.wait().await;
        // this should time out and be killed
        assert_matches!(exit, Some(ChildExit::Killed));
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
            ShutdownStyle::Graceful(Duration::from_millis(1000)),
            use_pty.then(PtySize::default),
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // We need to read the child output otherwise the child will be unable to
        // cleanly shut down as it waits for the receiving end of the PTY to read
        // the output before exiting.
        let mut output_child = child.clone();
        tokio::task::spawn(async move {
            let mut output = Vec::new();
            output_child.wait_with_piped_outputs(&mut output).await.ok();
        });

        child.stop().await;
        let exit = child.wait().await;

        // We should ignore the exit code of the process and always treat it as killed
        if cfg!(windows) {
            // There are no signals on Windows so we must kill
            assert_matches!(exit, Some(ChildExit::Killed));
        } else {
            assert_matches!(exit, Some(ChildExit::Interrupted));
        }
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
            ShutdownStyle::Graceful(Duration::from_millis(1000)),
            use_pty.then(PtySize::default),
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
                windows_sys::Win32::System::Threading::TerminateProcess(
                    windows_sys::Win32::System::Threading::OpenProcess(
                        windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
                        0,
                        pid,
                    ),
                    3,
                );
            }
        }

        let exit = child.wait().await;

        #[cfg(unix)]
        assert_matches!(exit, Some(ChildExit::KilledExternal));
        #[cfg(not(unix))]
        assert_matches!(exit, Some(ChildExit::Finished(Some(3))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_wait_with_output(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

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
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        let mut buffer = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut buffer).await.unwrap();

        let output = String::from_utf8(buffer).unwrap();

        // There are no ordering guarantees so we just check that both logs made it
        let expected_stdout = "hello world";
        let expected_stderr = "hello moon";
        assert!(output.contains(expected_stdout), "got: {output}");
        assert!(output.contains(expected_stderr), "got: {output}");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_wait_with_with_non_utf8_output(use_pty: bool) {
        let script = find_script_dir().join_component("hello_non_utf8.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

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
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        let mut out = Vec::new();

        let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

        let output = String::from_utf8(out).unwrap();
        let trimmed_out = output.trim();
        let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);
        assert!(
            output.ends_with('\n'),
            "expected newline to be added: {output}"
        );
        assert_eq!(trimmed_out, "look ma, no newline!");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    #[cfg(unix)]
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_kill_process_group(use_pty: bool) {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "while true; do sleep 0.2; done"]);
        let mut child = Child::spawn(
            cmd,
            // Bumping this to give ample time for the process to respond to the SIGINT to reduce
            // flakiness inherent with sending and receiving signals.
            ShutdownStyle::Graceful(Duration::from_millis(1000)),
            use_pty.then(PtySize::default),
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // We need to read the child output otherwise the child will be unable to
        // cleanly shut down as it waits for the receiving end of the PTY to read
        // the output before exiting.
        let mut output_child = child.clone();
        tokio::task::spawn(async move {
            let mut output = Vec::new();
            output_child.wait_with_piped_outputs(&mut output).await.ok();
        });

        let exit = child.stop().await;

        // On Unix systems, when not using a PTY, shell commands may not properly
        // respond to SIGINT and will timeout, resulting in being killed rather
        // than interrupted. This is different from using a proper interruptible
        // program like Node.js that naturally handles signals correctly
        // regardless of PTY usage.
        if cfg!(unix) && !use_pty {
            // On Unix without PTY, shell scripts may not respond to SIGINT properly
            assert_matches!(exit, Some(ChildExit::Killed) | Some(ChildExit::Interrupted));
        } else {
            assert_matches!(exit, Some(ChildExit::Interrupted));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_orphan_process() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hello; sleep 120; echo done"]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let child_pid = child.pid().unwrap() as i32;
        // We don't kill the process group to simulate what an external program might do
        unsafe {
            libc::kill(child_pid, libc::SIGKILL);
        }

        let exit = child.wait().await;
        assert_matches!(exit, Some(ChildExit::KilledExternal));

        let mut output = Vec::new();
        match tokio::time::timeout(
            Duration::from_millis(500),
            child.wait_with_piped_outputs(&mut output),
        )
        .await
        {
            Ok(exit_status) => {
                assert_matches!(exit_status, Ok(Some(ChildExit::KilledExternal)));
            }
            Err(_) => panic!("expected wait_with_piped_outputs to exit after it was killed"),
        }
    }

    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_multistop(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

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
