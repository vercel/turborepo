use std::{io, time::Duration};

use portable_pty::{Child as PtyChild, MasterPty as PtyController, native_pty_system};
use tokio::{process::Command as TokioCommand, sync::mpsc};
use tracing::debug;

use super::{ChildCommand, ChildExit, ChildIO, ChildInput, ChildOutput};
use crate::{Command, PtySize};

const CHILD_POLL_INTERVAL: Duration = Duration::from_micros(50);
#[cfg(any(unix, windows))]
const PROCESS_TREE_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(windows)]
const WINDOWS_DESCENDANT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct ChildHandle {
    pid: Option<u32>,
    imp: ChildHandleImpl,
    #[cfg(unix)]
    shutdown_semantics: ShutdownSemantics,
    #[cfg(unix)]
    pub(super) target_identity: Option<TargetIdentity>,
    #[cfg(unix)]
    pty_controller_fd: Option<libc::c_int>,
    #[cfg(windows)]
    _job: Option<crate::job_object::JobObject>,
    /// Shared handle to the ConPTY input pipe, used to deliver a Ctrl-C
    /// keystroke during graceful shutdown. ConPTY children are attached to a
    /// pseudoconsole rather than turbo's console, so console Ctrl-C events
    /// never reach them on their own.
    #[cfg(windows)]
    pty_input: Option<SharedPtyInput>,
}

#[cfg(windows)]
type SharedPtyInput = std::sync::Arc<std::sync::Mutex<Box<dyn io::Write + Send>>>;

/// A `Write` handle to the ConPTY input pipe that shares ownership with
/// `ChildHandle::pty_input` so the shutdown path can inject a Ctrl-C while
/// stdin forwarding holds the writer.
#[cfg(windows)]
struct SharedPtyWriter(SharedPtyInput);

#[cfg(windows)]
impl io::Write for SharedPtyWriter {
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

enum ChildHandleImpl {
    Tokio(Option<tokio::process::Child>),
    Pty(Box<dyn PtyChild + Send + Sync>),
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
enum GracefulInterruptTarget {
    DirectChild,
    ProcessGroup,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct ShutdownSemantics {
    // Who should receive the first graceful interrupt.
    graceful_interrupt_target: GracefulInterruptTarget,
    // Whether we should keep waiting on the process group after the direct child exits.
    wait_for_process_group_after_child_exit: bool,
}

#[cfg(unix)]
impl ShutdownSemantics {
    fn process_group() -> Self {
        Self {
            graceful_interrupt_target: GracefulInterruptTarget::ProcessGroup,
            wait_for_process_group_after_child_exit: true,
        }
    }

