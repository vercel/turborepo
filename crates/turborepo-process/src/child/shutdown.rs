use std::time::Duration;

use tokio::sync::mpsc;
use tracing::debug;

use super::{ChildCommand, ChildHandle};

#[cfg(unix)]
const PTY_PROCESS_GROUP_SIGINT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ChildExit {
    Finished(Option<i32>),
    /// The child process exited during graceful shutdown.
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

#[derive(Debug, Clone, Copy)]
pub enum ShutdownStyle {
    /// On Unix this sends SIGINT to the process group. On Windows, Turbo cannot
    /// send a signal directly, so it waits for an externally delivered console
    /// event or an explicit kill.
    ///
    /// `Graceful(Some(timeout))` escalates to `Kill` after `timeout` elapses.
    /// `Graceful(None)` waits indefinitely until an explicit `Kill` command
    /// arrives.
    Graceful(Option<Duration>),

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

impl ShutdownStyle {
    /// Process the shutdown style for the given child process.
    ///
    /// If an exit channel is provided, the exit code will be sent to the
    /// channel when the child process exits.
    pub(super) async fn process(
        &self,
        child: &mut ChildHandle,
        command_rx: &mut mpsc::Receiver<ChildCommand>,
    ) -> ChildExit {
        match self {
            #[allow(unused)]
            ShutdownStyle::Graceful(timeout) => {
                // try ro run the command for the given timeout
                #[cfg(unix)]
                {
                    let Some(pid) = child.pid() else {
                        return ChildExit::Interrupted;
                    };

                    let pid = pid as libc::pid_t;
                    let mut process_group_interrupt_sent = child.send_graceful_interrupt(pid);
                    let process_group_interrupt_deadline =
                        tokio::time::Instant::now() + PTY_PROCESS_GROUP_SIGINT_DELAY;
                    debug!("waiting for child {}", pid);

                    let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
                    let mut command_rx_open = true;

                    let exit = loop {
                        match deadline {
                            Some(deadline) => {
                                tokio::select! {
                                    result = child.wait() => {
                                        break match result {
                                            Ok(_exit_code) => ChildExit::Interrupted,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                    _ = tokio::time::sleep_until(process_group_interrupt_deadline), if !process_group_interrupt_sent => {
                                        child.send_fallback_graceful_interrupt(pid);
                                        process_group_interrupt_sent = true;
                                    }
                                    command = command_rx.recv(), if command_rx_open => {
                                        match command {
                                            Some(ChildCommand::Kill) => {
                                                debug!("graceful shutdown interrupted, killing child");
                                                break match child.kill().await {
                                                    Ok(_) => ChildExit::Killed,
                                                    Err(_) => ChildExit::Failed,
                                                };
                                            }
                                            Some(ChildCommand::Shutdown(_)) => {}
                                            None => command_rx_open = false,
                                        }
                                    }
                                    _ = tokio::time::sleep_until(deadline) => {
                                        debug!("graceful shutdown timed out, killing child");
                                        break match child.kill().await {
                                            Ok(_) => ChildExit::Killed,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                }
                            }
                            None => {
                                tokio::select! {
                                    result = child.wait() => {
                                        break match result {
                                            Ok(_exit_code) => ChildExit::Interrupted,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                    _ = tokio::time::sleep_until(process_group_interrupt_deadline), if !process_group_interrupt_sent => {
                                        child.send_fallback_graceful_interrupt(pid);
                                        process_group_interrupt_sent = true;
                                    }
                                    command = command_rx.recv(), if command_rx_open => {
                                        match command {
                                            Some(ChildCommand::Kill) => {
                                                debug!("graceful shutdown interrupted, killing child");
                                                break match child.kill().await {
                                                    Ok(_) => ChildExit::Killed,
                                                    Err(_) => ChildExit::Failed,
                                                };
                                            }
                                            Some(ChildCommand::Shutdown(_)) => {}
                                            None => command_rx_open = false,
                                        }
                                    }
                                }
                            }
                        }
                    };

                    if exit == ChildExit::Interrupted {
                        child
                            .wait_for_process_group_exit(
                                pid,
                                deadline,
                                command_rx,
                                &mut command_rx_open,
                            )
                            .await
                    } else {
                        exit
                    }
                }

                #[cfg(windows)]
                {
                    // Windows consoles deliver Ctrl+C to attached child processes.
                    // Windows PTY children do not receive that console event, so
                    // also write ETX to their PTY stdin before waiting.
                    child.send_graceful_interrupt();
                    let deadline = timeout.map(|timeout| tokio::time::Instant::now() + timeout);
                    let mut command_rx_open = true;

                    let exit = loop {
                        match deadline {
                            Some(deadline) => {
                                tokio::select! {
                                    result = child.wait_for_graceful_exit() => {
                                        break match result {
                                            Ok(_exit_code) => ChildExit::Interrupted,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                    command = command_rx.recv(), if command_rx_open => {
                                        match command {
                                            Some(ChildCommand::Kill) => {
                                                debug!("graceful shutdown interrupted, killing child");
                                                break match child.kill().await {
                                                    Ok(_) => ChildExit::Killed,
                                                    Err(_) => ChildExit::Failed,
                                                };
                                            }
                                            Some(ChildCommand::Shutdown(_)) => {}
                                            None => command_rx_open = false,
                                        }
                                    }
                                    _ = tokio::time::sleep_until(deadline) => {
                                        debug!("graceful shutdown timed out, killing child");
                                        break match child.kill().await {
                                            Ok(_) => ChildExit::Killed,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                }
                            }
                            None => {
                                tokio::select! {
                                    result = child.wait_for_graceful_exit() => {
                                        break match result {
                                            Ok(_exit_code) => ChildExit::Interrupted,
                                            Err(_) => ChildExit::Failed,
                                        };
                                    }
                                    command = command_rx.recv(), if command_rx_open => {
                                        match command {
                                            Some(ChildCommand::Kill) => {
                                                debug!("graceful shutdown interrupted, killing child");
                                                break match child.kill().await {
                                                    Ok(_) => ChildExit::Killed,
                                                    Err(_) => ChildExit::Failed,
                                                };
                                            }
                                            Some(ChildCommand::Shutdown(_)) => {}
                                            None => command_rx_open = false,
                                        }
                                    }
                                }
                            }
                        }
                    };

                    if exit == ChildExit::Interrupted {
                        child
                            .wait_for_job_exit(deadline, command_rx, &mut command_rx_open)
                            .await
                    } else {
                        exit
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
