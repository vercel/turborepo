use std::{
    fmt,
    io::{self, BufRead, Read, Write},
};

use tokio::{
    io::{AsyncBufRead, AsyncReadExt, BufReader},
    sync::mpsc,
};
use tracing::trace;

use super::{Child, ChildExit};

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
        // A channel keeps this from requiring `stdout_pipe` to be `Send`.
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
        // append a trailing newline for partial output at EOF, matching
        // the previous per-line reader's add_trailing_newline behavior.
        let mut stdout_ends_with_newline = true;
        let mut stderr_ends_with_newline = true;

        let mut is_exited = false;
        let mut exit_status = None;
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
                    // Exiting does not imply EOF: output can still be buffered,
                    // or a descendant can keep either pipe open. Read all output
                    // regardless of exit status, including during shutdown.
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

        self.cleanup_if_successful(exit_status);
        Ok(exit_status)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    use futures::poll;
    use test_case::test_case;
    use tokio::{io::AsyncWriteExt, sync::watch};

    use super::*;
    use crate::child::{ChildCommandChannel, ShutdownStyle};

    // Make exit ready independently of output, without process-scheduling races.
    fn exited_child(status: ChildExit, closing: bool) -> Child {
        let (_, exit_channel) = watch::channel(Some(status));
        let (command_channel, _) = ChildCommandChannel::new();
        Child {
            pid: None,
            #[cfg(unix)]
            target_identity: None,
            #[cfg(windows)]
            root_identity: None,
            command_channel,
            exit_channel,
            stdin: Arc::new(Mutex::new(None)),
            output: Arc::new(Mutex::new(None)),
            label: "output drain test".into(),
            shutdown_style: ShutdownStyle::Kill,
            closing: Arc::new(AtomicBool::new(closing)),
            _pty_test_guard: None,
        }
    }

    #[test_case(ChildExit::Finished(Some(0)), false; "success")]
    #[test_case(ChildExit::Finished(Some(42)), false; "failure")]
    #[test_case(ChildExit::Interrupted, true; "shutdown")]
    #[test_case(ChildExit::Killed, false; "restart")]
    #[test_case(ChildExit::KilledExternal, false; "external_kill")]
    #[tokio::test(start_paused = true)]
    async fn drains_both_streams_after_exit(status: ChildExit, closing: bool) {
        let mut child = exited_child(status, closing);
        let (mut stdout_tx, stdout_rx) = tokio::io::duplex(1024);
        let (mut stderr_tx, stderr_rx) = tokio::io::duplex(1024);
        let mut output = Vec::new();
        {
            let drain = child.wait_with_piped_async_outputs(
                &mut output,
                Some(BufReader::new(stdout_rx)),
                Some(BufReader::new(stderr_rx)),
            );
            tokio::pin!(drain);
            // Both reads are pending: the ready exit branch must run first.
            assert!(poll!(drain.as_mut()).is_pending());
            // Held-open pipes must not cause an automatic early return.
            tokio::time::advance(std::time::Duration::from_secs(10)).await;
            assert!(poll!(drain.as_mut()).is_pending());

            let stdout = async move {
                stdout_tx.write_all(&vec![b'x'; 262144]).await.unwrap();
                stdout_tx.write_all(b"\nFINAL_STDOUT\n").await.unwrap();
            };
            let stderr = async move {
                stderr_tx.write_all(&vec![b'y'; 262144]).await.unwrap();
                stderr_tx.write_all(b"\nFINAL_STDERR\n").await.unwrap();
            };
            let (exit, (), ()) = tokio::join!(drain, stdout, stderr);
            assert_eq!(exit.unwrap(), Some(status));
        }
        assert_eq!(output.iter().filter(|&&b| b == b'x').count(), 262144);
        assert_eq!(output.iter().filter(|&&b| b == b'y').count(), 262144);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("FINAL_STDOUT\n"));
        assert!(output.contains("FINAL_STDERR\n"));
    }

    #[test_case(false; "stdout")]
    #[test_case(true; "stderr")]
    #[tokio::test]
    async fn failure_drains_with_slow_sink(use_stderr: bool) {
        struct SlowSink(Vec<u8>);
        impl Write for SlowSink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                std::thread::sleep(std::time::Duration::from_millis(200));
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let status = ChildExit::Finished(Some(42));
        let mut child = exited_child(status, false);
        let (mut tx, rx) = tokio::io::duplex(8192);
        let mut output = SlowSink(Vec::new());
        let mut expected = vec![0xff; 32768];
        expected.extend_from_slice(b"FINAL_LINE");
        {
            let (stdout, stderr) = if use_stderr {
                (None, Some(BufReader::new(rx)))
            } else {
                (Some(BufReader::new(rx)), None)
            };
            let drain = child.wait_with_piped_async_outputs(&mut output, stdout, stderr);
            tokio::pin!(drain);
            assert!(poll!(drain.as_mut()).is_pending());
            let producer = async {
                tx.write_all(&expected).await.unwrap();
                drop(tx);
            };
            let (exit, ()) = tokio::join!(drain, producer);
            assert_eq!(exit.unwrap(), Some(status));
        }
        expected.push(b'\n');
        assert_eq!(output.0, expected);
    }
}
