use std::{
    fmt,
    io::{self, BufRead, Read, Write},
};

use tokio::{
    io::{AsyncBufRead, AsyncReadExt, BufReader},
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
        /// Reads the next chunk of the stream into `buffer`. Returns None
        /// once the stream has reached EOF.
        ///
        /// Chunk-oriented instead of line-oriented: nothing downstream of the
        /// sink is line-aware, so forwarding bounded chunks avoids one async
        /// read, wakeup, delimiter scan, and sink write per line. The buffer
        /// is flushed to the sink on every chunk, so memory stays bounded
        /// even for output without newlines.
        async fn next_chunk<R: AsyncBufRead + Unpin>(
            stream: &mut Option<R>,
            buffer: &mut Vec<u8>,
        ) -> Option<Result<(), io::Error>> {
            match stream {
                Some(stream) => {
                    buffer.clear();
                    match stream.read_buf(buffer).await {
                        Ok(0) => {
                            trace!("reached EOF");
                            None
                        }
                        Ok(_) => Some(Ok(())),
                        Err(e) => Some(Err(e)),
                    }
                }
                None => None,
            }
        }

        const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;

        let mut stdout_buffer = Vec::with_capacity(OUTPUT_CHUNK_BYTES);
        let mut stderr_buffer = Vec::with_capacity(OUTPUT_CHUNK_BYTES);
        // Whether the last byte written for the stream was a newline. Used to
        // append a trailing newline for partial output at drain/EOF, matching
        // the previous per-line reader's add_trailing_newline behavior.
        let mut stdout_ends_with_newline = true;
        let mut stderr_ends_with_newline = true;

        let mut is_exited = false;
        let mut exit_status = None;
        let mut draining_after_exit = false;
        let mut drain_deadline = tokio::time::Instant::now() + POST_EXIT_OUTPUT_DRAIN_TIMEOUT;
        loop {
            tokio::select! {
                Some(result) = next_chunk(&mut stdout_lines, &mut stdout_buffer) => {
                    trace!("processing stdout chunk");
                    result?;
                    stdout_pipe.write_all(&stdout_buffer)?;
                    stdout_ends_with_newline = stdout_buffer.last() == Some(&b'\n');
                }
                Some(result) = next_chunk(&mut stderr_lines, &mut stderr_buffer) => {
                    trace!("processing stderr chunk");
                    result?;
                    stdout_pipe.write_all(&stderr_buffer)?;
                    stderr_ends_with_newline = stderr_buffer.last() == Some(&b'\n');
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
                    if !stdout_ends_with_newline {
                        stdout_pipe.write_all(b"\n")?;
                    }
                    if !stderr_ends_with_newline {
                        stdout_pipe.write_all(b"\n")?;
                    }
                    return Ok(exit_status);
                }
                else => {
                    trace!("flushing child stdout/stderr buffers");
                    // Both streams are at EOF (or absent): forward a trailing
                    // newline when the final chunk did not end with one, so
                    // output from other tasks never shares the line.
                    if !stdout_ends_with_newline {
                        stdout_pipe.write_all(b"\n")?;
                    }
                    if !stderr_ends_with_newline {
                        stdout_pipe.write_all(b"\n")?;
                    }
                    break;
                }
            }
        }
        // The chunk buffers still hold the final chunk (already written);
        // dropping them here is fine.

        let status = exit_status.or(self.wait().await);
        self.cleanup_if_successful(status);
        Ok(status)
    }
}