    fn direct_child() -> Self {
        Self {
            graceful_interrupt_target: GracefulInterruptTarget::DirectChild,
            wait_for_process_group_after_child_exit: true,
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
pub(super) struct TargetIdentity {
    pub(super) process_group_id: libc::pid_t,
    pub(super) session_id: libc::pid_t,
}

#[cfg(unix)]
fn target_identity(target_pid: libc::pid_t) -> io::Result<TargetIdentity> {
    let process_group_id = unsafe { libc::getpgid(target_pid) };
    if process_group_id == -1 {
        return Err(io::Error::last_os_error());
    }

    let session_id = unsafe { libc::getsid(target_pid) };
    if session_id == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(TargetIdentity {
        process_group_id,
        session_id,
    })
}

#[cfg(unix)]
pub(super) fn process_group_matches_identity(
    target_pid: libc::pid_t,
    identity: TargetIdentity,
) -> bool {
    let process_group_id = unsafe { libc::getpgid(target_pid) };
    if process_group_id != -1 {
        if process_group_id != identity.process_group_id {
            return false;
        }

        let session_id = unsafe { libc::getsid(target_pid) };
        return session_id != -1 && session_id == identity.session_id;
    }

    let result = unsafe { libc::kill(-identity.process_group_id, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
pub(super) fn signal_process_group(process_group_id: libc::pid_t, signal: libc::c_int) {
    let _ = unsafe { libc::kill(-process_group_id, signal) };
}

#[cfg(windows)]
fn run_child_console_helper(pid: u32, command: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };

    std::process::Command::new(exe)
        .arg("__internal_windows_ctrl_c")
        .arg(command)
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn send_ctrl_c_to_child_console(pid: u32) -> bool {
    run_child_console_helper(pid, "ctrl_c")
}

#[cfg(unix)]
fn capture_target_identity(pid: Option<u32>) -> Option<TargetIdentity> {
    pid.and_then(|pid| match target_identity(pid as libc::pid_t) {
        Ok(identity) => Some(identity),
        Err(err) => {
            debug!("failed to capture target identity for process {pid}: {err}");
            None
        }
    })
}

impl ChildHandle {
    #[tracing::instrument(skip(command))]
    pub(super) fn spawn_normal(command: Command) -> io::Result<SpawnResult> {
        #[cfg(windows)]
        let command_for_fallback = command.clone();

        let mut command = std::process::Command::from(command);

        // Create a new process group so we can send signals (e.g. SIGINT) to
        // the child and all of its descendants via kill(-pgid, sig).
        #[cfg(unix)]
        use std::os::unix::process::CommandExt as _;
        #[cfg(unix)]
        command.process_group(0);

        #[cfg(windows)]
        let job = match crate::job_object::JobObject::new() {
            Ok(job) => Some(job),
            Err(err) => {
                debug!("failed to create Windows JobObject: {err}");
                None
            }
        };

        #[cfg(windows)]
        let wrapper_ctrl_c = std::env::var_os("__TURBO_WINDOWS_CTRL_C_FD").is_some();

        #[cfg(windows)]
        use std::os::windows::process::CommandExt as _;

        #[cfg(windows)]
        if wrapper_ctrl_c {
            command.show_window(windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE as u16);
        }

        #[cfg(windows)]
        if job.is_some() {
            let mut creation_flags = windows_sys::Win32::System::Threading::CREATE_SUSPENDED
                | windows_sys::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB;
            if wrapper_ctrl_c {
                creation_flags |= windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE
                    | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            }
            command.creation_flags(creation_flags);
        } else if wrapper_ctrl_c {
            command.creation_flags(
                windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE
                    | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
            );
        }

        let mut command = TokioCommand::from(command);

        #[cfg(not(windows))]
        let mut child = command.spawn()?;

        #[cfg(windows)]
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) if job.is_some() => {
                debug!("failed to spawn child with job breakaway: {err}");
                let mut fallback_command = TokioCommand::from(command_for_fallback);
                let mut creation_flags = windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
                if wrapper_ctrl_c {
                    creation_flags |= windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE
                        | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
                }
                if wrapper_ctrl_c {
                    fallback_command
                        .as_std_mut()
                        .show_window(windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE as u16);
                }
                fallback_command.creation_flags(creation_flags);
                fallback_command.spawn()?
            }
            Err(err) => return Err(err),
        };
        let pid = child.id();

        #[cfg(unix)]
        let target_identity = capture_target_identity(pid);

        #[cfg(windows)]
        let job = job.and_then(|job| match child.raw_handle() {
            Some(handle) => match job.assign_suspended_process(handle) {
                Ok(true) => Some(job),
                Ok(false) => None,
                Err(err) => {
                    debug!("failed to resume suspended process after job assignment: {err}");
                    child.start_kill().ok();
                    None
                }
            },
            None => {
                debug!("failed to get child process handle for job assignment");
                child.start_kill().ok();
                None
            }
        });

        let stdin = child.stdin.take().map(ChildInput::Std);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("child process must be started with piped stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("child process must be started with piped stderr"))?;

        Ok(SpawnResult {
            handle: Self {
                pid,
                imp: ChildHandleImpl::Tokio(Some(child)),
                #[cfg(unix)]
                shutdown_semantics: ShutdownSemantics::process_group(),
                #[cfg(unix)]
                target_identity,
                #[cfg(unix)]
                pty_controller_fd: None,
                #[cfg(windows)]
                _job: job,
                #[cfg(windows)]
                pty_input: None,
            },
            io: ChildIO {
                stdin,
                output: Some(ChildOutput::Std { stdout, stderr }),
            },
            controller: None,
        })
    }

    #[tracing::instrument(skip(command))]
    pub(super) fn spawn_pty(command: Command, size: PtySize) -> io::Result<SpawnResult> {
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

        #[cfg(unix)]
        let target_identity = capture_target_identity(pid);

        #[cfg(windows)]
        let job = pid.and_then(|pid| {
            crate::job_object::JobObject::new()
                .and_then(|job| job.assign_pid(pid).map(|_| job))
                .map_err(|e| debug!("failed to set up job object for PTY process {pid}: {e}"))
                .ok()
        });

        #[cfg(unix)]
        let pty_controller_fd = controller.as_raw_fd();

        let stdin = controller.take_writer().ok();
        let output = controller.try_clone_reader().ok().map(ChildOutput::Pty);

        #[cfg(windows)]
        let mut stdin = stdin;

        // portable-pty 0.9.0 creates ConPTY with PSEUDOCONSOLE_INHERIT_CURSOR,
        // which sends a Device Status Report (DSR) cursor position request
        // (\x1b[6n) on the output pipe during initialization. ConPTY blocks
        // until the host responds with a Cursor Position Report on stdin.
        // Without this response the PTY hangs indefinitely.
        // See https://github.com/vercel/turborepo/issues/11808
        #[cfg(windows)]
        if let Some(ref mut writer) = stdin {
            // Respond with cursor at position (1,1). The actual position
            // doesn't matter — ConPTY just needs a valid CPR to unblock.
            if let Err(e) = writer.write_all(b"\x1b[1;1R") {
                debug!("failed to write ConPTY cursor position response: {e}");
            }
        }

        // Keep a shared handle to the ConPTY input pipe so graceful shutdown
        // can deliver a Ctrl-C keystroke even while stdin forwarding (or an
        // exec stdin guard) owns the writer. This also keeps the input pipe
        // open for the child's lifetime: ConPTY terminates children when
        // their input pipe closes.
        #[cfg(windows)]
        let (stdin, pty_input) = match stdin {
            Some(writer) => {
                let shared: SharedPtyInput = std::sync::Arc::new(std::sync::Mutex::new(writer));
                let writer: Box<dyn io::Write + Send> = Box::new(SharedPtyWriter(shared.clone()));
                (Some(writer), Some(shared))
            }
            None => (None, None),
        };

        let stdin = if keep_stdin_open {
            stdin.map(ChildInput::Pty)
        } else {
            None
        };

        Ok(SpawnResult {
            handle: Self {
                pid,
                imp: ChildHandleImpl::Pty(child),
                #[cfg(unix)]
                shutdown_semantics: ShutdownSemantics::direct_child(),
                #[cfg(unix)]
                target_identity,
                #[cfg(unix)]
                pty_controller_fd,
                #[cfg(windows)]
                _job: job,
                #[cfg(windows)]
                pty_input,
            },
            io: ChildIO { stdin, output },
            controller: Some(controller),
        })
    }

    pub(super) fn pid(&self) -> Option<u32> {
        self.pid
    }

    #[cfg(unix)]
    fn process_group_id(&self) -> Option<libc::pid_t> {
        self.target_identity
            .map(|identity| identity.process_group_id)
            .or(self.pid.map(|pid| pid as libc::pid_t))
    }

    #[cfg(unix)]
    fn graceful_process_group_id(&self) -> Option<libc::pid_t> {
        self.pty_controller_fd
            .and_then(|fd| match unsafe { libc::tcgetpgrp(fd) } {
                process_group_id if process_group_id > 0 => Some(process_group_id),
                _ => None,
            })
            .or_else(|| self.process_group_id())
    }

    #[cfg(unix)]
    fn send_signal_to_process_group(&self, pid: libc::pid_t, signal: libc::c_int) {
        let Some(process_group_id) = self.graceful_process_group_id() else {
            debug!("missing process group id for child {pid}");
            return;
        };

        debug!("sending signal {signal} to process group -{process_group_id}");
        signal_process_group(process_group_id, signal);
    }

    #[cfg(unix)]
    pub(super) fn send_graceful_interrupt(&self, pid: libc::pid_t) {
        match self.shutdown_semantics.graceful_interrupt_target {
            GracefulInterruptTarget::DirectChild => {
                debug!("sending SIGINT to child {pid}");
                if unsafe { libc::kill(pid, libc::SIGINT) } == -1 {
                    debug!("failed to send SIGINT to {pid}");
                }
            }
            GracefulInterruptTarget::ProcessGroup => {
                self.send_signal_to_process_group(pid, libc::SIGINT);
            }
        }
    }

    #[cfg(unix)]
    fn should_wait_for_process_group_after_child_exit(&self) -> bool {
        self.shutdown_semantics
            .wait_for_process_group_after_child_exit
    }

    #[cfg(unix)]
    fn has_running_process_group(&self, pid: libc::pid_t) -> bool {
        if let Some(identity) = self.target_identity {
            return process_group_matches_identity(pid, identity);
        }

        let process_group_id = self.process_group_id().unwrap_or(pid);

        let result = unsafe { libc::kill(-process_group_id, 0) };
        result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(unix)]
    fn kill_process_group(&self, pid: libc::pid_t) {
        let process_group_id = self.process_group_id().unwrap_or(pid);

        debug!("killing process group {}", process_group_id);
        signal_process_group(process_group_id, libc::SIGKILL);
    }

    #[cfg(unix)]
    pub(super) async fn wait_for_process_group_exit(
        &mut self,
        pid: libc::pid_t,
        deadline: Option<tokio::time::Instant>,
        command_rx: &mut mpsc::Receiver<ChildCommand>,
        command_rx_open: &mut bool,
    ) -> ChildExit {
        if !self.should_wait_for_process_group_after_child_exit() {
            return ChildExit::Interrupted;
        }

        while self.has_running_process_group(pid) {
            match deadline {
                Some(deadline) => {
                    tokio::select! {
                        command = command_rx.recv(), if *command_rx_open => {
                            match command {
                                Some(ChildCommand::Kill) => {
                                    debug!("graceful shutdown interrupted, killing process group");
                                    self.kill_process_group(pid);
                                    return ChildExit::Killed;
                                }
                                Some(ChildCommand::Shutdown(_)) => {}
                                None => *command_rx_open = false,
                            }
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            debug!("graceful shutdown timed out, killing process group");
                            self.kill_process_group(pid);
                            return ChildExit::Killed;
                        }
                        _ = tokio::time::sleep(PROCESS_TREE_DRAIN_POLL_INTERVAL) => {}
                    }
                }
                None => {
                    tokio::select! {
                        command = command_rx.recv(), if *command_rx_open => {
                            match command {
                                Some(ChildCommand::Kill) => {
                                    debug!("graceful shutdown interrupted, killing process group");
                                    self.kill_process_group(pid);
                                    return ChildExit::Killed;
                                }
                                Some(ChildCommand::Shutdown(_)) => {}
                                None => *command_rx_open = false,
                            }
                        }
                        _ = tokio::time::sleep(PROCESS_TREE_DRAIN_POLL_INTERVAL) => {}
                    }
                }
            }
        }

        ChildExit::Interrupted
    }

    /// Attempt to deliver a Ctrl-C to a ConPTY child by writing ETX to the
    /// pseudoconsole's input pipe. ConPTY translates the keystroke into a
    /// CTRL_C_EVENT for the attached process tree, mirroring what happens
    /// when a user types Ctrl-C in a real console. Returns whether the
    /// keystroke was written.
    ///
    /// When the npm package wrapper captures Ctrl-C in raw mode, Windows does
    /// not generate the console event. In that case, synthesize it here so
    /// non-ConPTY children still receive the same event as direct `turbo` use.
    #[cfg(windows)]
    pub(super) fn send_graceful_interrupt(&self) -> bool {
        let Some(pty_input) = &self.pty_input else {
            if std::env::var_os("__TURBO_WINDOWS_CTRL_C_FD").is_some()
                && let Some(pid) = self.pid
            {
                let sent = send_ctrl_c_to_child_console(pid);
                if sent {
                    debug!("generated console Ctrl-C for child console {pid}");
                } else {
                    debug!("failed to generate console Ctrl-C for child console {pid}");
                }
                return sent;
            }
            return false;
        };

        let mut writer = pty_input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match writer.write_all(b"\x03").and_then(|()| writer.flush()) {
            Ok(()) => {
                debug!("wrote Ctrl-C to ConPTY input for child {:?}", self.pid);
                true
            }
            Err(err) => {
                debug!("failed to write Ctrl-C to ConPTY input: {err}");
                false
            }
        }
    }

    #[cfg(windows)]
    fn has_active_windows_job(&self) -> bool {
        self._job
            .as_ref()
            .is_some_and(|job| match job.active_processes() {
                Ok(active_processes) => active_processes > 0,
                Err(err) => {
                    debug!("failed to query job object: {err}");
                    false
                }
            })
    }

    #[cfg(windows)]
    fn has_running_windows_descendants(&self) -> bool {
        match self.pid {
            Some(pid) => match crate::job_object::has_descendant_processes(pid) {
                Ok(has_descendants) => has_descendants,
                Err(err) => {
                    debug!("failed to query descendant processes: {err}");
                    false
                }
            },
            None => false,
        }
    }

    #[cfg(windows)]
    fn terminate_windows_process_tree(&self) {
        if let Some(job) = &self._job
            && let Err(err) = job.terminate()
        {
            debug!("failed to terminate job object: {err}");
        }

        if let Some(pid) = self.pid
            && let Err(err) = crate::job_object::terminate_descendant_processes(pid)
        {
            debug!("failed to terminate descendant process tree: {err}");
        }
    }

    #[cfg(windows)]
    pub(super) async fn wait_for_job_exit(
        &mut self,
        deadline: Option<tokio::time::Instant>,
        command_rx: &mut mpsc::Receiver<ChildCommand>,
        command_rx_open: &mut bool,
    ) -> ChildExit {
        // PID snapshots are only a fallback for runners where Job Object
        // assignment fails. After the parent exits they can match unrelated
        // reused PIDs, so never let that path wait forever.
        let descendant_drain_deadline = self
            ._job
            .is_none()
            .then(|| tokio::time::Instant::now() + WINDOWS_DESCENDANT_DRAIN_TIMEOUT);

        loop {
            let has_active_job = self.has_active_windows_job();
            let has_descendants = self._job.is_none() && self.has_running_windows_descendants();

            if !has_active_job && !has_descendants {
                break;
            }

            tokio::select! {
                command = command_rx.recv(), if *command_rx_open => {
                    match command {
                        Some(ChildCommand::Kill) => {
                            debug!("process tree drain interrupted, terminating job object");
                            self.terminate_windows_process_tree();
                            return ChildExit::Killed;
                        }
                        Some(ChildCommand::Shutdown(_)) => {}
                        None => *command_rx_open = false,
                    }
                }
                _ = async {
                    if let Some(deadline) = deadline {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if deadline.is_some() => {
                    debug!("graceful shutdown timed out, terminating Windows process tree");
                    self.terminate_windows_process_tree();
                    return ChildExit::Killed;
                }
                _ = async {
                    if let Some(deadline) = descendant_drain_deadline {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if has_descendants && descendant_drain_deadline.is_some() => {
                    debug!("timed out waiting for Windows descendant process tree after direct child exit");
                    break;
                }
                _ = tokio::time::sleep(PROCESS_TREE_DRAIN_POLL_INTERVAL) => {}
            }
        }

        ChildExit::Interrupted
    }

    /// Perform a `wait` syscall on the child until it exits
    pub(super) async fn wait(&mut self) -> io::Result<Option<i32>> {
        match &mut self.imp {
            ChildHandleImpl::Tokio(child) => {
                let result = match child {
                    Some(child) => child.wait().await.map(|status| status.code()),
                    None => Ok(None),
                };

                #[cfg(windows)]
                if result.is_ok() {
                    // Drop the process handle before querying the Job Object so
                    // the exited direct child is not counted during tree drain.
                    child.take();
                }

                result
            }
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

    pub(super) async fn kill(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        if let Some(process_group_id) = self.process_group_id() {
            signal_process_group(process_group_id, libc::SIGKILL);
        }

        match &mut self.imp {
            ChildHandleImpl::Tokio(Some(child)) => child.kill().await,
            ChildHandleImpl::Tokio(None) => Ok(()),
            ChildHandleImpl::Pty(child) => {
                let mut killer = child.clone_killer();
                tokio::task::spawn_blocking(move || killer.kill())
                    .await
                    .map_err(|err| io::Error::other(format!("pty kill task failed: {err}")))?
            }
        }
    }
}

pub(super) struct SpawnResult {
    pub(super) handle: ChildHandle,
    pub(super) io: ChildIO,
    pub(super) controller: Option<Box<dyn PtyController + Send>>,
}
