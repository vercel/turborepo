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
const POST_EXIT_OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
#[cfg(unix)]
const PROCESS_GROUP_DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(unix)]
const PARENT_DEATH_ESCALATION_DELAY: Duration = Duration::from_secs(2);

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
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

#[derive(Debug, Clone, Copy)]
pub enum ShutdownStyle {
    /// On Windows this immediately kills the process. On Unix it sends SIGINT
    /// to the process group.
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

struct ChildHandle {
    pid: Option<u32>,
    imp: ChildHandleImpl,
    #[cfg(unix)]
    shutdown_semantics: ShutdownSemantics,
    #[cfg(unix)]
    target_identity: Option<TargetIdentity>,
    #[cfg(unix)]
    parent_death_guard: Option<ParentDeathGuard>,
    #[cfg(windows)]
    _job: Option<super::job_object::JobObject>,
}

enum ChildHandleImpl {
    Tokio(tokio::process::Child),
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
            wait_for_process_group_after_child_exit: false,
        }
    }

    fn direct_child_then_wait_for_process_group() -> Self {
        Self {
            graceful_interrupt_target: GracefulInterruptTarget::DirectChild,
            wait_for_process_group_after_child_exit: true,
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct TargetIdentity {
    process_group_id: libc::pid_t,
    session_id: libc::pid_t,
}

#[cfg(unix)]
#[derive(Debug)]
struct ParentDeathGuard {
    write_fd: Option<OwnedFd>,
    watchdog_pid: Option<libc::pid_t>,
}

#[cfg(unix)]
impl ParentDeathGuard {
    fn spawn_for_pid(target_pid: libc::pid_t) -> io::Result<Self> {
        let (read_fd, write_fd) = parent_death_pipe()?;
        let watchdog_pid = spawn_parent_death_watchdog(target_pid, read_fd)?;

        Ok(Self {
            write_fd: Some(write_fd),
            watchdog_pid: Some(watchdog_pid),
        })
    }

    fn disarm(&mut self) {
        let Some(write_fd) = self.write_fd.take() else {
            return;
        };

        let _ = unsafe { libc::write(write_fd.as_raw_fd(), [1_u8].as_ptr().cast(), 1) };
        drop(write_fd);
        self.reap_watchdog();
    }

    fn reap_watchdog(&mut self) {
        let Some(watchdog_pid) = self.watchdog_pid.take() else {
            return;
        };

        let mut status = 0;
        loop {
            let wait_result = unsafe { libc::waitpid(watchdog_pid, &mut status, 0) };
            if wait_result != -1 {
                break;
            }

            if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
    }
}

#[cfg(unix)]
impl Drop for ParentDeathGuard {
    fn drop(&mut self) {
        self.write_fd.take();
        self.reap_watchdog();
    }
}

#[cfg(unix)]
fn parent_death_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }

    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    set_cloexec(read_fd.as_raw_fd())?;
    set_cloexec(write_fd.as_raw_fd())?;
    Ok((read_fd, write_fd))
}

#[cfg(unix)]
fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
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
fn process_group_matches_identity(target_pid: libc::pid_t, identity: TargetIdentity) -> bool {
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
fn close_fd(fd: RawFd) {
    if fd >= 0 {
        let _ = unsafe { libc::close(fd) };
    }
}

#[cfg(unix)]
fn close_inherited_fds(pipe_read_fd: RawFd, target_exit_fd: Option<RawFd>) {
    let max_fd = unsafe { libc::getdtablesize() };
    let max_fd = if max_fd > 0 { max_fd } else { 1024 };

    for fd in 0..max_fd {
        let fd = fd as RawFd;
        if fd == pipe_read_fd || Some(fd) == target_exit_fd {
            continue;
        }
        close_fd(fd);
    }
}

#[cfg(unix)]
fn signal_process_group(process_group_id: libc::pid_t, signal: libc::c_int) {
    let _ = unsafe { libc::kill(-process_group_id, signal) };
}

#[cfg(unix)]
fn sleep_unchecked(duration: Duration) {
    let mut remaining = libc::timespec {
        tv_sec: duration.as_secs() as libc::time_t,
        tv_nsec: duration.subsec_nanos() as libc::c_long,
    };

    loop {
        let mut next = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::nanosleep(&remaining, &mut next) };
        if result == 0 {
            break;
        }

        if io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            break;
        }
        remaining = next;
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentDeathWatchdogEvent {
    Disarmed,
    ParentDied,
    Error,
}

#[cfg(unix)]
fn wait_for_parent_death_or_disarm(pipe_read_fd: RawFd) -> ParentDeathWatchdogEvent {
    loop {
        let mut fd = libc::pollfd {
            fd: pipe_read_fd,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };

        let poll_result = unsafe { libc::poll(&mut fd, 1, -1) };
        if poll_result == -1 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return ParentDeathWatchdogEvent::Error;
        }

        if fd.revents == 0 {
            continue;
        }

        let mut byte = 0_u8;
        let read_result = unsafe { libc::read(pipe_read_fd, (&mut byte as *mut u8).cast(), 1) };
        if read_result > 0 {
            return ParentDeathWatchdogEvent::Disarmed;
        }
        if read_result == 0 {
            return ParentDeathWatchdogEvent::ParentDied;
        }
        if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return ParentDeathWatchdogEvent::Error;
    }
}

#[cfg(unix)]
fn run_parent_death_watchdog(
    pipe_read_fd: RawFd,
    target_pid: libc::pid_t,
    identity: TargetIdentity,
) -> ! {
    // The watchdog must not keep unrelated task pipes open.
    close_inherited_fds(pipe_read_fd, None);
    let event = wait_for_parent_death_or_disarm(pipe_read_fd);
    close_fd(pipe_read_fd);

    if event == ParentDeathWatchdogEvent::ParentDied
        && process_group_matches_identity(target_pid, identity)
    {
        signal_process_group(identity.process_group_id, libc::SIGTERM);
        sleep_unchecked(PARENT_DEATH_ESCALATION_DELAY);
        if process_group_matches_identity(target_pid, identity) {
            signal_process_group(identity.process_group_id, libc::SIGKILL);
        }
    }

    unsafe { libc::_exit(0) }
}

#[cfg(unix)]
fn spawn_parent_death_watchdog(
    target_pid: libc::pid_t,
    read_fd: OwnedFd,
) -> io::Result<libc::pid_t> {
    let identity = target_identity(target_pid)?;
    let read_fd = read_fd.into_raw_fd();

    match unsafe { libc::fork() } {
        -1 => {
            let err = io::Error::last_os_error();
            close_fd(read_fd);
            Err(err)
        }
        0 => run_parent_death_watchdog(read_fd, target_pid, identity),
        watchdog_pid => {
            close_fd(read_fd);
            Ok(watchdog_pid)
        }
    }
}

#[cfg(unix)]
fn setup_parent_death_guard(pid: Option<u32>) -> Option<ParentDeathGuard> {
    pid.and_then(
        |pid| match ParentDeathGuard::spawn_for_pid(pid as libc::pid_t) {
            Ok(parent_death_guard) => Some(parent_death_guard),
            Err(err) => {
                debug!("failed to set up parent-death guard for process {pid}: {err}");
                None
            }
        },
    )
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
    pub fn spawn_normal(command: Command) -> io::Result<SpawnResult> {
        let mut command = TokioCommand::from(command);

        // Create a new process group so we can send signals (e.g. SIGINT) to
        // the child and all of its descendants via kill(-pgid, sig).
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn()?;
        let pid = child.id();

        #[cfg(unix)]
        let target_identity = capture_target_identity(pid);

        #[cfg(unix)]
        let parent_death_guard = setup_parent_death_guard(pid);

        #[cfg(windows)]
        let job = pid.and_then(|pid| {
            super::job_object::JobObject::new()
                .and_then(|job| job.assign_pid(pid).map(|_| job))
                .map_err(|e| debug!("failed to set up job object for process {pid}: {e}"))
                .ok()
        });

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
                #[cfg(unix)]
                shutdown_semantics: ShutdownSemantics::process_group(),
                #[cfg(unix)]
                target_identity,
                #[cfg(unix)]
                parent_death_guard,
                #[cfg(windows)]
                _job: job,
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

        #[cfg(unix)]
        let target_identity = capture_target_identity(pid);

        #[cfg(unix)]
        let parent_death_guard = setup_parent_death_guard(pid);

        #[cfg(windows)]
        let job = pid.and_then(|pid| {
            super::job_object::JobObject::new()
                .and_then(|job| job.assign_pid(pid).map(|_| job))
                .map_err(|e| debug!("failed to set up job object for PTY process {pid}: {e}"))
                .ok()
        });

        let mut stdin = controller.take_writer().ok();
        let output = controller.try_clone_reader().ok().map(ChildOutput::Pty);

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

        // If we don't want to keep stdin open we take it here and it is immediately
        // dropped resulting in a EOF being sent to the child process.
        if !keep_stdin_open {
            stdin.take();
        }

        Ok(SpawnResult {
            handle: Self {
                pid,
                imp: ChildHandleImpl::Pty(child),
                #[cfg(unix)]
                shutdown_semantics: ShutdownSemantics::direct_child_then_wait_for_process_group(),
                #[cfg(unix)]
                target_identity,
                #[cfg(unix)]
                parent_death_guard,
                #[cfg(windows)]
                _job: job,
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

    #[cfg(unix)]
    fn process_group_id(&self) -> Option<libc::pid_t> {
        self.target_identity
            .map(|identity| identity.process_group_id)
            .or(self.pid.map(|pid| pid as libc::pid_t))
    }

    #[cfg(unix)]
    fn send_graceful_interrupt(&self, pid: libc::pid_t) {
        let target = match self.shutdown_semantics.graceful_interrupt_target {
            GracefulInterruptTarget::DirectChild => {
                debug!("sending SIGINT to child {}", pid);
                pid
            }
            GracefulInterruptTarget::ProcessGroup => {
                let Some(process_group_id) = self.process_group_id() else {
                    debug!("missing process group id for child {}", pid);
                    return;
                };
                debug!("sending SIGINT to process group -{}", process_group_id);
                -process_group_id
            }
        };

        if unsafe { libc::kill(target, libc::SIGINT) } == -1 {
            debug!("failed to send SIGINT to {target}");
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
    async fn wait_for_process_group_exit(
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
                        _ = tokio::time::sleep(PROCESS_GROUP_DRAIN_POLL_INTERVAL) => {}
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
                        _ = tokio::time::sleep(PROCESS_GROUP_DRAIN_POLL_INTERVAL) => {}
                    }
                }
            }
        }

        ChildExit::Interrupted
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
        #[cfg(unix)]
        if let Some(process_group_id) = self.process_group_id() {
            signal_process_group(process_group_id, libc::SIGKILL);
        }

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

    #[cfg(unix)]
    fn disarm_parent_death_guard(&mut self) {
        if let Some(parent_death_guard) = &mut self.parent_death_guard {
            parent_death_guard.disarm();
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
    async fn process(
        &self,
        child: &mut ChildHandle,
        command_rx: &mut mpsc::Receiver<ChildCommand>,
    ) -> ChildExit {
        match self {
            // Windows doesn't give the ability to send a signal to a process so we
            // can't make use of the graceful shutdown timeout.
            #[allow(unused)]
            ShutdownStyle::Graceful(timeout) => {
                // try ro run the command for the given timeout
                #[cfg(unix)]
                {
                    let Some(pid) = child.pid() else {
                        return ChildExit::Interrupted;
                    };

                    child.send_graceful_interrupt(pid as libc::pid_t);
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
                                pid as libc::pid_t,
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
    shutdown_style: ShutdownStyle,
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

    pub async fn shutdown(
        &self,
        shutdown_style: ShutdownStyle,
    ) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Shutdown(shutdown_style)).await
    }

    pub async fn kill(&self) -> Result<(), mpsc::error::SendError<ChildCommand>> {
        self.0.send(ChildCommand::Kill).await
    }
}

pub enum ChildCommand {
    Shutdown(ShutdownStyle),
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
                    manager.handle_child_command(command, &mut command_rx, &mut child, controller).await;
                }
                status = child.wait() => {
                    drop(controller);
                    manager.handle_child_exit(status, &mut child).await;
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
            shutdown_style,
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
        self.shutdown(self.shutdown_style).await
    }

    pub async fn shutdown(&mut self, shutdown_style: ShutdownStyle) -> Option<ChildExit> {
        // if this fails, it's because the channel is dropped (toctou)
        // we can just ignore it
        self.command_channel.shutdown(shutdown_style).await.ok();
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

    pub(crate) fn has_exited(&self) -> bool {
        self.exit_channel.borrow().is_some()
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

        Ok(exit_status.or(self.wait().await))
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
        command_rx: &mut mpsc::Receiver<ChildCommand>,
        child: &mut ChildHandle,
        controller: Option<Box<dyn PtyController + Send>>,
    ) {
        let exit = match command.unwrap_or(ChildCommand::Shutdown(self.shutdown_style)) {
            ChildCommand::Shutdown(shutdown_style) => {
                debug!("stopping child process");
                shutdown_style.process(child, command_rx).await
            }
            ChildCommand::Kill => {
                debug!("killing child process");
                ShutdownStyle::Kill.process(child, command_rx).await
            }
        };
        #[cfg(unix)]
        child.disarm_parent_death_guard();
        // ignore the send error, failure means the channel is dropped
        trace!("sending child exit after shutdown");
        self.exit_tx.send(Some(exit)).ok();
        drop(controller);
    }

    async fn handle_child_exit(&self, status: io::Result<Option<i32>>, child: &mut ChildHandle) {
        // If a shutdown was initiated we defer to the exit returned by
        // `ShutdownStyle::process` as that will have information if the child
        // responded to a SIGINT or a SIGKILL. The `wait` response this function
        // gets in that scenario would make it appear that the child was killed by an
        // external process.
        if self.shutdown_initiated {
            return;
        }

        debug!("child process exited normally");
        #[cfg(unix)]
        child.disarm_parent_death_guard();
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
    use std::{
        assert_matches, io,
        process::Stdio,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use futures::{StreamExt, stream::FuturesUnordered};
    use test_case::test_case;
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        process::Command as TokioCommand,
        sync::oneshot,
    };
    use tracing_test::traced_test;
    use turbopath::AbsoluteSystemPathBuf;

    #[cfg(unix)]
    use super::ParentDeathGuard;
    use super::{Child, ChildInput, ChildOutput, Command};
    use crate::{
        PtySize,
        child::{ChildExit, ShutdownStyle},
    };

    const STARTUP_DELAY: Duration = Duration::from_millis(500);
    // We skip testing PTY usage on Windows
    const TEST_PTY: bool = !cfg!(windows);

    struct ObservedOutput {
        buffer: Arc<Mutex<Vec<u8>>>,
        ready_tx: Option<oneshot::Sender<()>>,
    }

    impl ObservedOutput {
        fn new() -> (Self, Arc<Mutex<Vec<u8>>>, oneshot::Receiver<()>) {
            let buffer = Arc::new(Mutex::new(Vec::new()));
            let (ready_tx, ready_rx) = oneshot::channel();
            (
                Self {
                    buffer: buffer.clone(),
                    ready_tx: Some(ready_tx),
                },
                buffer,
                ready_rx,
            )
        }
    }

    impl io::Write for ObservedOutput {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let saw_ready = {
                let mut buffer = self.buffer.lock().unwrap();
                buffer.extend_from_slice(buf);
                String::from_utf8_lossy(&buffer).contains("ready")
            };

            if saw_ready && let Some(ready_tx) = self.ready_tx.take() {
                ready_tx.send(()).ok();
            }

            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    const EOT: char = '\u{4}';

    fn find_script_dir() -> AbsoluteSystemPathBuf {
        let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
        let mut root = cwd;
        while !root.join_component(".git").exists() {
            root = root.parent().unwrap().to_owned();
        }
        root.join_components(&["crates", "turborepo-process", "test", "scripts"])
    }

    #[cfg(unix)]
    async fn spawn_parent_death_target() -> (tokio::process::Child, libc::pid_t, libc::pid_t) {
        let script = find_script_dir().join_component("spawn_child_sleep.js");
        let mut command = TokioCommand::new("node");
        command
            .arg(script.as_std_path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .process_group(0);

        let mut child = command.spawn().unwrap();
        let child_pid = child.id().expect("child should have a pid") as libc::pid_t;
        let stdout = child.stdout.take().expect("child should have stdout");
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(2), stdout.read_line(&mut line))
            .await
            .expect("timed out waiting for child pid")
            .expect("failed to read child pid from stdout");

        let grandchild_pid = line
            .trim()
            .strip_prefix("CHILD_PID=")
            .expect("child pid output should be prefixed")
            .parse::<libc::pid_t>()
            .expect("child pid should parse");

        (child, child_pid, grandchild_pid)
    }

    #[cfg(unix)]
    async fn spawn_term_ignoring_parent_death_target()
    -> (tokio::process::Child, libc::pid_t, libc::pid_t) {
        let mut command = TokioCommand::new("sh");
        command
            .args([
                "-c",
                "trap '' TERM; sh -c \"trap '' TERM; while true; do sleep 0.2; done\" & \
                 CHILD_PID=$!; echo CHILD_PID=$CHILD_PID; while true; do sleep 0.2; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .process_group(0);

        let mut child = command.spawn().unwrap();
        let child_pid = child.id().expect("child should have a pid") as libc::pid_t;
        let stdout = child.stdout.take().expect("child should have stdout");
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(2), stdout.read_line(&mut line))
            .await
            .expect("timed out waiting for child pid")
            .expect("failed to read child pid from stdout");

        let grandchild_pid = line
            .trim()
            .strip_prefix("CHILD_PID=")
            .expect("child pid output should be prefixed")
            .parse::<libc::pid_t>()
            .expect("child pid should parse");

        (child, child_pid, grandchild_pid)
    }

    #[cfg(unix)]
    fn process_exists(pid: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
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

    /// Regression test for #7834: proves that a child process can block
    /// before producing any output if stdin is an open pipe instead of EOF.
    ///
    /// This models the v1.13 regression on Windows stream mode:
    /// `tsx watch` received an open piped stdin and never started executing.
    #[tokio::test]
    async fn test_std_open_stdin_blocks_startup_until_eof() {
        let script = find_script_dir().join_component("startup_after_stdin_eof.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let ChildOutput::Std { mut stdout, .. } = child.outputs().unwrap() else {
            panic!("expected stdio child");
        };

        let mut output = Vec::new();
        let result =
            tokio::time::timeout(Duration::from_secs(1), stdout.read_to_end(&mut output)).await;
        assert!(
            result.is_err(),
            "child should stay blocked while stdin is held open"
        );
        assert!(
            output.is_empty(),
            "child should not produce output before stdin reaches EOF"
        );

        // Closing the parent's stdin pipe should unblock the child immediately.
        drop(child.stdin_inner());

        tokio::time::timeout(Duration::from_secs(5), stdout.read_to_end(&mut output))
            .await
            .expect("child should finish reading after stdin is closed")
            .expect("failed to read child output");

        let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("child should exit after stdin is closed");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));

        let output = String::from_utf8(output).unwrap().replace("\r\n", "\n");
        assert_eq!(output, "stdin bytes=0\nstarted\n");
    }

    /// Regression test for #7834: verifies the pre-v1.13 behavior where tasks
    /// that do not need input start immediately when stdin is already at EOF.
    #[tokio::test]
    async fn test_std_null_stdin_allows_startup() {
        let script = find_script_dir().join_component("startup_after_stdin_eof.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

        let mut output = Vec::new();
        let exit = tokio::time::timeout(
            Duration::from_secs(5),
            child.wait_with_piped_outputs(&mut output),
        )
        .await
        .expect("child should not block when stdin is null")
        .expect("failed to wait for child output");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));

        let output = String::from_utf8(output).unwrap().replace("\r\n", "\n");
        assert_eq!(output, "stdin bytes=0\nstarted\n");
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
            ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
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
            ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
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
    async fn test_graceful_shutdown_drains_final_output(use_pty: bool) {
        let script = find_script_dir().join_component("graceful_sigint_output.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);

        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
            use_pty.then(PtySize::default),
        )
        .unwrap();

        let mut output_child = child.clone();
        let (mut observer, output, ready_rx) = ObservedOutput::new();
        let output_task = tokio::spawn(async move {
            output_child
                .wait_with_piped_outputs(&mut observer)
                .await
                .unwrap()
        });

        tokio::time::timeout(Duration::from_secs(2), ready_rx)
            .await
            .expect("timed out waiting for startup output")
            .expect("ready notification channel closed unexpectedly");
        child.set_closing();
        child.stop().await;
        let exit = output_task.await.unwrap();
        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();

        assert!(output.contains("ready"), "missing startup output: {output}");

        if cfg!(windows) {
            assert_matches!(exit, Some(ChildExit::Killed));
        } else {
            assert!(
                output.contains("received SIGINT"),
                "missing SIGINT receipt log: {output}"
            );
            assert!(
                output.contains("exiting after SIGINT"),
                "missing SIGINT exit log: {output}"
            );
            assert_matches!(exit, Some(ChildExit::Interrupted));
        }
    }

    // Regression test: a wrapper process (simulating npm/pnpm) forwards SIGINT
    // to its child. When turbo sends SIGINT to the process group, the child
    // gets it twice — once from the group signal, once from the wrapper.
    // For PTY children we now signal only the direct PID to avoid this.
    #[cfg(unix)]
    #[tokio::test]
    #[traced_test]
    async fn test_pty_child_receives_single_sigint() {
        let script = find_script_dir().join_component("wrapper_count_sigints.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(2000))),
            Some(PtySize::default()),
        )
        .unwrap();

        let mut output_child = child.clone();
        let (mut observer, output, ready_rx) = ObservedOutput::new();
        let output_task = tokio::spawn(async move {
            output_child
                .wait_with_piped_outputs(&mut observer)
                .await
                .unwrap()
        });

        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("timed out waiting for ready")
            .expect("ready channel closed");

        child.set_closing();
        child.stop().await;
        output_task.await.unwrap();

        let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("SIGINT_COUNT=1"),
            "expected exactly one SIGINT, got output: {output}"
        );
        assert!(
            !output.contains("SIGINT_COUNT=2"),
            "child received SIGINT twice: {output}"
        );
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
            ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
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
    #[test_case(TEST_PTY)]
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
            ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
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

        // On Unix, shell scripts may not respond to SIGINT and will timeout,
        // resulting in being killed rather than interrupted. For non-PTY
        // children, SIGINT goes to the process group but shells may still
        // continue their loop. For PTY children, SIGINT goes only to the
        // direct child to avoid double delivery through package managers,
        // which also means the shell may not exit gracefully.
        if cfg!(unix) {
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

    #[cfg(unix)]
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_graceful_shutdown_waits_for_force_kill(use_pty: bool) {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "trap '' INT; while true; do sleep 0.2; done"]);
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_secs(5))),
            use_pty.then(PtySize::default),
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let mut shutdown_child = child.clone();
        let shutdown =
            tokio::spawn(
                async move { shutdown_child.shutdown(ShutdownStyle::Graceful(None)).await },
            );

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !shutdown.is_finished(),
            "graceful shutdown should keep waiting until explicitly forced"
        );

        assert_eq!(child.kill().await, Some(ChildExit::Killed));
        assert_eq!(shutdown.await.unwrap(), Some(ChildExit::Killed));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_parent_death_guard_drop_kills_process_group() {
        let (mut child, child_pid, grandchild_pid) = spawn_parent_death_target().await;
        let guard = ParentDeathGuard::spawn_for_pid(child_pid).unwrap();
        drop(guard);

        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("timed out waiting for watchdog to kill child")
            .expect("failed waiting for child process");

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !process_exists(grandchild_pid),
            "watchdog should kill the entire child process group"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_parent_death_guard_disarm_keeps_process_group_alive() {
        let (mut child, child_pid, grandchild_pid) = spawn_parent_death_target().await;
        let mut guard = ParentDeathGuard::spawn_for_pid(child_pid).unwrap();
        guard.disarm();

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(
            process_exists(child_pid),
            "child should still be alive after disarm"
        );
        assert!(
            process_exists(grandchild_pid),
            "grandchild should still be alive after disarm"
        );

        unsafe {
            libc::kill(-child_pid, libc::SIGKILL);
        }
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("timed out waiting for cleanup after SIGKILL")
            .expect("failed waiting for child process");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_parent_death_guard_escalates_after_sigterm() {
        let (mut child, child_pid, grandchild_pid) =
            spawn_term_ignoring_parent_death_target().await;
        let guard = ParentDeathGuard::spawn_for_pid(child_pid).unwrap();
        drop(guard);

        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("timed out waiting for watchdog escalation")
            .expect("failed waiting for child process");

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !process_exists(grandchild_pid),
            "watchdog should escalate to SIGKILL for TERM-ignoring process trees"
        );
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

    // Regression tests for https://github.com/vercel/turborepo/issues/11808
    //
    // On Windows, portable-pty 0.9.0 added PSEUDOCONSOLE_INHERIT_CURSOR to
    // ConPTY creation, which requires the host to handle DSR (Device Status
    // Report) escape sequences. Turborepo doesn't, causing ConPTY to hang.
    //
    // Additionally, an unconditional `drop(stdin)` in the PTY path of
    // wait_with_piped_outputs would kill ConPTY children on Windows because
    // closing ConPTY stdin terminates the session.
    //
    // These tests verify the fixes: PTY children start, produce output, and
    // exit normally without hanging or being killed by stdin closure.

    /// Verifies that a PTY-spawned short-lived process produces output and
    /// exits cleanly via wait_with_piped_outputs. Uses a timeout to catch
    /// the ConPTY hang that occurred with PSEUDOCONSOLE_INHERIT_CURSOR.
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_pty_child_does_not_hang(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        let mut out = Vec::new();

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            child.wait_with_piped_outputs(&mut out),
        )
        .await;

        let exit = result
            .expect("PTY child hung — likely PSEUDOCONSOLE_INHERIT_CURSOR regression")
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
        assert_eq!(trimmed, "hello world");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    /// Simulates the persistent-task flow: stdin is taken by the caller
    /// (as the TUI does for interactive tasks) BEFORE wait_with_piped_outputs
    /// is called. The child should still produce output and exit normally
    /// without wait_with_piped_outputs interfering with stdin.
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_pty_stdin_taken_before_piped_outputs(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        // Take stdin before piping outputs, simulating TUI taking ownership.
        // For PTY children, this returns Some; for non-PTY, stdin() returns None
        // (Std variant is filtered out), but stdin_inner still removes it.
        let _stdin_guard = child.stdin();

        // Verify stdin_inner is now empty (already taken).
        assert!(
            child.stdin_inner().is_none(),
            "stdin should already be taken"
        );

        let mut out = Vec::new();

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            child.wait_with_piped_outputs(&mut out),
        )
        .await;

        let exit = result
            .expect("child hung — wait_with_piped_outputs likely interfered with taken stdin")
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
        assert_eq!(trimmed, "hello world");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    /// Verifies that a PTY-spawned process with open stdin that has NOT been
    /// taken by the caller still completes normally. This is the non-persistent
    /// task path where exec.rs does not take stdin before
    /// wait_with_piped_outputs.
    ///
    /// Before the fix, on Windows the unconditional stdin drop inside
    /// wait_with_piped_outputs would kill the ConPTY child immediately.
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_pty_untaken_stdin_does_not_kill_child(use_pty: bool) {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        // Do NOT take stdin — this simulates a non-persistent task where
        // exec.rs skips stdin handling on Windows (closing_stdin_ends_process).
        // wait_with_piped_outputs should still work without killing the child.
        let mut out = Vec::new();

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            child.wait_with_piped_outputs(&mut out),
        )
        .await;

        let exit = result
            .expect("child process hung or was killed by premature stdin closure")
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
        assert_eq!(trimmed, "hello world");
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    /// Regression test for #12393: proves that dropping stdin causes a
    /// persistent-style child (one that exits on stdin EOF) to terminate.
    ///
    /// This documents the mechanism behind the bug: when the task executor
    /// took stdin and passed it to `TaskOutput::set_stdin()` in stream mode,
    /// the stdin was dropped immediately, sending EOF to the child.
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_dropping_stdin_terminates_persistent_child(use_pty: bool) {
        let script = find_script_dir().join_component("persistent_server.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // Take stdin and immediately drop it — simulates the bug where
        // TaskOutput::stream().set_stdin() dropped stdin in stream mode.
        {
            let _dropped = child.stdin();
        }

        // The child should exit because it received EOF on stdin.
        let mut out = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            child.wait_with_piped_outputs(&mut out),
        )
        .await;

        let exit = result
            .expect("child should have exited after stdin was dropped")
            .unwrap();

        let output = String::from_utf8(out).unwrap();
        assert!(
            output.contains("server ready"),
            "expected 'server ready' in output, got: {output:?}"
        );
        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    /// Regression test for #12393: proves that holding stdin in a guard
    /// keeps a persistent-style child alive.
    ///
    /// This is the correct behavior after the fix: in stream mode, stdin
    /// is held by `_stdin_guard` instead of being passed to
    /// `TaskOutput::set_stdin()` which would drop it.
    ///
    /// PTY-only: `child.stdin()` returns `None` for non-PTY children
    /// (`ChildInput::Std` is filtered out), so the guard mechanism only
    /// applies to PTY-spawned processes — which is the production path
    /// for persistent tasks on Unix.
    #[tokio::test]
    async fn test_held_stdin_keeps_persistent_child_alive() {
        if !TEST_PTY {
            return;
        }
        let script = find_script_dir().join_component("persistent_server.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(cmd, ShutdownStyle::Kill, Some(PtySize::default())).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // Hold stdin in a guard — simulates the correct persistent task flow.
        let _stdin_guard = child.stdin();
        assert!(
            _stdin_guard.is_some(),
            "PTY child should return Some from stdin()"
        );

        // The child should NOT exit while we hold stdin. Give it a moment
        // and verify it's still alive by checking that wait times out.
        let result = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        assert!(
            result.is_err(),
            "child should still be alive while stdin is held"
        );

        // Now drop the guard — child should exit.
        drop(_stdin_guard);

        let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("child should exit after stdin guard is dropped");

        assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
    }

    /// Verifies that stopping a parent process also kills its child processes.
    ///
    /// On Unix this works via process groups (setpgid + kill(-pgid)).
    /// On Windows this works via Job Objects
    /// (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
    ///
    /// The test spawns a Node.js script that itself spawns a long-running child
    /// process, captures the grandchild's PID from stdout, stops the parent,
    /// and then checks that the grandchild is no longer alive.
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    #[traced_test]
    async fn test_process_tree_cleanup(use_pty: bool) {
        let script = find_script_dir().join_component("spawn_child_sleep.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
            use_pty.then(PtySize::default),
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        // Read stdout to get the grandchild PID
        let grandchild_pid = {
            let mut out = Vec::new();
            match child.outputs().unwrap() {
                ChildOutput::Std { mut stdout, .. } => {
                    let mut buf = vec![0u8; 256];
                    let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                        .await
                        .expect("timed out reading grandchild PID")
                        .expect("failed to read stdout");
                    out.extend_from_slice(&buf[..n]);
                }
                ChildOutput::Pty(mut reader) => {
                    let mut buf = vec![0u8; 256];
                    let n = reader.read(&mut buf).expect("failed to read pty output");
                    out.extend_from_slice(&buf[..n]);
                }
            };
            let output = String::from_utf8(out).unwrap();
            let pid_line = output
                .lines()
                .find(|line| line.contains("CHILD_PID="))
                .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
            pid_line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap()
        };

        // Verify grandchild is alive before we stop
        assert!(
            is_process_alive(grandchild_pid),
            "grandchild process {grandchild_pid} should be alive before stop"
        );

        // Stop the parent process
        child.stop().await;

        // Give the OS a moment to clean up
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify grandchild is dead
        assert!(
            !is_process_alive(grandchild_pid),
            "grandchild process {grandchild_pid} should have been killed"
        );
    }

    #[cfg(unix)]
    #[test_case(false)]
    #[test_case(TEST_PTY)]
    #[tokio::test]
    async fn test_force_kill_process_tree_cleanup(use_pty: bool) {
        let script = find_script_dir().join_component("spawn_child_sleep.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child =
            Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let grandchild_pid = {
            let mut out = Vec::new();
            match child.outputs().unwrap() {
                ChildOutput::Std { mut stdout, .. } => {
                    let mut buf = vec![0u8; 256];
                    let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                        .await
                        .expect("timed out reading grandchild PID")
                        .expect("failed to read stdout");
                    out.extend_from_slice(&buf[..n]);
                }
                ChildOutput::Pty(mut reader) => {
                    let mut buf = vec![0u8; 256];
                    let n = reader.read(&mut buf).expect("failed to read pty output");
                    out.extend_from_slice(&buf[..n]);
                }
            };
            let output = String::from_utf8(out).unwrap();
            let pid_line = output
                .lines()
                .find(|line| line.contains("CHILD_PID="))
                .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
            pid_line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap()
        };

        assert!(
            is_process_alive(grandchild_pid),
            "grandchild process {grandchild_pid} should be alive before force kill"
        );

        assert_eq!(child.kill().await, Some(ChildExit::Killed));
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            !is_process_alive(grandchild_pid),
            "grandchild process {grandchild_pid} should have been force killed"
        );
    }

    // Regression tests for the pre_exec/setsid -> process_group(0) migration.
    //
    // We replaced an unsafe pre_exec callback that called setsid() with tokio's
    // safe process_group(0) API. These tests verify the critical invariants:
    //
    // 1. The child gets its own process group (PGID == child PID, not parent's)
    // 2. Grandchildren inherit the child's process group
    // 3. kill(-pgid, SIGINT) reaches both child and grandchild
    // 4. The child is NOT a session leader (regression guard against setsid)

    #[cfg(unix)]
    #[tokio::test]
    async fn test_child_has_own_process_group() {
        let script = find_script_dir().join_component("sleep_5_interruptable.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
            None,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;
        let child_pgid = unsafe { libc::getpgid(child_pid) };
        let parent_pgid = unsafe { libc::getpgid(0) };

        // process_group(0) should make the child's PGID equal its own PID
        assert_eq!(
            child_pgid, child_pid,
            "child PGID ({child_pgid}) should equal child PID ({child_pid})"
        );

        // The child's process group must differ from the parent's
        assert_ne!(
            child_pgid, parent_pgid,
            "child PGID ({child_pgid}) must differ from parent PGID ({parent_pgid})"
        );

        child.stop().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_grandchild_inherits_child_process_group() {
        let script = find_script_dir().join_component("spawn_child_sleep.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
            None,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;

        // Read the grandchild PID from stdout
        let grandchild_pid = {
            let mut out = Vec::new();
            match child.outputs().unwrap() {
                ChildOutput::Std { mut stdout, .. } => {
                    let mut buf = vec![0u8; 256];
                    let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                        .await
                        .expect("timed out reading grandchild PID")
                        .expect("failed to read stdout");
                    out.extend_from_slice(&buf[..n]);
                }
                ChildOutput::Pty(_) => unreachable!("test uses non-PTY mode"),
            };
            let output = String::from_utf8(out).unwrap();
            let pid_line = output
                .lines()
                .find(|line| line.contains("CHILD_PID="))
                .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
            pid_line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse::<libc::pid_t>()
                .unwrap()
        };

        let child_pgid = unsafe { libc::getpgid(child_pid) };
        let grandchild_pgid = unsafe { libc::getpgid(grandchild_pid) };

        // Grandchild should be in the same process group as the child
        assert_eq!(
            grandchild_pgid, child_pgid,
            "grandchild PGID ({grandchild_pgid}) should match child PGID ({child_pgid})"
        );

        // Both should use child_pid as the group ID
        assert_eq!(
            child_pgid, child_pid,
            "process group ID ({child_pgid}) should equal child PID ({child_pid})"
        );

        child.stop().await;
        // Give OS time to clean up
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_sigint_to_process_group_reaches_grandchild() {
        let script = find_script_dir().join_component("spawn_child_sleep.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd.open_stdin();
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(2000))),
            None,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let child_pid = child.pid().expect("child should have a pid");

        // Read the grandchild PID
        let grandchild_pid = {
            let mut out = Vec::new();
            match child.outputs().unwrap() {
                ChildOutput::Std { mut stdout, .. } => {
                    let mut buf = vec![0u8; 256];
                    let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                        .await
                        .expect("timed out reading grandchild PID")
                        .expect("failed to read stdout");
                    out.extend_from_slice(&buf[..n]);
                }
                ChildOutput::Pty(_) => unreachable!("test uses non-PTY mode"),
            };
            let output = String::from_utf8(out).unwrap();
            let pid_line = output
                .lines()
                .find(|line| line.contains("CHILD_PID="))
                .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
            pid_line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap()
        };

        assert!(
            is_process_alive(grandchild_pid),
            "grandchild should be alive before signal"
        );

        // Send SIGINT to the process group (negative PID), exactly as
        // ShutdownStyle::Graceful does in production code
        let pgid = -(child_pid as i32);
        unsafe {
            libc::kill(pgid, libc::SIGINT);
        }

        // Wait for processes to die
        tokio::time::sleep(Duration::from_millis(500)).await;

        assert!(
            !is_process_alive(grandchild_pid),
            "grandchild should be dead after SIGINT to process group"
        );

        // Consume the exit
        child.wait().await;
    }

    // Guard against accidentally reverting to setsid(). With process_group(0),
    // the child calls setpgid(0, 0) which creates a new process group but does
    // NOT create a new session. If someone reintroduces setsid(), the child's
    // SID would equal its PID. With setpgid, the SID is inherited from the
    // parent.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_child_is_not_session_leader() {
        let script = find_script_dir().join_component("sleep_5_interruptable.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        let mut child = Child::spawn(
            cmd,
            ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
            None,
        )
        .unwrap();

        tokio::time::sleep(STARTUP_DELAY).await;

        let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;
        let child_sid = unsafe { libc::getsid(child_pid) };
        let parent_sid = unsafe { libc::getsid(0) };

        // With process_group(0), the child inherits the parent's session.
        // If setsid() were used instead, child_sid would equal child_pid.
        assert_ne!(
            child_sid, child_pid,
            "child SID ({child_sid}) should NOT equal child PID ({child_pid}) — that would mean \
             setsid() was called"
        );
        assert_eq!(
            child_sid, parent_sid,
            "child SID ({child_sid}) should equal parent SID ({parent_sid})"
        );

        child.stop().await;
    }

    fn is_process_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            // kill(pid, 0) checks if process exists without sending a signal
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::CloseHandle,
                System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            };
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if handle.is_null() {
                    return false;
                }
                // Process handle opened — check if it's actually still running
                let mut exit_code: u32 = 0;
                let result = windows_sys::Win32::System::Threading::GetExitCodeProcess(
                    handle,
                    &mut exit_code,
                );
                CloseHandle(handle);
                // STILL_ACTIVE (259) means the process is still running
                result != 0 && exit_code == 259
            }
        }
    }
}
