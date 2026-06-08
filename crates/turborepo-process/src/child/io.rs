#[cfg(windows)]
use std::sync::{Arc, Mutex};
use std::{
    fmt,
    io::{self, BufRead, Read, Write},
};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader},
    sync::mpsc,
};
use tracing::{debug, trace};

use super::{Child, ChildExit};

const POST_EXIT_OUTPUT_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

pub(super) struct ChildIO {
    pub(super) stdin: Option<ChildInput>,
    pub(super) output: Option<ChildOutput>,
}

pub(super) enum ChildInput {
    Std(tokio::process::ChildStdin),
    Pty(Box<dyn Write + Send>),
}

#[cfg(windows)]
#[derive(Clone)]
pub(super) struct SharedPtyWriter(Arc<Mutex<Box<dyn Write + Send>>>);

#[cfg(windows)]
impl SharedPtyWriter {
    pub(super) fn new(writer: Box<dyn Write + Send>) -> Self {
        Self(Arc::new(Mutex::new(writer)))
    }
}

#[cfg(windows)]
impl Write for SharedPtyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush()
    }
}

#[derive(Debug)]
pub struct ChildStdinGuard {
    _stdin: ChildInput,
}

pub enum ChildStdin {
    Writable(Box<dyn Write + Send>),
    Guard(ChildStdinGuard),
}

impl fmt::Debug for ChildStdin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Writable(_) => f.debug_tuple("Writable").finish(),
            Self::Guard(guard) => f.debug_tuple("Guard").field(guard).finish(),
        }
    }
}

pub(super) enum ChildOutput {
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

impl Child {
    pub(super) fn stdin_inner(&mut self) -> Option<ChildInput> {
        self.stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub(super) fn outputs(&self) -> Option<ChildOutput> {
        self.output
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub fn stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        let stdin = self.stdin_inner()?;
        match stdin {
            ChildInput::Std(_) => None,
            ChildInput::Pty(stdin) => Some(stdin),
        }
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        let stdin = self.stdin_inner()?;
        match stdin {
            ChildInput::Std(stdin) => Some(ChildStdin::Guard(ChildStdinGuard {
                _stdin: ChildInput::Std(stdin),
            })),
            ChildInput::Pty(stdin) => Some(ChildStdin::Writable(stdin)),
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
                // On Unix, drop stdin before reading so the master PTY writer
                // sends EOT and releases its fd, allowing the reader to reach
                // EOF once the controller is dropped after the child exits.
                //
                // On Windows, do NOT drop stdin here: ConPTY treats a closed
                // stdin pipe as the session ending and immediately terminates
                // the child process.
                if !cfg!(windows) {
                    drop(self.stdin_inner());
                }
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
        self.cleanup_if_successful(status);

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
        let mut exit_status = None;
        let mut draining_after_exit = false;
        let mut drain_deadline = tokio::time::Instant::now() + POST_EXIT_OUTPUT_DRAIN_TIMEOUT;
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
                    exit_status = status;
                    // We don't abort in the cases of a zero exit code as we could be
                    // caching this task and should read all the logs it produces.
                    if status == Some(ChildExit::Finished(Some(0))) {
                        continue;
                    }

                    if self.is_closing() {
                        // During Turbo-initiated shutdown, give the pipe readers a
                        // short grace window to pull the child's final log lines.
                        draining_after_exit = true;
                        drain_deadline = tokio::time::Instant::now() + POST_EXIT_OUTPUT_DRAIN_TIMEOUT;
                    } else {
                        debug!("child process failed, skipping reading stdout/stderr");
                        return Ok(status);
                    }
                }
                _ = tokio::time::sleep_until(drain_deadline), if draining_after_exit => {
                    trace!("post-exit output drain timed out");
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
                    return Ok(exit_status);
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

        let status = exit_status.or(self.wait().await);
        self.cleanup_if_successful(status);
        Ok(status)
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
