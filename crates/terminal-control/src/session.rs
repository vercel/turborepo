use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::frame::Frame;
use crate::recording::{self, InputOrigin};
use crate::semantic;
use crate::shot::{self, Host, Options, Shot, respond_to_output};
use crate::terminal_core::{InputModes, SCROLLBACK_ROWS, TerminalCore};
use crate::terminal_theme::TerminalTheme;
use anyhow::{Context, Result, bail};
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::workspace::{ActivityKind, WorkspaceContext};
pub use crate::workspace::{Direction as PaneDirection, PaneStatus, TabPosition, WindowStatus};

const OUTPUT_QUEUE: usize = 64;
const OUTPUT_BATCH: usize = OUTPUT_QUEUE;
const OUTPUT_CHUNK: usize = 1024;
const INITIAL_OUTPUT_GRACE: Duration = Duration::from_millis(50);
const CONTROL_PROTOCOL_VERSION: u8 = 1;
const ATTACH_PROTOCOL_VERSION: u8 = 2;
const WINDOW_PROTOCOL_VERSION: u8 = 3;
const LAYOUT_COMMAND_PROTOCOL_VERSION: u8 = 4;
const WORKSPACE_CONTROL_PROTOCOL_VERSION: u8 = 5;
const SEMANTIC_PROTOCOL_VERSION: u8 = 6;
const CURRENT_PROTOCOL_VERSION: u8 = SEMANTIC_PROTOCOL_VERSION;
const ATTACHED_TERMINAL_ERROR: &str = "workspace already has an attached terminal";

fn attachment_rejection(name: &str, error: &str) -> anyhow::Error {
    if error == ATTACHED_TERMINAL_ERROR {
        anyhow::anyhow!(
            "workspace {name:?} already has an attached terminal; detach it there with ctrl-b d, or choose another workspace with `termctrl run NAME`"
        )
    } else {
        anyhow::anyhow!(error.to_owned())
    }
}

fn valid_workspace_attachment_size(cols: u16, rows: u16) -> bool {
    cols > 0 && rows >= 2
}

struct Output {
    at_ms: u64,
    bytes: Vec<u8>,
}

/// One running terminal application controlled in-process by its caller.
///
/// `Session` is the embedded equivalent of the CLI `session` lifecycle. It owns a PTY and the
/// visible terminal state, so callers can send input, wait for content, take shots, and resize
/// without spawning a new `termctrl` command for each action. Ghostty terminal state is
/// thread-confined, so a session must remain on the thread where it was created.
pub struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
    #[cfg(unix)]
    process_group: Option<i32>,
    terminal: TerminalCore,
    ansi: Vec<u8>,
    host: Host,
    receive: Receiver<Option<Output>>,
    max_bytes: usize,
    ansi_truncated: bool,
    output_closed: bool,
    stopped: bool,
    exit: Option<ProcessExit>,
    exit_drain_started: Option<Instant>,
    exit_ready: bool,
    last_output: Option<Instant>,
    recording: Option<recording::Writer>,
    capture_input: bool,
    captured_input: Vec<(InputOrigin, Vec<u8>)>,
    cols: u16,
    rows: u16,
    cell_width: u16,
    cell_height: u16,
    launch: SessionLaunch,
    mirror: Option<Box<dyn Write + Send>>,
    semantic: Option<semantic::Host>,
}

/// Lifecycle state of a running or completed session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Running,
    Exited,
}

/// Termination information observed for a completed terminal application.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProcessExit {
    pub code: u32,
    pub signal: Option<String>,
    pub success: bool,
}

impl From<ExitStatus> for ProcessExit {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.exit_code(),
            signal: status.signal().map(str::to_owned),
            success: status.success(),
        }
    }
}

/// Reason a session capture returned its visible shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureReason {
    Idle,
    Deadline,
    Exited,
    OutputClosed,
}

/// A visible shot together with the condition that made it observable.
#[derive(Deserialize, Serialize)]
pub struct CaptureResult {
    pub shot: Shot,
    pub reason: CaptureReason,
}

/// Observable state of one embedded or named terminal session.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionStatus {
    pub state: SessionState,
    pub exit: Option<ProcessExit>,
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub idle_for_ms: Option<u64>,
    pub has_visible_content: bool,
    pub recording: bool,
    pub logs_truncated: bool,
    pub launch: SessionLaunch,
}

/// Non-secret launch settings retained for status display and named-session restart.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionLaunch {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub record: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub max_bytes: usize,
    pub opentui_host: bool,
    pub color: shot::ColorMode,
}

/// One named daemon session discovered in the local runtime directory.
#[derive(Debug, Serialize)]
pub struct NamedSessionStatus {
    pub name: String,
    pub status: Option<SessionStatus>,
    pub error: Option<String>,
    pub unavailable: Option<UnavailableReason>,
}

/// Why a named session socket could not report normal status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    Stale,
    IncompatibleProtocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneKind {
    Exited,
    Stale,
}

impl Session {
    /// Start `command` inside a live PTY-backed session.
    pub fn start(
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
    ) -> Result<Self> {
        Self::start_with_theme(command, cwd, record, options, TerminalTheme::default())
    }

    pub(crate) fn start_with_theme(
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        theme: TerminalTheme,
    ) -> Result<Self> {
        if command.is_empty() {
            bail!("provide a command after --");
        }
        if options.cols == 0 || options.rows == 0 {
            bail!("terminal dimensions must be greater than zero");
        }
        let cwd = match cwd {
            Some(cwd) if cwd.is_absolute() => cwd.to_owned(),
            Some(cwd) => std::env::current_dir()
                .context("resolve session working directory")?
                .join(cwd),
            None => std::env::current_dir().context("resolve session working directory")?,
        };
        let cwd = fs::canonicalize(&cwd).context("canonicalize session working directory")?;
        let terminal =
            TerminalCore::new_with_theme(options.rows, options.cols, SCROLLBACK_ROWS, theme)?;
        let started = Instant::now();
        let recording = record
            .map(|path| {
                recording::Writer::new(
                    path,
                    started,
                    options.cols,
                    options.rows,
                    options.cell_width,
                    options.cell_height,
                )
            })
            .transpose()?;
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: options.rows,
                cols: options.cols,
                pixel_width: options.cell_width,
                pixel_height: options.cell_height,
            })
            .context("open session pseudo-terminal")?;
        let semantic = options
            .opentui_host
            .then(semantic::Host::bind)
            .transpose()?;
        let mut builder = CommandBuilder::new(&command[0]);
        builder.args(&command[1..]);
        shot::configure_pty_environment(&mut builder, options);
        if let Some(path) = semantic.as_ref().and_then(semantic::Host::path) {
            builder.env(semantic::SOCKET_ENV, path);
        }
        builder.cwd(&cwd);
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("open session PTY reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("open session PTY writer")?;
        let (send, receive) = mpsc::sync_channel(OUTPUT_QUEUE);
        let (reader_ready, wait_for_reader) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = reader_ready.send(());
            let mut buffer = [0_u8; OUTPUT_CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(len) => {
                        if send
                            .send(Some(Output {
                                at_ms: started.elapsed().as_millis() as u64,
                                bytes: buffer[..len].to_vec(),
                            }))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = send.send(None);
        });
        wait_for_reader
            .recv()
            .context("session PTY reader exited during startup")?;
        let child = pair
            .slave
            .spawn_command(builder)
            .context("spawn session command")?;
        drop(pair.slave);
        #[cfg(unix)]
        let process_group = child.process_id().and_then(|pid| i32::try_from(pid).ok());
        Ok(Self {
            master: pair.master,
            child,
            #[cfg(unix)]
            process_group,
            terminal,
            ansi: Vec::new(),
            host: Host::new_with_theme(writer, options, theme),
            receive,
            max_bytes: options.max_bytes,
            ansi_truncated: false,
            output_closed: false,
            stopped: false,
            exit: None,
            exit_drain_started: None,
            exit_ready: false,
            last_output: None,
            recording,
            capture_input: false,
            captured_input: Vec::new(),
            cols: options.cols,
            rows: options.rows,
            cell_width: options.cell_width,
            cell_height: options.cell_height,
            launch: SessionLaunch {
                command: command.to_vec(),
                cwd,
                record: record.map(Path::to_owned),
                cols: options.cols,
                rows: options.rows,
                cell_width: options.cell_width,
                cell_height: options.cell_height,
                max_bytes: options.max_bytes,
                opentui_host: options.opentui_host,
                color: options.color,
            },
            mirror: None,
            semantic,
        })
    }

    /// Mirror subsequent PTY output to another terminal-facing writer.
    pub fn mirror_to(&mut self, writer: impl Write + Send + 'static) {
        self.mirror = Some(Box::new(writer));
    }

    /// Send one input burst to the terminal application.
    pub fn send(&mut self, input: &[u8]) -> Result<()> {
        self.consume_batch()?;
        if self.has_exited()? || self.stopped {
            bail!("session command has exited");
        }
        self.write_input(input)
    }

    pub(crate) fn send_current(&mut self, input: &[u8]) -> Result<()> {
        if self.exit.is_some() || self.stopped {
            bail!("session command has exited");
        }
        self.write_input(input)
    }

    pub(crate) fn set_theme(&mut self, theme: TerminalTheme) -> Result<()> {
        self.terminal.set_theme(theme)?;
        self.host.set_theme(theme);
        Ok(())
    }

    pub(crate) fn send_current_if_open(&mut self, input: &[u8]) -> Result<bool> {
        if self.exit.is_some() || self.stopped {
            return Ok(false);
        }
        if !self.host.send_if_open(input)? {
            return Ok(false);
        }
        self.record_input(InputOrigin::Client, input)?;
        Ok(true)
    }

    fn write_input(&mut self, input: &[u8]) -> Result<()> {
        if self.exit.is_some() || self.stopped {
            bail!("session command has exited");
        }
        self.host.send(input)?;
        self.record_input(InputOrigin::Client, input)?;
        Ok(())
    }

    pub(crate) fn capture_input(&mut self) {
        self.capture_input = true;
    }

    pub(crate) fn take_captured_input(&mut self) -> Vec<(InputOrigin, Vec<u8>)> {
        std::mem::take(&mut self.captured_input)
    }

    fn record_input(&mut self, origin: InputOrigin, input: &[u8]) -> Result<()> {
        if let Some(recording) = &mut self.recording {
            recording.input(origin, input)?;
        }
        if self.capture_input {
            self.captured_input.push((origin, input.to_vec()));
        }
        Ok(())
    }

    /// Send ordered input bursts, optionally pacing them for recorded interactions.
    pub fn send_all(&mut self, input: &[Vec<u8>], pace: Duration) -> Result<()> {
        self.consume_batch()?;
        if self.has_exited()? || self.stopped {
            bail!("session command has exited");
        }
        let last = input.len().saturating_sub(1);
        for (index, bytes) in input.iter().enumerate() {
            self.write_input(bytes)?;
            if !pace.is_zero() && index < last {
                thread::sleep(pace);
                self.consume_batch()?;
            }
        }
        Ok(())
    }

    /// Wait until visible terminal text contains `text`.
    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.consume_batch()?;
            if self.terminal.text()?.contains(text) {
                return Ok(());
            }
            if self.has_exited()? || self.stopped {
                bail!("session ended before visible terminal included {text:?}");
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for visible terminal text {text:?}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait until no terminal output has arrived for `settle`.
    pub fn wait_for_idle(&mut self, settle: Duration, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        let deadline = started + timeout;
        loop {
            self.consume_batch()?;
            if self.output_closed || self.last_output.unwrap_or(started).elapsed() >= settle {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for terminal output to settle");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait for the terminal application to exit, returning `None` on timeout.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Result<Option<ProcessExit>> {
        let deadline = Instant::now() + timeout;
        loop {
            self.consume_batch()?;
            if self.has_exited()? || self.stopped {
                return Ok(self.exit.clone());
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Capture visible terminal state and report whether it settled, exited, or reached a limit.
    pub fn capture(&mut self, settle: Duration, deadline: Duration) -> Result<CaptureResult> {
        let started = Instant::now();
        let deadline = started + deadline;
        loop {
            self.consume_batch()?;
            let reason = if self.has_exited()? || self.stopped {
                Some(CaptureReason::Exited)
            } else if self.output_closed {
                Some(CaptureReason::OutputClosed)
            } else if self.last_output.map_or_else(
                || settle.is_zero() || started.elapsed() >= settle.max(INITIAL_OUTPUT_GRACE),
                |last_output| last_output.elapsed() >= settle,
            ) {
                Some(CaptureReason::Idle)
            } else if Instant::now() >= deadline {
                Some(CaptureReason::Deadline)
            } else {
                None
            };
            if let Some(reason) = reason {
                return Ok(CaptureResult {
                    shot: Shot {
                        frame: self.terminal.frame()?,
                        ansi: self.ansi.clone(),
                    },
                    reason,
                });
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Inspect session lifecycle, geometry, and whether a visible frame is available.
    pub fn status(&mut self) -> Result<SessionStatus> {
        self.consume_batch()?;
        self.has_exited()?;
        Ok(SessionStatus {
            state: if self.exit.is_some() || self.stopped {
                SessionState::Exited
            } else {
                SessionState::Running
            },
            exit: self.exit.clone(),
            cols: self.cols,
            rows: self.rows,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            idle_for_ms: self
                .last_output
                .map(|last| last.elapsed().as_millis() as u64),
            has_visible_content: self.terminal.frame()?.has_visible_content(),
            recording: self.recording.is_some(),
            logs_truncated: self.ansi_truncated,
            launch: self.launch.clone(),
        })
    }

    /// Return readable normal-screen scrollback, or the exact retained ANSI/VT stream.
    pub fn logs(&mut self, ansi: bool) -> Result<Vec<u8>> {
        self.consume_batch()?;
        if ansi {
            return Ok(self.ansi.clone());
        }
        self.terminal.scrollback_text()
    }

    /// Return the application's semantic UI snapshot, or an empty snapshot when unavailable.
    pub fn semantic_snapshot(&mut self, timeout: Duration) -> Result<Value> {
        let Some(mut semantic) = self.semantic.take() else {
            return Ok(semantic::empty_semantic_snapshot());
        };
        let result = semantic
            .snapshot(timeout, || {
                self.consume_output_batch()?;
                Ok(self.has_exited()? || self.stopped)
            })
            .map(|snapshot| snapshot.unwrap_or_else(semantic::empty_semantic_snapshot));
        if self.exit.is_none() && !self.stopped {
            self.semantic = Some(semantic);
        }
        result
    }

    /// Resize the PTY and reflow subsequent terminal parsing at the new dimensions.
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<()> {
        if cols == 0 || rows == 0 {
            bail!("terminal dimensions must be greater than zero");
        }
        if (cols, rows, cell_width, cell_height)
            == (self.cols, self.rows, self.cell_width, self.cell_height)
        {
            return Ok(());
        }
        self.consume_batch()?;
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: cell_width,
                pixel_height: cell_height,
            })
            .context("resize session pseudo-terminal")?;
        self.host.resize(cols, rows, cell_width, cell_height);
        self.terminal.resize(cols, rows, cell_width, cell_height)?;
        self.cols = cols;
        self.rows = rows;
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        if let Some(recording) = &mut self.recording {
            recording.resize(cols, rows, cell_width, cell_height)?;
        }
        Ok(())
    }

    /// Add a named moment to the active recording timeline.
    pub fn mark(&mut self, name: &str) -> Result<()> {
        self.consume_batch()?;
        let recording = self
            .recording
            .as_mut()
            .context("session was not started with --record")?;
        recording.marker(name)
    }

    /// Terminate the application owned by this session.
    pub fn stop(&mut self) -> Result<()> {
        self.terminate();
        Ok(())
    }

    pub(crate) fn pump(&mut self) -> Result<()> {
        self.consume_batch()
    }

    pub(crate) fn current_frame(&mut self) -> Result<Frame> {
        self.terminal.frame()
    }

    pub(crate) fn frame_revision(&self) -> u64 {
        self.terminal.revision()
    }

    pub(crate) fn is_exited(&mut self) -> Result<bool> {
        self.has_exited()
    }

    pub(crate) fn exit_observed(&self) -> bool {
        self.exit_ready
    }

    pub(crate) fn idle_for(&self, started: Instant) -> Duration {
        self.last_output
            .map_or_else(|| started.elapsed(), |last| last.elapsed())
    }

    pub(crate) fn snapshot(&mut self) -> Result<Shot> {
        self.consume_batch()?;
        Ok(Shot {
            frame: self.terminal.frame()?,
            ansi: self.ansi.clone(),
        })
    }

    pub(crate) fn input_modes(&self) -> Result<InputModes> {
        self.terminal.input_modes()
    }

    pub(crate) fn title(&self) -> Result<String> {
        self.terminal.title()
    }

    pub(crate) fn take_bells(&self) -> u64 {
        self.terminal.take_bells()
    }

    pub(crate) fn cursor_style(&self) -> libghostty_vt::render::CursorVisualStyle {
        self.terminal.cursor_style()
    }

    fn consume_batch(&mut self) -> Result<()> {
        if let Some(semantic) = &mut self.semantic {
            semantic.pump();
        }
        self.consume_output_batch()
    }

    fn consume_output_batch(&mut self) -> Result<()> {
        let mut outputs = Vec::new();
        for _ in 0..OUTPUT_BATCH {
            match self.receive.try_recv() {
                Ok(Some(output)) => outputs.push(output),
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.output_closed = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        self.apply_outputs(outputs)
    }

    fn consume_one(&mut self) -> Result<bool> {
        match self.receive.try_recv() {
            Ok(Some(output)) => {
                self.apply_output(output)?;
                Ok(true)
            }
            Ok(None) | Err(TryRecvError::Disconnected) => {
                self.output_closed = true;
                Ok(false)
            }
            Err(TryRecvError::Empty) => Ok(false),
        }
    }

    fn has_exited(&mut self) -> Result<bool> {
        if self.exit.is_none()
            && let Some(status) = self.child.try_wait().context("poll session command")?
        {
            self.exit = Some(status.into());
            self.exit_drain_started = Some(Instant::now());
        }
        if self.exit.is_none() {
            return Ok(false);
        }
        self.consume_batch()?;
        if self.output_closed {
            #[cfg(unix)]
            if let Some(process_group) = self.process_group.take() {
                unsafe {
                    libc::kill(-process_group, libc::SIGKILL);
                }
            }
            self.exit_ready = true;
            return Ok(true);
        }
        let elapsed = self
            .exit_drain_started
            .map_or(Duration::ZERO, |at| at.elapsed());
        #[cfg(unix)]
        if elapsed >= Duration::from_millis(50)
            && let Some(process_group) = self.process_group.take()
        {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
        if elapsed >= Duration::from_secs(1) {
            self.exit_ready = true;
        }
        Ok(self.exit_ready)
    }

    fn terminate(&mut self) {
        if self.stopped {
            return;
        }
        #[cfg(unix)]
        let process_group = self.process_group;
        #[cfg(unix)]
        if let Some(process_group) = process_group {
            unsafe {
                libc::kill(-process_group, libc::SIGHUP);
            }
        }
        let graceful_deadline = Instant::now() + Duration::from_millis(150);
        while self.exit.is_none() && Instant::now() < graceful_deadline {
            let _ = self.consume_one();
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit = Some(status.into());
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        if self.exit.is_some() {
            let _ = self.finish_exited_output();
        }
        #[cfg(unix)]
        if let Some(process_group) = process_group {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
        #[cfg(unix)]
        {
            self.process_group = None;
        }
        if self.exit.is_none() {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while self.exit.is_none() && Instant::now() < deadline {
            // The PTY reader may be blocked by the bounded queue while the child exits.
            // Keep draining one chunk at a time so forced shutdown cannot deadlock on backpressure.
            let _ = self.consume_one();
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit = Some(status.into());
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let drain_deadline = Instant::now() + Duration::from_millis(100);
        while !self.output_closed && Instant::now() < drain_deadline {
            match self.receive.recv_timeout(Duration::from_millis(5)) {
                Ok(Some(output)) => {
                    let _ = self.apply_output(output);
                }
                Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.output_closed = true;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        self.output_closed = true;
        self.stopped = true;
        self.exit_ready = self.exit.is_some();
        self.semantic.take();
    }

    fn finish_exited_output(&mut self) -> Result<()> {
        let kill_after = Instant::now() + Duration::from_millis(50);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !self.output_closed && Instant::now() < deadline {
            // A cleanly exited application should close the PTY promptly. Only signal its
            // saved group if output remains open long enough to indicate a live descendant.
            #[cfg(unix)]
            if Instant::now() >= kill_after
                && let Some(process_group) = self.process_group.take()
            {
                unsafe {
                    libc::kill(-process_group, libc::SIGKILL);
                }
            }
            match self.receive.recv_timeout(Duration::from_millis(10)) {
                Ok(Some(output)) => {
                    let mut outputs = vec![output];
                    for _ in 1..OUTPUT_BATCH {
                        match self.receive.recv_timeout(Duration::from_millis(1)) {
                            Ok(Some(output)) => outputs.push(output),
                            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                self.output_closed = true;
                                break;
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        }
                    }
                    self.apply_outputs(outputs)?;
                }
                Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    self.output_closed = true;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        #[cfg(unix)]
        if self.output_closed {
            self.process_group.take();
        }
        self.exit_ready = true;
        self.semantic.take();
        Ok(())
    }

    fn apply_output(&mut self, output: Output) -> Result<()> {
        self.apply_outputs(vec![output])
    }

    fn apply_outputs(&mut self, outputs: Vec<Output>) -> Result<()> {
        if outputs.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(outputs.iter().map(|output| output.bytes.len()).sum());
        for output in outputs {
            if let Some(mirror) = &mut self.mirror {
                mirror
                    .write_all(&output.bytes)
                    .context("mirror PTY output")?;
            }
            if let Some(recording) = &mut self.recording {
                recording.output(output.at_ms, &output.bytes)?;
            }
            retain_recent(
                &mut self.ansi,
                &output.bytes,
                self.max_bytes,
                &mut self.ansi_truncated,
            );
            bytes.extend_from_slice(&output.bytes);
        }
        if let Some(mirror) = &mut self.mirror {
            mirror.flush().context("flush mirrored PTY output")?;
        }
        let response = respond_to_output(&mut self.terminal, &mut self.host, &bytes)?;
        if !response.is_empty() {
            self.record_input(InputOrigin::Host, &response)?;
        }
        self.last_output = Some(Instant::now());
        Ok(())
    }
}

fn retain_recent(ansi: &mut Vec<u8>, bytes: &[u8], max_bytes: usize, truncated: &mut bool) {
    if max_bytes == 0 {
        *truncated |= !bytes.is_empty();
        ansi.clear();
        return;
    }
    if bytes.len() >= max_bytes {
        *truncated |= !ansi.is_empty() || bytes.len() > max_bytes;
        ansi.clear();
        ansi.extend_from_slice(&bytes[bytes.len() - max_bytes..]);
        return;
    }
    let excess = ansi
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(max_bytes);
    if excess > 0 {
        ansi.drain(..excess);
        *truncated = true;
    }
    ansi.extend_from_slice(bytes);
}

impl Drop for Session {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Serialize, Deserialize)]
enum Request {
    Ping,
    Status,
    Wait {
        text: String,
        timeout_ms: u64,
        #[serde(default)]
        pane: Option<crate::workspace::PaneId>,
    },
    Send {
        input: Vec<Vec<u8>>,
        pace_ms: u64,
        #[serde(default)]
        pane: Option<crate::workspace::PaneId>,
    },
    Show {
        settle_ms: u64,
        deadline_ms: u64,
        #[serde(default)]
        pane: Option<crate::workspace::PaneId>,
    },
    Logs {
        ansi: bool,
    },
    Resize {
        cols: u16,
        rows: u16,
        cell_width: Option<u16>,
        cell_height: Option<u16>,
    },
    Mark {
        name: String,
    },
    Windows,
    WorkspaceContext {
        #[serde(default)]
        pane: Option<crate::workspace::PaneId>,
    },
    SetTabPosition {
        position: TabPosition,
    },
    MoveWindow {
        name: String,
        index: usize,
    },
    CreateWindow {
        name: Option<String>,
        command: Vec<String>,
        cwd: Option<PathBuf>,
    },
    SelectWindow {
        name: String,
    },
    RenameWindow {
        name: String,
        new_name: String,
    },
    CloseWindow {
        name: String,
    },
    WindowPanes {
        name: String,
    },
    WindowLayout {
        name: String,
        columns: u16,
        rows: u16,
        #[serde(default)]
        command: Vec<String>,
    },
    ShowWindow {
        name: String,
        settle_ms: u64,
        deadline_ms: u64,
    },
    SendWindow {
        name: String,
        input: Vec<Vec<u8>>,
        pace_ms: u64,
    },
    WaitWindow {
        name: String,
        text: String,
        timeout_ms: u64,
    },
    LogsWindow {
        name: String,
        ansi: bool,
    },
    MovePane {
        pane: crate::workspace::PaneId,
        window: String,
        vertical: bool,
    },
    ResizePane {
        pane: crate::workspace::PaneId,
        direction: PaneDirection,
        cells: u16,
    },
    ToggleZoom {
        pane: crate::workspace::PaneId,
    },
    Panes,
    Layout {
        columns: u16,
        rows: u16,
        #[serde(default)]
        command: Vec<String>,
    },
    FocusPane {
        pane: crate::workspace::PaneId,
    },
    ClosePane {
        pane: crate::workspace::PaneId,
    },
    Attach {
        id: u64,
        socket: PathBuf,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
        theme: TerminalTheme,
    },
    ResizeAttachment {
        id: u64,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    },
    SemanticSnapshot {
        deadline_unix_ms: u64,
        #[serde(default)]
        pane: Option<crate::workspace::PaneId>,
        #[serde(default)]
        window: Option<String>,
    },
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProtocolCapability {
    Pane,
    Attachment,
    Window,
    LayoutCommand,
    WorkspaceControl,
    Semantic,
}

impl ProtocolCapability {
    const fn version(self) -> u8 {
        match self {
            Self::Pane => CONTROL_PROTOCOL_VERSION,
            Self::Attachment => ATTACH_PROTOCOL_VERSION,
            Self::Window => WINDOW_PROTOCOL_VERSION,
            Self::LayoutCommand => LAYOUT_COMMAND_PROTOCOL_VERSION,
            Self::WorkspaceControl => WORKSPACE_CONTROL_PROTOCOL_VERSION,
            Self::Semantic => SEMANTIC_PROTOCOL_VERSION,
        }
    }

    const fn restart_error(self) -> &'static str {
        match self {
            Self::Pane => {
                "running session predates pane layout control; restart it with the current termctrl"
            }
            Self::Attachment => {
                "running workspace predates terminal reattachment; restart it with the current termctrl"
            }
            Self::Window => {
                "running workspace predates named windows; restart it with the current termctrl"
            }
            Self::LayoutCommand => {
                "running workspace predates pane startup commands; restart it with the current termctrl"
            }
            Self::WorkspaceControl => {
                "running workspace predates runtime workspace controls; restart it with the current termctrl"
            }
            Self::Semantic => {
                "running session predates semantic snapshots; restart it with the current termctrl"
            }
        }
    }

    fn require(self, response: &Response) -> Result<()> {
        if response.protocol_version < self.version() {
            bail!(self.restart_error());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RequestRequirements {
    capability: Option<ProtocolCapability>,
    hold_name_lock: bool,
}

impl Request {
    fn requirements(&self) -> RequestRequirements {
        let capability = match self {
            Self::Layout { command, .. } | Self::WindowLayout { command, .. }
                if !command.is_empty() =>
            {
                ProtocolCapability::LayoutCommand
            }
            Self::WorkspaceContext { .. }
            | Self::SetTabPosition { .. }
            | Self::MoveWindow { .. } => ProtocolCapability::WorkspaceControl,
            Self::Windows
            | Self::CreateWindow { .. }
            | Self::SelectWindow { .. }
            | Self::RenameWindow { .. }
            | Self::CloseWindow { .. }
            | Self::WindowPanes { .. }
            | Self::WindowLayout { .. }
            | Self::ShowWindow { .. }
            | Self::SendWindow { .. }
            | Self::WaitWindow { .. }
            | Self::LogsWindow { .. }
            | Self::MovePane { .. }
            | Self::ResizePane { .. }
            | Self::ToggleZoom { .. } => ProtocolCapability::Window,
            Self::Wait { pane: Some(_), .. }
            | Self::Send { pane: Some(_), .. }
            | Self::Show { pane: Some(_), .. }
            | Self::Panes
            | Self::Layout { .. }
            | Self::FocusPane { .. }
            | Self::ClosePane { .. } => ProtocolCapability::Pane,
            Self::Attach { .. } | Self::ResizeAttachment { .. } => ProtocolCapability::Attachment,
            Self::SemanticSnapshot { .. } => ProtocolCapability::Semantic,
            Self::Ping
            | Self::Status
            | Self::Wait { pane: None, .. }
            | Self::Send { pane: None, .. }
            | Self::Show { pane: None, .. }
            | Self::Logs { .. }
            | Self::Resize { .. }
            | Self::Mark { .. }
            | Self::Stop => {
                return RequestRequirements {
                    capability: None,
                    hold_name_lock: false,
                };
            }
        };
        RequestRequirements {
            capability: Some(capability),
            hold_name_lock: capability == ProtocolCapability::LayoutCommand,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Response {
    #[serde(default)]
    protocol_version: u8,
    error: Option<String>,
    captured: Option<Shot>,
    status: Option<SessionStatus>,
    logs: Option<Vec<u8>>,
    #[serde(default)]
    panes: Option<Vec<crate::workspace::PaneStatus>>,
    #[serde(default)]
    windows: Option<Vec<crate::workspace::WindowStatus>>,
    #[serde(default)]
    context: Option<WorkspaceContext>,
    #[serde(default)]
    semantic_snapshot: Option<SemanticSnapshotResult>,
}

impl Default for Response {
    fn default() -> Self {
        Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            error: None,
            captured: None,
            status: None,
            logs: None,
            panes: None,
            windows: None,
            context: None,
            semantic_snapshot: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SemanticSnapshotResult {
    value: Value,
}

impl Response {
    fn error(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

#[doc(hidden)]
pub fn start(
    name: &str,
    command: &[String],
    cwd: Option<&Path>,
    record: Option<&Path>,
    options: &Options,
) -> Result<()> {
    validate_name(name)?;
    implementation::start(name, command, cwd, record, options)
}

#[doc(hidden)]
pub fn restart(
    name: &str,
    command: &[String],
    cwd: Option<&Path>,
    record: Option<&Path>,
    options: &Options,
) -> Result<()> {
    validate_name(name)?;
    implementation::restart(name, command, cwd, record, options)
}

#[doc(hidden)]
pub fn wait(name: &str, text: String, timeout: Duration) -> Result<()> {
    wait_for_target(name, TerminalTarget::Selected, text, timeout)
}

#[derive(Clone, Debug)]
pub(crate) enum TerminalTarget {
    Selected,
    Window(String),
    Pane(crate::workspace::PaneId),
}

pub(crate) fn terminal_target(
    window: Option<String>,
    pane: Option<crate::workspace::PaneId>,
) -> Result<TerminalTarget> {
    match (window, pane) {
        (Some(_), Some(_)) => {
            bail!("window and pane cannot be combined; pane ids are already globally stable")
        }
        (Some(window), None) => Ok(TerminalTarget::Window(window)),
        (None, Some(pane)) => Ok(TerminalTarget::Pane(pane)),
        (None, None) => Ok(TerminalTarget::Selected),
    }
}

#[doc(hidden)]
pub fn wait_for(
    name: &str,
    pane: Option<crate::workspace::PaneId>,
    text: String,
    timeout: Duration,
) -> Result<()> {
    let target = pane.map_or(TerminalTarget::Selected, TerminalTarget::Pane);
    wait_for_target(name, target, text, timeout)
}

pub(crate) fn wait_for_target(
    name: &str,
    target: TerminalTarget,
    text: String,
    timeout: Duration,
) -> Result<()> {
    let operation = match target {
        TerminalTarget::Selected => Request::Wait {
            text,
            timeout_ms: timeout.as_millis() as u64,
            pane: None,
        },
        TerminalTarget::Pane(pane) => Request::Wait {
            text,
            timeout_ms: timeout.as_millis() as u64,
            pane: Some(pane),
        },
        TerminalTarget::Window(name) => Request::WaitWindow {
            name,
            text,
            timeout_ms: timeout.as_millis() as u64,
        },
    };
    request(name, operation)?;
    Ok(())
}

#[doc(hidden)]
pub fn wait_for_in(
    name: &str,
    window: Option<String>,
    pane: Option<crate::workspace::PaneId>,
    text: String,
    timeout: Duration,
) -> Result<()> {
    wait_for_target(name, terminal_target(window, pane)?, text, timeout)
}

#[doc(hidden)]
pub fn status(name: &str) -> Result<SessionStatus> {
    request(name, Request::Status)?
        .status
        .ok_or_else(|| anyhow::anyhow!("session did not return status"))
}

#[doc(hidden)]
pub fn send(name: &str, input: Vec<Vec<u8>>, pace: Duration) -> Result<()> {
    send_to_target(name, TerminalTarget::Selected, input, pace)
}

#[doc(hidden)]
pub fn send_to(
    name: &str,
    pane: Option<crate::workspace::PaneId>,
    input: Vec<Vec<u8>>,
    pace: Duration,
) -> Result<()> {
    let target = pane.map_or(TerminalTarget::Selected, TerminalTarget::Pane);
    send_to_target(name, target, input, pace)
}

pub(crate) fn send_to_target(
    name: &str,
    target: TerminalTarget,
    input: Vec<Vec<u8>>,
    pace: Duration,
) -> Result<()> {
    let operation = match target {
        TerminalTarget::Selected => Request::Send {
            input,
            pace_ms: pace.as_millis() as u64,
            pane: None,
        },
        TerminalTarget::Pane(pane) => Request::Send {
            input,
            pace_ms: pace.as_millis() as u64,
            pane: Some(pane),
        },
        TerminalTarget::Window(name) => Request::SendWindow {
            name,
            input,
            pace_ms: pace.as_millis() as u64,
        },
    };
    request(name, operation)?;
    Ok(())
}

#[doc(hidden)]
pub fn send_to_in(
    name: &str,
    window: Option<String>,
    pane: Option<crate::workspace::PaneId>,
    input: Vec<Vec<u8>>,
    pace: Duration,
) -> Result<()> {
    send_to_target(name, terminal_target(window, pane)?, input, pace)
}

#[doc(hidden)]
pub fn show(name: &str, settle: Duration, deadline: Duration) -> Result<Shot> {
    show_target(name, TerminalTarget::Selected, settle, deadline)
}

#[doc(hidden)]
pub fn show_pane(
    name: &str,
    pane: Option<crate::workspace::PaneId>,
    settle: Duration,
    deadline: Duration,
) -> Result<Shot> {
    let target = pane.map_or(TerminalTarget::Selected, TerminalTarget::Pane);
    show_target(name, target, settle, deadline)
}

pub(crate) fn show_target(
    name: &str,
    target: TerminalTarget,
    settle: Duration,
    deadline: Duration,
) -> Result<Shot> {
    show_target_response(name, target, settle, deadline)?
        .captured
        .ok_or_else(|| anyhow::anyhow!("session did not return a visible screen"))
}

// Retained upstream helper for the CLI omitted from this library-only copy.
#[allow(dead_code)]
pub(crate) fn show_target_with_status(
    name: &str,
    target: TerminalTarget,
    settle: Duration,
    deadline: Duration,
) -> Result<(Shot, SessionStatus)> {
    let response = show_target_response(name, target, settle, deadline)?;
    Ok((
        response
            .captured
            .context("session did not return a visible screen")?,
        response.status.context("session did not return status")?,
    ))
}

fn show_target_response(
    name: &str,
    target: TerminalTarget,
    settle: Duration,
    deadline: Duration,
) -> Result<Response> {
    let operation = match target {
        TerminalTarget::Selected => Request::Show {
            settle_ms: settle.as_millis() as u64,
            deadline_ms: deadline.as_millis() as u64,
            pane: None,
        },
        TerminalTarget::Pane(pane) => Request::Show {
            settle_ms: settle.as_millis() as u64,
            deadline_ms: deadline.as_millis() as u64,
            pane: Some(pane),
        },
        TerminalTarget::Window(name) => Request::ShowWindow {
            name,
            settle_ms: settle.as_millis() as u64,
            deadline_ms: deadline.as_millis() as u64,
        },
    };
    request(name, operation)
}

#[doc(hidden)]
pub fn show_in(
    name: &str,
    window: Option<String>,
    pane: Option<crate::workspace::PaneId>,
    settle: Duration,
    deadline: Duration,
) -> Result<Shot> {
    show_target(name, terminal_target(window, pane)?, settle, deadline)
}

#[doc(hidden)]
pub fn semantic_snapshot_in(
    name: &str,
    window: Option<String>,
    pane: Option<crate::workspace::PaneId>,
    timeout: Duration,
) -> Result<Value> {
    validate_name(name)?;
    let (window, pane) = match terminal_target(window, pane)? {
        TerminalTarget::Selected => (None, None),
        TerminalTarget::Window(window) => (Some(window), None),
        TerminalTarget::Pane(pane) => (None, Some(pane)),
    };
    let response = semantic::request_snapshot(
        &socket_path(name)?,
        &Request::SemanticSnapshot {
            deadline_unix_ms: semantic::deadline_unix_ms(timeout)?,
            pane,
            window,
        },
        timeout,
    )?;
    ProtocolCapability::Semantic.require(&response)?;
    if let Some(error) = response.error {
        bail!(error);
    }
    response
        .semantic_snapshot
        .map(|snapshot| snapshot.value)
        .context("session did not return a semantic snapshot")
}

#[doc(hidden)]
pub fn panes(name: &str) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(name, Request::Panes)?)
}

#[doc(hidden)]
pub fn windows(name: &str) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(name, Request::Windows)?)
}

#[doc(hidden)]
pub fn workspace_context(
    name: &str,
    pane: Option<crate::workspace::PaneId>,
) -> Result<WorkspaceContext> {
    request(name, Request::WorkspaceContext { pane })?
        .context
        .ok_or_else(|| anyhow::anyhow!("session did not return workspace context"))
}

#[doc(hidden)]
pub fn set_workspace_tab_position(
    workspace: &str,
    position: TabPosition,
) -> Result<Vec<WindowStatus>> {
    window_response(request(workspace, Request::SetTabPosition { position })?)
}

#[doc(hidden)]
pub fn move_workspace_window(
    workspace: &str,
    name: String,
    index: usize,
) -> Result<Vec<WindowStatus>> {
    window_response(request(workspace, Request::MoveWindow { name, index })?)
}

#[doc(hidden)]
pub fn panes_in_window(
    workspace: &str,
    window: Option<String>,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    match window {
        Some(name) => pane_response(request(workspace, Request::WindowPanes { name })?),
        None => panes(workspace),
    }
}

#[doc(hidden)]
pub fn set_workspace_layout_in_window(
    workspace: &str,
    window: Option<String>,
    columns: u16,
    rows: u16,
    command: Vec<String>,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    match window {
        Some(name) => pane_response(request(
            workspace,
            Request::WindowLayout {
                name,
                columns,
                rows,
                command,
            },
        )?),
        None => pane_response(request(
            workspace,
            Request::Layout {
                columns,
                rows,
                command,
            },
        )?),
    }
}

#[doc(hidden)]
pub fn logs_window(workspace: &str, window: String, ansi: bool) -> Result<Vec<u8>> {
    request(workspace, Request::LogsWindow { name: window, ansi })?
        .logs
        .ok_or_else(|| anyhow::anyhow!("session did not return logs"))
}

#[doc(hidden)]
pub fn move_workspace_pane(
    workspace: &str,
    pane: crate::workspace::PaneId,
    window: String,
    vertical: bool,
) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(
        workspace,
        Request::MovePane {
            pane,
            window,
            vertical,
        },
    )?)
}

#[doc(hidden)]
pub fn resize_workspace_pane(
    workspace: &str,
    pane: crate::workspace::PaneId,
    direction: PaneDirection,
    cells: u16,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(
        workspace,
        Request::ResizePane {
            pane,
            direction,
            cells,
        },
    )?)
}

#[doc(hidden)]
pub fn toggle_workspace_zoom(
    workspace: &str,
    pane: crate::workspace::PaneId,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(workspace, Request::ToggleZoom { pane })?)
}

fn window_response(response: Response) -> Result<Vec<crate::workspace::WindowStatus>> {
    response
        .windows
        .ok_or_else(|| anyhow::anyhow!("session did not return windows"))
}

#[doc(hidden)]
pub fn create_workspace_window(
    workspace: &str,
    name: Option<String>,
    command: Vec<String>,
    cwd: Option<PathBuf>,
) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(
        workspace,
        Request::CreateWindow { name, command, cwd },
    )?)
}

#[doc(hidden)]
pub fn select_workspace_window(
    workspace: &str,
    name: String,
) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(workspace, Request::SelectWindow { name })?)
}

#[doc(hidden)]
pub fn rename_workspace_window(
    workspace: &str,
    name: String,
    new_name: String,
) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(
        workspace,
        Request::RenameWindow { name, new_name },
    )?)
}

#[doc(hidden)]
pub fn close_workspace_window(
    workspace: &str,
    name: String,
) -> Result<Vec<crate::workspace::WindowStatus>> {
    window_response(request(workspace, Request::CloseWindow { name })?)
}

fn pane_response(response: Response) -> Result<Vec<crate::workspace::PaneStatus>> {
    response
        .panes
        .ok_or_else(|| anyhow::anyhow!("session did not return panes"))
}

#[doc(hidden)]
pub fn set_workspace_layout(
    name: &str,
    columns: u16,
    rows: u16,
    command: Vec<String>,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(
        name,
        Request::Layout {
            columns,
            rows,
            command,
        },
    )?)
}

#[doc(hidden)]
pub fn focus_workspace_pane(
    name: &str,
    pane: crate::workspace::PaneId,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(name, Request::FocusPane { pane })?)
}

#[doc(hidden)]
pub fn close_workspace_pane(
    name: &str,
    pane: crate::workspace::PaneId,
) -> Result<Vec<crate::workspace::PaneStatus>> {
    pane_response(request(name, Request::ClosePane { pane })?)
}

#[doc(hidden)]
pub fn resize(
    name: &str,
    cols: u16,
    rows: u16,
    cell_width: Option<u16>,
    cell_height: Option<u16>,
) -> Result<()> {
    request(
        name,
        Request::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
        },
    )?;
    Ok(())
}

#[doc(hidden)]
pub fn mark(name: &str, marker: String) -> Result<()> {
    request(name, Request::Mark { name: marker })?;
    Ok(())
}

#[doc(hidden)]
pub fn logs(name: &str, ansi: bool) -> Result<Vec<u8>> {
    request(name, Request::Logs { ansi })?
        .logs
        .ok_or_else(|| anyhow::anyhow!("session did not return logs"))
}

#[doc(hidden)]
pub fn list() -> Result<Vec<NamedSessionStatus>> {
    implementation::list()
}

#[doc(hidden)]
pub fn stop(name: &str) -> Result<()> {
    request(name, Request::Stop)?;
    Ok(())
}

#[doc(hidden)]
pub fn prune(name: &str, dry_run: bool) -> Result<Option<PruneKind>> {
    validate_name(name)?;
    implementation::prune(name, dry_run)
}

#[doc(hidden)]
pub fn serve(
    name: String,
    socket: PathBuf,
    command: Vec<String>,
    cwd: Option<PathBuf>,
    record: Option<PathBuf>,
    options: Options,
) -> Result<()> {
    implementation::serve(name, socket, command, cwd, record, options)
}

#[doc(hidden)]
pub fn serve_workspace(
    name: String,
    socket: PathBuf,
    command: Vec<String>,
    cwd: Option<PathBuf>,
    record: Option<PathBuf>,
    options: Options,
    tab_position: TabPosition,
) -> Result<()> {
    implementation::serve_workspace(name, socket, command, cwd, record, options, tab_position)
}

/// Run a named session in the foreground, mirrored through the current terminal.
pub fn run_foreground(
    name: &str,
    command: &[String],
    cwd: Option<&Path>,
    record: Option<&Path>,
    options: &Options,
    tab_position: TabPosition,
) -> Result<()> {
    validate_name(name)?;
    implementation::run_foreground(name, command, cwd, record, options, tab_position)
}

/// Attach the current terminal to an existing named workspace.
pub fn attach(name: &str, options: &Options) -> Result<()> {
    validate_name(name)?;
    implementation::attach(socket_path(name)?, name, options)
}

#[doc(hidden)]
pub fn infer_name(command: &[String]) -> Result<String> {
    let executable = command.first().ok_or_else(|| {
        anyhow::anyhow!("cannot infer a session name: provide a command after --")
    })?;
    let name = Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot infer a session name from executable {executable:?}: basename is empty or invalid"
            )
        })?;
    validate_name(name).map_err(|error| {
        anyhow::anyhow!(
            "cannot infer a session name from executable {executable:?}: basename {name:?} is invalid: {error}"
        )
    })?;
    Ok(name.to_owned())
}

fn request(name: &str, operation: Request) -> Result<Response> {
    validate_name(name)?;
    let requirements = operation.requirements();
    let response = if requirements.hold_name_lock {
        implementation::request_layout_command(name, &operation)?
    } else {
        implementation::request(socket_path(name)?, &operation)?
    };
    if let Some(capability) = requirements.capability {
        capability.require(&response)?;
    }
    if let Some(error) = response.error {
        bail!(error);
    }
    Ok(response)
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_name(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("session names may contain only ASCII letters, digits, '.', '-', and '_'");
    }
    Ok(())
}

fn socket_path(name: &str) -> Result<PathBuf> {
    Ok(implementation::runtime_dir()?.join(format!("{name}.sock")))
}

#[cfg(unix)]
mod implementation {
    use std::fs;
    use std::fs::OpenOptions;
    use std::io::{ErrorKind, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use anyhow::{Context, Result, bail};

    use super::{
        ATTACHED_TERMINAL_ERROR, NamedSessionStatus, ProtocolCapability, PruneKind, Request,
        Response, SemanticSnapshotResult, Session, SessionState, TabPosition, UnavailableReason,
    };
    use crate::shot::{self, Options};
    use crate::workspace::{Workspace, WorkspaceAttachmentOptions, WorkspaceTerminal};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
    const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_CONTROL_DURATION_MS: u64 = 10 * 60 * 1000;
    const ATTACHED_WORKSPACE_POLL: Duration = Duration::from_millis(16);
    const DETACHED_WORKSPACE_POLL: Duration = Duration::from_millis(50);
    const DETACHED_WORKSPACE_ACTIVE: Duration = Duration::from_millis(500);
    static NEXT_ATTACHMENT_ID: AtomicU64 = AtomicU64::new(1);

    struct StartLock(fs::File);

    struct AttachmentWriter {
        stream: UnixStream,
        cleanup: Option<Box<dyn FnOnce() + Send>>,
    }

    struct AttachmentEndpoint {
        id: u64,
        path: PathBuf,
        listener: UnixListener,
    }

    impl AttachmentEndpoint {
        fn bind(name: &str) -> Result<Self> {
            let runtime = runtime_dir()?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let id = now
                ^ (u64::from(std::process::id()) << 32)
                ^ NEXT_ATTACHMENT_ID.fetch_add(1, Ordering::Relaxed);
            let path = runtime.join(format!("{name}.attach-{id:016x}"));
            ensure_socket_path(&path)?;
            let listener = UnixListener::bind(&path)
                .with_context(|| format!("bind attachment socket {}", path.display()))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("secure attachment socket {}", path.display()))?;
            listener
                .set_nonblocking(true)
                .context("set attachment socket nonblocking")?;
            Ok(Self { id, path, listener })
        }

        fn accept(&self, deadline: Instant) -> Result<UnixStream> {
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        let _ = fs::remove_file(&self.path);
                        stream
                            .set_nonblocking(false)
                            .context("set workspace attachment blocking")?;
                        return Ok(stream);
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            bail!("timed out waiting for workspace attachment");
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error).context("accept workspace attachment"),
                }
            }
        }
    }

    impl Drop for AttachmentEndpoint {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    impl Write for AttachmentWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.stream.write(bytes)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.stream.flush()
        }
    }

    impl Drop for AttachmentWriter {
        fn drop(&mut self) {
            let _ = self.stream.shutdown(std::net::Shutdown::Write);
            if let Some(cleanup) = self.cleanup.take() {
                cleanup();
            }
        }
    }

    impl StartLock {
        fn acquire(path: &Path) -> Result<Self> {
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)
                .with_context(|| format!("open {}", path.display()))?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                bail!("another session operation is already starting this name");
            }
            Ok(Self(file))
        }
    }

    impl Drop for StartLock {
        fn drop(&mut self) {
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
    pub fn runtime_dir() -> Result<PathBuf> {
        crate::runtime::directory()
    }

    pub fn start(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
    ) -> Result<()> {
        if command.is_empty() {
            bail!("provide a command after --");
        }
        let runtime = runtime_dir()?;
        ensure_socket_path(&runtime.join(format!("{name}.sock")))?;
        let _lock = StartLock::acquire(&runtime.join(format!("{name}.lock")))?;
        start_locked(name, command, cwd, record, options, &runtime)
    }

    pub fn restart(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
    ) -> Result<()> {
        if command.is_empty() {
            bail!("provide a command after --");
        }
        let runtime = runtime_dir()?;
        ensure_socket_path(&runtime.join(format!("{name}.sock")))?;
        let _lock = StartLock::acquire(&runtime.join(format!("{name}.lock")))?;
        let socket = runtime.join(format!("{name}.sock"));
        if let Ok(response) = request(socket.clone(), &Request::Stop) {
            if let Some(error) = response.error {
                bail!(error);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while request(socket.clone(), &Request::Ping).is_ok() {
                if Instant::now() >= deadline {
                    bail!("timed out stopping session {name:?} before restart");
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        start_locked(name, command, cwd, record, options, &runtime)
    }

    fn start_locked(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        runtime: &Path,
    ) -> Result<()> {
        let socket = runtime.join(format!("{name}.sock"));
        if socket.exists() {
            if request(socket.clone(), &Request::Ping).is_ok() {
                bail!("session {name:?} is already running");
            }
            match fs::remove_file(&socket) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("remove stale {}", socket.display()));
                }
            }
        }
        let mut daemon =
            Command::new(std::env::current_exe().context("locate termctrl executable")?);
        daemon
            .arg("__serve")
            .arg("--name")
            .arg(name)
            .arg("--socket")
            .arg(&socket)
            .arg("--cols")
            .arg(options.cols.to_string())
            .arg("--rows")
            .arg(options.rows.to_string())
            .arg("--cell-width")
            .arg(options.cell_width.to_string())
            .arg("--cell-height")
            .arg(options.cell_height.to_string())
            .arg("--max-bytes")
            .arg(options.max_bytes.to_string());
        daemon
            .env_remove("TERMCTRL_WORKSPACE")
            .env_remove("TERMCTRL_PANE_ID")
            .env_remove("TERMCTRL_LAUNCH_WINDOW_ID")
            .env("TERMCTRL_SESSION", name);
        if options.opentui_host {
            daemon.arg("--opentui-host");
        }
        match options.color {
            shot::ColorMode::Auto => {}
            shot::ColorMode::Always => {
                daemon.arg("--color").arg("always");
            }
            shot::ColorMode::Never => {
                daemon.arg("--color").arg("never");
            }
        }
        if let Some(cwd) = cwd {
            daemon.arg("--cwd").arg(cwd);
        }
        if let Some(record) = record {
            let record = if record.is_absolute() {
                record.to_owned()
            } else {
                std::env::current_dir()
                    .context("resolve recording output directory")?
                    .join(record)
            };
            daemon.arg("--record").arg(record);
        }
        daemon
            .arg("--")
            .args(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut daemon = daemon.spawn().context("start session daemon")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if request(socket.clone(), &Request::Ping).is_ok() {
                return Ok(());
            }
            if let Some(status) = daemon.try_wait().context("poll session daemon")? {
                bail!("session daemon exited before becoming ready: {status}");
            }
            if Instant::now() >= deadline {
                let _ = daemon.kill();
                bail!("timed out starting session {name:?}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn request(socket: PathBuf, request: &Request) -> Result<Response> {
        request_with_timeout(socket, request, None)
    }

    pub fn request_layout_command(name: &str, operation: &Request) -> Result<Response> {
        let runtime = runtime_dir()?;
        request_layout_command_in(&runtime, name, operation)
    }

    fn request_layout_command_in(
        runtime: &Path,
        name: &str,
        operation: &Request,
    ) -> Result<Response> {
        let requirements = operation.requirements();
        if !requirements.hold_name_lock {
            bail!("request does not require the session name lock");
        }
        let _lock = StartLock::acquire(&runtime.join(format!("{name}.lock")))?;
        let socket = runtime.join(format!("{name}.sock"));
        let response = request(socket.clone(), &Request::Ping)?;
        if let Some(error) = response.error {
            bail!(error);
        }
        requirements
            .capability
            .context("locked request has no protocol capability")?
            .require(&response)?;
        request(socket, operation)
    }

    fn request_with_timeout(
        socket: PathBuf,
        request: &Request,
        timeout: Option<Duration>,
    ) -> Result<Response> {
        ensure_socket_path(&socket)?;
        let mut stream = match timeout {
            Some(timeout) => connect_with_timeout(&socket, timeout),
            None => UnixStream::connect(&socket),
        }
        .with_context(|| format!("connect to session at {}; is it running?", socket.display()))?;
        stream
            .set_read_timeout(timeout)
            .context("bound session response")?;
        stream
            .set_write_timeout(timeout)
            .context("bound session request")?;
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, &mut stream);
        serde_json::to_writer(&mut writer, request).context("write session request")?;
        writer.flush().context("flush session request")?;
        drop(writer);
        stream
            .shutdown(std::net::Shutdown::Write)
            .context("finish session request")?;
        serde_json::from_reader(std::io::BufReader::with_capacity(64 * 1024, stream))
            .context("read session response")
    }

    fn connect_with_timeout(path: &Path, timeout: Duration) -> std::io::Result<UnixStream> {
        let path = path.as_os_str().as_bytes();
        if path.contains(&0) {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "Unix socket path contains a null byte",
            ));
        }
        let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
        if path.len() >= address.sun_path.len() {
            return Err(std::io::Error::new(
                ErrorKind::InvalidInput,
                "Unix socket path is too long",
            ));
        }
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (target, byte) in address.sun_path.iter_mut().zip(path.iter().copied()) {
            *target = byte as libc::c_char;
        }
        let address_len = std::mem::offset_of!(libc::sockaddr_un, sun_path) + path.len() + 1;
        #[cfg(any(
            target_os = "aix",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            address.sun_len = u8::try_from(address_len).unwrap_or(u8::MAX);
        }
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let connected = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                (&raw const address).cast(),
                address_len as libc::socklen_t,
            )
        };
        if connected < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINPROGRESS) {
                return Err(error);
            }
            let deadline = Instant::now() + timeout;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(std::io::Error::new(
                        ErrorKind::TimedOut,
                        "timed out connecting to Unix socket",
                    ));
                }
                let mut poll = libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
                let ready = unsafe { libc::poll(&mut poll, 1, timeout_ms.max(1)) };
                if ready > 0 {
                    break;
                }
                if ready == 0 {
                    return Err(std::io::Error::new(
                        ErrorKind::TimedOut,
                        "timed out connecting to Unix socket",
                    ));
                }
                if std::io::Error::last_os_error().kind() != ErrorKind::Interrupted {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let mut socket_error = 0;
            let mut error_len = std::mem::size_of_val(&socket_error) as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&raw mut socket_error).cast(),
                    &mut error_len,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if socket_error != 0 {
                return Err(std::io::Error::from_raw_os_error(socket_error));
            }
        }
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(UnixStream::from(fd))
    }

    pub fn attach(socket: PathBuf, name: &str, options: &Options) -> Result<()> {
        let timing = std::env::var_os("TERMCTRL_ATTACH_TIMING").is_some();
        let attach_started = Instant::now();
        let mark = move |label: &str| {
            if timing {
                eprintln!(
                    "TIMING attach {label}={:.1}ms",
                    attach_started.elapsed().as_secs_f64() * 1_000.0
                );
            }
        };
        ensure_socket_path(&socket)?;
        require_attachment_terminal()?;
        let raw = RawMode::enter()?;
        let (theme, retained_input) = crate::terminal_theme::discover();
        mark("theme");
        let endpoint = AttachmentEndpoint::bind(name)?;
        let result = (|| {
            let response = request(
                socket.clone(),
                &Request::Attach {
                    id: endpoint.id,
                    socket: endpoint.path.clone(),
                    cols: options.cols,
                    rows: options.rows,
                    cell_width: options.cell_width,
                    cell_height: options.cell_height,
                    theme,
                },
            )?;
            ProtocolCapability::Attachment.require(&response)?;
            if let Some(error) = response.error {
                return Err(super::attachment_rejection(name, &error));
            }
            mark("attach-request");
            let mut stream = endpoint.accept(Instant::now() + Duration::from_secs(5))?;
            mark("accepted");
            stream
                .set_write_timeout(Some(Duration::from_millis(250)))
                .context("bound workspace attachment input")?;
            if !retained_input.is_empty() {
                stream
                    .write_all(&retained_input)
                    .context("forward input retained during theme discovery")?;
            }
            let resize_running = Arc::new(AtomicBool::new(true));
            let resize_flag = Arc::clone(&resize_running);
            let resize_socket = socket.clone();
            let mut last_size = (options.cols, options.rows);
            let mut uncertain = false;
            let mut retry_after = Instant::now();
            let cell_width = options.cell_width;
            let cell_height = options.cell_height;
            let resize = thread::spawn(move || {
                while resize_flag.load(Ordering::Relaxed) {
                    if let Ok((cols, rows)) = crossterm::terminal::size()
                        && super::valid_workspace_attachment_size(cols, rows)
                        && ((cols, rows) != last_size || uncertain)
                        && Instant::now() >= retry_after
                    {
                        match request_with_timeout(
                            resize_socket.clone(),
                            &Request::ResizeAttachment {
                                id: endpoint.id,
                                cols,
                                rows,
                                cell_width,
                                cell_height,
                            },
                            Some(Duration::from_millis(250)),
                        ) {
                            Ok(response) if response.error.is_none() => {
                                last_size = (cols, rows);
                                uncertain = false;
                            }
                            Ok(_) | Err(_) => {
                                uncertain = true;
                                retry_after = Instant::now() + Duration::from_secs(1);
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
            let _resize_loop = ResizeLoop {
                running: resize_running,
                thread: Some(resize),
            };
            let _screen = AttachedTerminal;
            relay_attachment(&mut stream)?;
            Ok(())
        })();
        drop(raw);
        result
    }

    fn relay_attachment(stream: &mut UnixStream) -> Result<()> {
        let timing = std::env::var_os("TERMCTRL_ATTACH_TIMING").is_some();
        let relay_started = Instant::now();
        let mut first_output = true;
        let stream_fd = stream.as_raw_fd();
        let max_fd = stream_fd.max(libc::STDIN_FILENO) + 1;
        let mut stdout = std::io::stdout().lock();
        let mut bytes = [0_u8; 16 * 1024];
        loop {
            let mut read_fds = unsafe { std::mem::zeroed::<libc::fd_set>() };
            unsafe {
                libc::FD_ZERO(&mut read_fds);
                libc::FD_SET(libc::STDIN_FILENO, &mut read_fds);
                libc::FD_SET(stream_fd, &mut read_fds);
            }
            let ready = unsafe {
                libc::pselect(
                    max_fd,
                    &mut read_fds,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if ready < 0 {
                if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(std::io::Error::last_os_error()).context("wait for attachment I/O");
            }
            if unsafe { libc::FD_ISSET(stream_fd, &read_fds) } {
                match stream.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(length) => {
                        if timing && std::mem::take(&mut first_output) {
                            eprintln!(
                                "TIMING attach first-output={:.1}ms",
                                relay_started.elapsed().as_secs_f64() * 1_000.0
                            );
                        }
                        stdout
                            .write_all(&bytes[..length])
                            .context("write attached workspace output")?;
                        stdout.flush().context("flush attached workspace output")?;
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => {}
                    Err(error) => return Err(error).context("read attached workspace output"),
                }
            }
            if unsafe { libc::FD_ISSET(libc::STDIN_FILENO, &read_fds) } {
                let length = unsafe {
                    libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), bytes.len())
                };
                if length == 0 {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    break;
                }
                if length < 0 {
                    if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(std::io::Error::last_os_error()).context("read attachment input");
                }
                stream
                    .write_all(&bytes[..usize::try_from(length).unwrap_or(0)])
                    .context("send attachment input")?;
                stream.flush().context("flush attachment input")?;
            }
        }
        Ok(())
    }

    struct AttachedTerminal;

    struct ResizeLoop {
        running: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Drop for ResizeLoop {
        fn drop(&mut self) {
            self.running.store(false, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    impl Drop for AttachedTerminal {
        fn drop(&mut self) {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(
                b"\x1b[?2026l\x1b[?1l\x1b>\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[0 q\x1b[0m\x1b[?25h\x1b[?1049l\x1b[23;0t",
            );
            let _ = stdout.flush();
        }
    }

    pub fn list() -> Result<Vec<NamedSessionStatus>> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(runtime_dir()?).context("read session runtime directory")? {
            let path = entry.context("read session runtime entry")?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("sock") {
                continue;
            }
            let Some(name) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let (status, error, unavailable) = match request(path, &Request::Status) {
                Ok(response) => (response.status, response.error, None),
                Err(error) => {
                    let reason = if stale_socket_error(&error) {
                        UnavailableReason::Stale
                    } else {
                        UnavailableReason::IncompatibleProtocol
                    };
                    (None, Some(format!("{error:#}")), Some(reason))
                }
            };
            sessions.push(NamedSessionStatus {
                name,
                status,
                error,
                unavailable,
            });
        }
        sessions.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(sessions)
    }

    pub fn prune(name: &str, dry_run: bool) -> Result<Option<PruneKind>> {
        let runtime = runtime_dir()?;
        prune_in(&runtime, name, dry_run)
    }

    fn prune_in(runtime: &Path, name: &str, dry_run: bool) -> Result<Option<PruneKind>> {
        let _lock = StartLock::acquire(&runtime.join(format!("{name}.lock")))?;
        let socket = runtime.join(format!("{name}.sock"));
        ensure_socket_path(&socket)?;
        let metadata = match fs::symlink_metadata(&socket) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect session socket {}", socket.display()));
            }
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            bail!(
                "refusing to prune untrusted session socket {}",
                socket.display()
            );
        }
        match request(socket.clone(), &Request::Status) {
            Ok(response) => {
                if let Some(error) = response.error {
                    bail!(error);
                }
                let status = response
                    .status
                    .context("session did not return status while pruning")?;
                if status.state != SessionState::Exited {
                    return Ok(None);
                }
                if !dry_run {
                    let response = request(socket, &Request::Stop)?;
                    if let Some(error) = response.error {
                        bail!(error);
                    }
                }
                Ok(Some(PruneKind::Exited))
            }
            Err(error) if stale_socket_error(&error) => {
                if !dry_run {
                    fs::remove_file(&socket).with_context(|| {
                        format!("remove stale session socket {}", socket.display())
                    })?;
                }
                Ok(Some(PruneKind::Stale))
            }
            Err(error) => Err(error).context("refusing to prune an unresponsive session"),
        }
    }

    fn stale_socket_error(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    ErrorKind::ConnectionRefused | ErrorKind::NotFound
                )
            })
        })
    }

    pub fn serve(
        name: String,
        socket: PathBuf,
        command: Vec<String>,
        cwd: Option<PathBuf>,
        record: Option<PathBuf>,
        mut options: Options,
    ) -> Result<()> {
        ensure_socket_path(&socket)?;
        if command.is_empty() {
            bail!("provide a command after --");
        }
        options.env.insert("TERMCTRL_SESSION".to_owned(), name);
        let result = (|| {
            let listener = UnixListener::bind(&socket)
                .with_context(|| format!("bind {}", socket.display()))?;
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("secure {}", socket.display()))?;
            listener
                .set_nonblocking(true)
                .context("set session socket nonblocking")?;
            let mut session =
                Session::start(&command, cwd.as_deref(), record.as_deref(), &options)?;
            let result = run(&listener, &mut session);
            let _ = session.stop();
            result
        })();
        let _ = fs::remove_file(&socket);
        result
    }

    pub fn serve_workspace(
        name: String,
        socket: PathBuf,
        command: Vec<String>,
        cwd: Option<PathBuf>,
        record: Option<PathBuf>,
        options: Options,
        tab_position: TabPosition,
    ) -> Result<()> {
        ensure_socket_path(&socket)?;
        let result = (|| {
            let listener = UnixListener::bind(&socket)
                .with_context(|| format!("bind {}", socket.display()))?;
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("secure {}", socket.display()))?;
            listener
                .set_nonblocking(true)
                .context("set workspace socket nonblocking")?;
            let mut workspace = Workspace::start_named_with_theme(
                &name,
                &command,
                cwd.as_deref(),
                record.as_deref(),
                &options,
                crate::terminal_theme::TerminalTheme::default(),
                tab_position,
            )?;
            let mut terminal = WorkspaceTerminal::detached();
            let mut active_until = Instant::now();
            let run_result = (|| {
                'workspace: loop {
                    let running = terminal.tick(&mut workspace)?;
                    loop {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                let finished =
                                    handle_workspace(stream, &mut workspace, &mut terminal)?;
                                active_until = Instant::now() + DETACHED_WORKSPACE_ACTIVE;
                                if finished {
                                    break 'workspace;
                                }
                            }
                            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                            Err(error) => return Err(error).context("accept workspace request"),
                        }
                    }
                    if !running || terminal.finished() {
                        break;
                    }
                    if terminal.is_attached() {
                        thread::sleep(ATTACHED_WORKSPACE_POLL);
                    } else if Instant::now() < active_until {
                        thread::sleep(Duration::from_millis(5));
                    } else {
                        wait_for_workspace_request(&listener, DETACHED_WORKSPACE_POLL)?;
                    }
                }
                Ok(())
            })();
            let stop_result = workspace.try_stop();
            match (run_result, stop_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(stop_error)) => Err(error).context(format!(
                    "workspace finalization also failed: {stop_error:#}"
                )),
            }
        })();
        let _ = fs::remove_file(&socket);
        result
    }

    fn wait_for_workspace_request(listener: &UnixListener, timeout: Duration) -> Result<()> {
        let mut descriptor = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        loop {
            let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
            if result >= 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != ErrorKind::Interrupted {
                return Err(error).context("wait for detached workspace request");
            }
        }
    }

    struct RawMode;

    impl RawMode {
        fn enter() -> Result<Self> {
            enable_raw_mode().context("enable terminal raw mode")?;
            Ok(Self)
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
        }
    }

    pub fn run_foreground(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        tab_position: TabPosition,
    ) -> Result<()> {
        require_attachment_terminal()?;
        let runtime = runtime_dir()?;
        let socket = runtime.join(format!("{name}.sock"));
        ensure_socket_path(&socket)?;
        let _lock = StartLock::acquire(&runtime.join(format!("{name}.lock")))?;
        if socket.exists() {
            if request(socket.clone(), &Request::Ping).is_ok() {
                if !command.is_empty() {
                    bail!("workspace {name:?} already exists; omit the command to attach");
                }
                drop(_lock);
                return attach(socket, name, options);
            }
            fs::remove_file(&socket)
                .with_context(|| format!("remove stale {}", socket.display()))?;
        }
        spawn_workspace(name, command, cwd, record, options, tab_position, &socket)?;
        drop(_lock);
        attach(socket, name, options)
    }

    fn require_attachment_terminal() -> Result<()> {
        if unsafe { libc::isatty(libc::STDIN_FILENO) != 1 }
            || unsafe { libc::isatty(libc::STDOUT_FILENO) != 1 }
        {
            bail!("workspace attachment requires terminal stdin and stdout");
        }
        Ok(())
    }

    fn spawn_workspace(
        name: &str,
        command: &[String],
        cwd: Option<&Path>,
        record: Option<&Path>,
        options: &Options,
        tab_position: TabPosition,
        socket: &Path,
    ) -> Result<()> {
        let mut daemon =
            Command::new(std::env::current_exe().context("locate termctrl executable")?);
        daemon
            .arg("__serve-workspace")
            .arg("--name")
            .arg(name)
            .arg("--socket")
            .arg(socket)
            .arg("--cols")
            .arg(options.cols.to_string())
            .arg("--rows")
            .arg(options.rows.to_string())
            .arg("--cell-width")
            .arg(options.cell_width.to_string())
            .arg("--cell-height")
            .arg(options.cell_height.to_string())
            .arg("--max-bytes")
            .arg(options.max_bytes.to_string())
            .arg("--tab-position")
            .arg(tab_position.as_str());
        if options.opentui_host {
            daemon.arg("--opentui-host");
        }
        match options.color {
            shot::ColorMode::Auto => {}
            shot::ColorMode::Always => {
                daemon.arg("--color").arg("always");
            }
            shot::ColorMode::Never => {
                daemon.arg("--color").arg("never");
            }
        }
        if let Some(cwd) = cwd {
            daemon.arg("--cwd").arg(cwd);
        }
        if let Some(record) = record {
            let record = if record.is_absolute() {
                record.to_owned()
            } else {
                std::env::current_dir()
                    .context("resolve recording output directory")?
                    .join(record)
            };
            daemon.arg("--record").arg(record);
        }
        if !command.is_empty() {
            daemon.arg("--").args(command);
        }
        unsafe {
            daemon.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        daemon
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut daemon = daemon.spawn().context("start workspace daemon")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if request(socket.to_owned(), &Request::Ping).is_ok() {
                return Ok(());
            }
            if let Some(status) = daemon.try_wait().context("poll workspace daemon")? {
                bail!("workspace daemon for {name:?} exited before becoming ready: {status}");
            }
            if Instant::now() >= deadline {
                let _ = daemon.kill();
                bail!("timed out starting workspace {name:?}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn ensure_socket_path(path: &Path) -> Result<()> {
        if path.as_os_str().as_encoded_bytes().len() >= 100 {
            bail!(
                "session socket path is too long for portable Unix sockets: {}; set TERMCTRL_RUNTIME_DIR to a shorter directory",
                path.display()
            );
        }
        Ok(())
    }

    fn run(listener: &UnixListener, session: &mut Session) -> Result<()> {
        let timing = std::env::var_os("TERMCTRL_SERVE_TIMING").map(PathBuf::from);
        let log = |line: String| {
            if let Some(path) = &timing
                && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        };
        let serve_started = Instant::now();
        loop {
            // Keep parsing and recording output even when no control request is in flight.
            let phase_started = Instant::now();
            session.consume_batch()?;
            if phase_started.elapsed() > Duration::from_millis(50) {
                log(format!(
                    "TIMING serve consume at={:.0}ms took={:.0}ms",
                    serve_started.elapsed().as_secs_f64() * 1_000.0,
                    phase_started.elapsed().as_secs_f64() * 1_000.0,
                ));
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let handle_started = Instant::now();
                    let stopped = handle(stream, session)?;
                    log(format!(
                        "TIMING serve handle at={:.0}ms took={:.0}ms",
                        serve_started.elapsed().as_secs_f64() * 1_000.0,
                        handle_started.elapsed().as_secs_f64() * 1_000.0,
                    ));
                    if stopped {
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error).context("accept session request"),
            }
        }
    }

    fn handle(mut stream: UnixStream, session: &mut Session) -> Result<bool> {
        handle_control(&mut stream, "session", |request| {
            let stop = matches!(request, Request::Stop);
            let response = respond(session, request)
                .unwrap_or_else(|error| Response::error(format!("{error:#}")));
            (response, stop)
        })
    }

    fn handle_workspace(
        mut stream: UnixStream,
        workspace: &mut Workspace,
        terminal: &mut WorkspaceTerminal,
    ) -> Result<bool> {
        handle_control(&mut stream, "workspace", |request| {
            let stop = matches!(request, Request::Stop);
            let response = respond_workspace(workspace, request, terminal)
                .unwrap_or_else(|error| Response::error(format!("{error:#}")));
            let finished = response.error.is_none() && (stop || workspace.is_empty());
            (response, finished)
        })
    }

    fn handle_control(
        stream: &mut UnixStream,
        subject: &str,
        mut dispatch: impl FnMut(Request) -> (Response, bool),
    ) -> Result<bool> {
        let timing = std::env::var_os("TERMCTRL_SERVE_TIMING").map(PathBuf::from);
        let log = |line: String| {
            if let Some(path) = &timing
                && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        };
        let handle_started = Instant::now();
        stream
            .set_nonblocking(false)
            .with_context(|| format!("set {subject} connection blocking"))?;
        stream
            .set_read_timeout(Some(CONTROL_TIMEOUT))
            .with_context(|| format!("set {subject} request timeout"))?;
        stream
            .set_write_timeout(Some(CONTROL_TIMEOUT))
            .with_context(|| format!("set {subject} response timeout"))?;
        let mut bytes = Vec::new();
        let response = match Read::by_ref(stream)
            .take(MAX_REQUEST_BYTES + 1)
            .read_to_end(&mut bytes)
        {
            Ok(_) if bytes.len() as u64 > MAX_REQUEST_BYTES => {
                Response::error(format!("{subject} request exceeds 1 MiB"))
            }
            Ok(_) => match serde_json::from_slice::<Request>(&bytes) {
                Ok(request) => {
                    log(format!(
                        "TIMING control read={:.0}ms",
                        handle_started.elapsed().as_secs_f64() * 1_000.0
                    ));
                    let dispatch_started = Instant::now();
                    let (response, finished) = dispatch(request);
                    log(format!(
                        "TIMING control dispatch={:.0}ms",
                        dispatch_started.elapsed().as_secs_f64() * 1_000.0
                    ));
                    let write_started = Instant::now();
                    let written = write_response(stream, &response).is_ok();
                    log(format!(
                        "TIMING control write={:.0}ms",
                        write_started.elapsed().as_secs_f64() * 1_000.0
                    ));
                    return Ok(written && finished);
                }
                Err(error) => Response::error(format!("invalid {subject} request: {error}")),
            },
            Err(error) => Response::error(format!("failed to read {subject} request: {error}")),
        };
        let _ = write_response(stream, &response);
        Ok(false)
    }

    fn write_response(stream: &mut UnixStream, response: &Response) -> Result<()> {
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, &mut *stream);
        serde_json::to_writer(&mut writer, response).context("write session response")?;
        writer.flush().context("flush session response")
    }

    fn respond(session: &mut Session, request: Request) -> Result<Response> {
        let mut response = Response::default();
        match request {
            Request::Ping => {}
            Request::Status => response.status = Some(session.status()?),
            Request::Send {
                input,
                pace_ms,
                pane,
            } => {
                if pane.is_some_and(|pane| pane != 0) {
                    bail!("single sessions only contain pane 0");
                }
                session.send_all(&input, Duration::from_millis(pace_ms))?;
            }
            Request::Wait {
                text,
                timeout_ms,
                pane,
            } => {
                if pane.is_some_and(|pane| pane != 0) {
                    bail!("single sessions only contain pane 0");
                }
                session.wait_for_text(&text, Duration::from_millis(timeout_ms))?;
            }
            Request::Show {
                settle_ms,
                deadline_ms,
                pane,
            } => {
                if pane.is_some_and(|pane| pane != 0) {
                    bail!("single sessions only contain pane 0");
                }
                response.captured = Some(
                    session
                        .capture(
                            Duration::from_millis(settle_ms),
                            Duration::from_millis(deadline_ms),
                        )?
                        .shot,
                );
                response.status = Some(session.status()?);
            }
            Request::Logs { ansi } => response.logs = Some(session.logs(ansi)?),
            Request::Resize {
                cols,
                rows,
                cell_width,
                cell_height,
            } => {
                let status = session.status()?;
                session.resize(
                    cols,
                    rows,
                    cell_width.unwrap_or(status.cell_width),
                    cell_height.unwrap_or(status.cell_height),
                )?;
            }
            Request::Mark { name } => session.mark(&name)?,
            Request::SemanticSnapshot {
                deadline_unix_ms,
                pane,
                window,
            } => {
                if window.is_some() {
                    bail!("only workspaces support named windows");
                }
                if pane.is_some_and(|pane| pane != 0) {
                    bail!("single sessions only contain pane 0");
                }
                response.semantic_snapshot = Some(SemanticSnapshotResult {
                    value: session
                        .semantic_snapshot(crate::semantic::remaining(deadline_unix_ms)?)?,
                });
            }
            Request::Windows
            | Request::WorkspaceContext { .. }
            | Request::SetTabPosition { .. }
            | Request::MoveWindow { .. }
            | Request::CreateWindow { .. }
            | Request::SelectWindow { .. }
            | Request::RenameWindow { .. }
            | Request::CloseWindow { .. }
            | Request::WindowPanes { .. }
            | Request::WindowLayout { .. }
            | Request::ShowWindow { .. }
            | Request::SendWindow { .. }
            | Request::WaitWindow { .. }
            | Request::LogsWindow { .. }
            | Request::MovePane { .. } => {
                bail!("only workspaces support named windows")
            }
            Request::Panes => {
                let status = session.status()?;
                response.panes = Some(vec![crate::workspace::PaneStatus {
                    id: 0,
                    active: true,
                    visible: true,
                    state: status.state,
                    x: 0,
                    y: 0,
                    cols: status.cols,
                    rows: status.rows,
                    title: session.title()?,
                    command: status.launch.command,
                    cwd: status.launch.cwd,
                }]);
            }
            Request::Layout { .. }
            | Request::FocusPane { .. }
            | Request::ClosePane { .. }
            | Request::ResizePane { .. }
            | Request::ToggleZoom { .. }
            | Request::Attach { .. }
            | Request::ResizeAttachment { .. } => {
                bail!("only attached workspaces support pane layout control")
            }
            Request::Stop => session.stop()?,
        }
        Ok(response)
    }

    fn respond_workspace(
        workspace: &mut Workspace,
        request: Request,
        terminal: &mut WorkspaceTerminal,
    ) -> Result<Response> {
        let mut response = Response::default();
        match request {
            Request::Ping => {}
            Request::Status => response.status = Some(workspace.status()?),
            Request::Send {
                input,
                pace_ms,
                pane,
            } => {
                let duration = pace_ms.saturating_mul(input.len().saturating_sub(1) as u64);
                require_control_duration("paced input", duration)?;
                workspace.send_all(pane, &input, Duration::from_millis(pace_ms), |workspace| {
                    terminal.tick(workspace)
                })?;
            }
            Request::Wait {
                text,
                timeout_ms,
                pane,
            } => {
                require_control_duration("wait timeout", timeout_ms)?;
                workspace.wait_for_text(
                    pane,
                    &text,
                    Duration::from_millis(timeout_ms),
                    |workspace| terminal.tick(workspace),
                )?;
            }
            Request::Show {
                settle_ms,
                deadline_ms,
                pane,
            } => {
                require_control_duration("capture deadline", deadline_ms)?;
                response.captured = Some(workspace.capture(
                    pane,
                    Duration::from_millis(settle_ms),
                    Duration::from_millis(deadline_ms),
                    |workspace| terminal.tick(workspace),
                )?);
                response.status = Some(workspace.status()?);
            }
            Request::Logs { ansi } => response.logs = Some(workspace.active_logs(ansi)?),
            Request::Resize { .. } => {
                bail!("visible workspace dimensions are owned by the attached terminal")
            }
            Request::Mark { name } => workspace.mark_recording(&name)?,
            Request::SemanticSnapshot {
                deadline_unix_ms,
                pane,
                window,
            } => {
                response.semantic_snapshot = Some(SemanticSnapshotResult {
                    value: workspace.semantic_snapshot_in(
                        window.as_deref(),
                        pane,
                        crate::semantic::remaining(deadline_unix_ms)?,
                    )?,
                });
            }
            Request::Windows => response.windows = Some(workspace.windows()),
            Request::WorkspaceContext { pane } => {
                response.context = Some(workspace.context(pane)?);
            }
            Request::SetTabPosition { position } => {
                workspace.set_tab_position(position);
                response.windows = Some(workspace.windows());
            }
            Request::MoveWindow { name, index } => {
                workspace.move_window(&name, index)?;
                response.windows = Some(workspace.windows());
            }
            Request::CreateWindow { name, command, cwd } => {
                terminal.tick(workspace)?;
                workspace.create_window(name.as_deref(), &command, cwd.as_deref())?;
                response.windows = Some(workspace.windows());
            }
            Request::SelectWindow { name } => {
                terminal.tick(workspace)?;
                workspace.select_window(&name)?;
                response.windows = Some(workspace.windows());
            }
            Request::RenameWindow { name, new_name } => {
                workspace.rename_window(&name, &new_name)?;
                response.windows = Some(workspace.windows());
            }
            Request::CloseWindow { name } => {
                terminal.tick(workspace)?;
                workspace.close_window(&name)?;
                response.windows = Some(workspace.windows());
            }
            Request::WindowPanes { name } => {
                response.panes = Some(workspace.panes_in(Some(&name))?);
            }
            Request::WindowLayout {
                name,
                columns,
                rows,
                command,
            } => {
                terminal.tick(workspace)?;
                response.panes = Some(workspace.set_grid_in_with_command(
                    Some(&name),
                    columns,
                    rows,
                    Some(&command),
                )?);
            }
            Request::ShowWindow {
                name,
                settle_ms,
                deadline_ms,
            } => {
                require_control_duration("capture deadline", deadline_ms)?;
                response.captured = Some(workspace.capture_window(
                    &name,
                    Duration::from_millis(settle_ms),
                    Duration::from_millis(deadline_ms),
                    |workspace| terminal.tick(workspace),
                )?);
                response.status = Some(workspace.status()?);
            }
            Request::SendWindow {
                name,
                input,
                pace_ms,
            } => {
                let duration = pace_ms.saturating_mul(input.len().saturating_sub(1) as u64);
                require_control_duration("paced input", duration)?;
                workspace.send_all_in(
                    &name,
                    &input,
                    Duration::from_millis(pace_ms),
                    |workspace| terminal.tick(workspace),
                )?;
            }
            Request::WaitWindow {
                name,
                text,
                timeout_ms,
            } => {
                require_control_duration("wait timeout", timeout_ms)?;
                workspace.wait_for_text_in(
                    &name,
                    &text,
                    Duration::from_millis(timeout_ms),
                    |workspace| terminal.tick(workspace),
                )?;
            }
            Request::LogsWindow { name, ansi } => {
                response.logs = Some(workspace.logs_in(&name, ansi)?);
            }
            Request::MovePane {
                pane,
                window,
                vertical,
            } => {
                terminal.tick(workspace)?;
                workspace.move_pane(pane, &window, vertical)?;
                response.windows = Some(workspace.windows());
            }
            Request::ResizePane {
                pane,
                direction,
                cells,
            } => {
                terminal.tick(workspace)?;
                response.panes = Some(workspace.resize_pane(pane, direction, cells)?);
            }
            Request::ToggleZoom { pane } => {
                terminal.tick(workspace)?;
                response.panes = Some(workspace.toggle_zoom_pane(pane)?);
            }
            Request::Panes => response.panes = Some(workspace.panes_in(None)?),
            Request::Layout {
                columns,
                rows,
                command,
            } => {
                terminal.tick(workspace)?;
                if workspace.is_empty() {
                    bail!("workspace has ended");
                }
                response.panes = Some(workspace.set_grid_in_with_command(
                    None,
                    columns,
                    rows,
                    Some(&command),
                )?);
            }
            Request::FocusPane { pane } => {
                terminal.tick(workspace)?;
                if workspace.is_empty() {
                    bail!("workspace has ended");
                }
                workspace.focus_pane(pane)?;
                response.panes = Some(workspace.panes_in(None)?);
            }
            Request::ClosePane { pane } => {
                terminal.tick(workspace)?;
                if workspace.is_empty() {
                    bail!("workspace has ended");
                }
                workspace.close_pane(pane)?;
                response.panes = Some(workspace.panes_in(None)?);
            }
            Request::Attach {
                id,
                socket,
                cols,
                rows,
                cell_width,
                cell_height,
                theme,
            } => {
                terminal.tick(workspace)?;
                if terminal.finished() || workspace.is_empty() {
                    bail!("workspace has ended");
                }
                if terminal.is_attached() {
                    bail!(ATTACHED_TERMINAL_ERROR);
                }
                let runtime = runtime_dir()?;
                let metadata = fs::metadata(&socket)
                    .with_context(|| format!("inspect attachment socket {}", socket.display()))?;
                if socket.parent() != Some(runtime.as_path())
                    || !metadata.file_type().is_socket()
                    || metadata.uid() != unsafe { libc::geteuid() }
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    bail!(
                        "attachment socket must be an owner-only socket in the runtime directory"
                    );
                }
                let stream = UnixStream::connect(&socket).with_context(|| {
                    format!("connect workspace attachment at {}", socket.display())
                })?;
                stream
                    .set_write_timeout(Some(Duration::from_millis(250)))
                    .context("bound workspace attachment output")?;
                let mut reader = stream
                    .try_clone()
                    .context("clone workspace attachment reader")?;
                let shutdown = stream
                    .try_clone()
                    .context("clone workspace attachment shutdown handle")?;
                let (send, receive) = std::sync::mpsc::sync_channel(64);
                let reader = thread::spawn(move || {
                    let mut bytes = [0_u8; 1024];
                    loop {
                        match reader.read(&mut bytes) {
                            Ok(0) => break,
                            Ok(length) => {
                                if send.send(bytes[..length].to_vec()).is_err() {
                                    return;
                                }
                            }
                            Err(error) if error.kind() == ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                });
                let cleanup = Box::new(move || {
                    let _ = shutdown.shutdown(std::net::Shutdown::Both);
                    let _ = reader.join();
                });
                terminal.attach(
                    workspace,
                    receive,
                    Box::new(AttachmentWriter {
                        stream,
                        cleanup: Some(cleanup),
                    }),
                    WorkspaceAttachmentOptions {
                        id,
                        cols,
                        rows,
                        cell_width,
                        cell_height,
                        theme,
                    },
                )?;
            }
            Request::ResizeAttachment {
                id,
                cols,
                rows,
                cell_width,
                cell_height,
            } => terminal.resize_attachment(workspace, id, cols, rows, cell_width, cell_height)?,
            Request::Stop => workspace.try_stop()?,
        }
        Ok(response)
    }

    fn require_control_duration(label: &str, milliseconds: u64) -> Result<()> {
        if milliseconds > MAX_CONTROL_DURATION_MS {
            bail!("{label} exceeds the 10 minute workspace control limit");
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn name_start_lock_rejects_a_concurrent_owner() {
            let path = std::env::temp_dir().join(format!(
                "termctrl-start-lock-test-{}.lock",
                std::process::id()
            ));
            let held = StartLock::acquire(&path).unwrap();

            assert!(StartLock::acquire(&path).is_err());
            drop(held);
            assert!(StartLock::acquire(&path).is_ok());
            let _ = fs::remove_file(path);
        }

        #[test]
        fn workspace_show_returns_capture_and_status_atomically() {
            let command = [
                "sh".to_owned(),
                "-c".to_owned(),
                "printf READY; cat".to_owned(),
            ];
            let mut workspace = Workspace::start_named_with_theme(
                "atomic-show",
                &command,
                None,
                None,
                &Options::default(),
                crate::terminal_theme::TerminalTheme::default(),
                TabPosition::Bottom,
            )
            .unwrap();
            let mut terminal = WorkspaceTerminal::detached();

            let response = respond_workspace(
                &mut workspace,
                Request::Show {
                    settle_ms: 0,
                    deadline_ms: 0,
                    pane: None,
                },
                &mut terminal,
            )
            .unwrap();

            assert!(response.captured.is_some());
            assert!(response.status.is_some());
            workspace.stop();
        }

        #[test]
        fn only_connection_failures_identify_stale_sockets() {
            let stale = anyhow::Error::new(std::io::Error::from(ErrorKind::ConnectionRefused));
            let incompatible = anyhow::Error::new(std::io::Error::from(ErrorKind::UnexpectedEof));

            assert!(stale_socket_error(&stale));
            assert!(!stale_socket_error(&incompatible));
        }

        #[test]
        fn prune_dry_run_preserves_and_prune_removes_a_trusted_stale_socket() {
            let runtime = std::env::temp_dir().join(format!(
                "termctrl-prune-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            fs::create_dir_all(&runtime).unwrap();
            let socket = runtime.join("stale.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
            drop(listener);
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                match request(socket.clone(), &Request::Ping) {
                    Err(error) if stale_socket_error(&error) => break,
                    Err(_) => {}
                    Ok(_) => panic!("closed test socket unexpectedly returned a response"),
                }
                assert!(
                    Instant::now() < deadline,
                    "test socket did not become stale"
                );
                thread::sleep(Duration::from_millis(1));
            }

            assert_eq!(
                prune_in(&runtime, "stale", true).unwrap(),
                Some(PruneKind::Stale)
            );
            assert!(socket.exists());
            assert_eq!(
                prune_in(&runtime, "stale", false).unwrap(),
                Some(PruneKind::Stale)
            );
            assert!(!socket.exists());
            let _ = fs::remove_dir_all(runtime);
        }

        #[test]
        fn layout_command_capability_check_holds_the_name_lifecycle_lock() {
            let runtime = std::env::temp_dir().join(format!(
                "termctrl-layout-lock-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            fs::create_dir_all(&runtime).unwrap();
            let held = StartLock::acquire(&runtime.join("workspace.lock")).unwrap();

            let error = request_layout_command_in(
                &runtime,
                "workspace",
                &Request::Layout {
                    columns: 2,
                    rows: 1,
                    command: vec!["nvim".to_owned()],
                },
            )
            .err()
            .unwrap();

            assert!(error.to_string().contains("already starting this name"));
            drop(held);
            let _ = fs::remove_dir_all(runtime);
        }
    }
}

#[cfg(not(unix))]
mod implementation {
    use super::{NamedSessionStatus, Options, PruneKind, Request, Response, TabPosition};
    use anyhow::{Result, bail};
    use std::path::{Path, PathBuf};

    pub fn runtime_dir() -> Result<PathBuf> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn serve_workspace(
        _: String,
        _: PathBuf,
        _: Vec<String>,
        _: Option<PathBuf>,
        _: Option<PathBuf>,
        _: Options,
        _: TabPosition,
    ) -> Result<()> {
        bail!("persistent workspaces require Unix sockets")
    }
    pub fn start(
        _: &str,
        _: &[String],
        _: Option<&Path>,
        _: Option<&Path>,
        _: &Options,
    ) -> Result<()> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn restart(
        _: &str,
        _: &[String],
        _: Option<&Path>,
        _: Option<&Path>,
        _: &Options,
    ) -> Result<()> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn request(_: PathBuf, _: &Request) -> Result<Response> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn request_layout_command(_: &str, _: &Request) -> Result<Response> {
        bail!("persistent sessions require Unix sockets")
    }

    pub fn attach(_: PathBuf, _: &str, _: &Options) -> Result<()> {
        bail!("workspace attachment requires Unix sockets")
    }
    pub fn list() -> Result<Vec<NamedSessionStatus>> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn prune(_: &str, _: bool) -> Result<Option<PruneKind>> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn serve(
        _: String,
        _: PathBuf,
        _: Vec<String>,
        _: Option<PathBuf>,
        _: Option<PathBuf>,
        _: Options,
    ) -> Result<()> {
        bail!("persistent sessions require Unix sockets")
    }
    pub fn run_foreground(
        _: &str,
        _: &[String],
        _: Option<&Path>,
        _: Option<&Path>,
        _: &Options,
        _: TabPosition,
    ) -> Result<()> {
        bail!("persistent sessions require Unix sockets")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_workspace_attachment_error_names_recovery_actions() {
        let error = attachment_rejection("editor", "workspace already has an attached terminal")
            .to_string();

        assert!(error.contains("workspace \"editor\""));
        assert!(error.contains("ctrl-b d"));
        assert!(error.contains("termctrl run NAME"));
    }

    #[test]
    fn transient_one_row_attachment_sizes_are_not_sent() {
        assert!(!valid_workspace_attachment_size(80, 1));
        assert!(valid_workspace_attachment_size(80, 2));
    }

    #[test]
    fn old_control_responses_deserialize_but_require_a_workspace_restart() {
        let response: Response = serde_json::from_str(
            r#"{
                "error": null,
                "captured": null,
                "status": null,
                "logs": null,
                "panes": [{
                    "id": 0,
                    "active": true,
                    "state": "running",
                    "cols": 80,
                    "rows": 24,
                    "command": ["zsh"],
                    "cwd": "/tmp"
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(response.protocol_version, 0);
        assert!(response.windows.is_none());
        let pane = &response.panes.as_ref().unwrap()[0];
        assert_eq!((pane.x, pane.y), (0, 0));
        assert!(pane.visible);
        assert!(pane.title.is_empty());
        let window: WindowStatus = serde_json::from_str(
            r#"{
                "index": 0,
                "name": "main",
                "active": true,
                "pane_count": 1,
                "active_pane": 0,
                "cols": 80,
                "rows": 24
            }"#,
        )
        .unwrap();
        assert!(window.activity_kinds.is_empty());
        assert!(
            ProtocolCapability::Pane
                .require(&response)
                .unwrap_err()
                .to_string()
                .contains("restart it with the current termctrl")
        );
        assert!(
            ProtocolCapability::Window
                .require(&response)
                .unwrap_err()
                .to_string()
                .contains("predates named windows")
        );
        assert!(
            ProtocolCapability::LayoutCommand
                .require(&response)
                .unwrap_err()
                .to_string()
                .contains("predates pane startup commands")
        );
        assert!(
            ProtocolCapability::WorkspaceControl
                .require(&response)
                .unwrap_err()
                .to_string()
                .contains("predates runtime workspace controls")
        );
        let version_four = Response {
            protocol_version: LAYOUT_COMMAND_PROTOCOL_VERSION,
            ..Response::default()
        };
        ProtocolCapability::Window.require(&version_four).unwrap();
        ProtocolCapability::LayoutCommand
            .require(&version_four)
            .unwrap();
        assert!(
            ProtocolCapability::WorkspaceControl
                .require(&version_four)
                .is_err()
        );
        assert!(ProtocolCapability::Semantic.require(&version_four).is_err());
        assert_eq!(
            Response::default().protocol_version,
            SEMANTIC_PROTOCOL_VERSION
        );
    }

    #[test]
    fn workspace_request_requirements_are_local_to_each_request() {
        let cases = [
            (Request::Panes, Some(ProtocolCapability::Pane), false),
            (
                Request::Show {
                    settle_ms: 0,
                    deadline_ms: 0,
                    pane: Some(3),
                },
                Some(ProtocolCapability::Pane),
                false,
            ),
            (
                Request::Attach {
                    id: 1,
                    socket: PathBuf::from("/tmp/attach.sock"),
                    cols: 80,
                    rows: 24,
                    cell_width: 9,
                    cell_height: 18,
                    theme: TerminalTheme::default(),
                },
                Some(ProtocolCapability::Attachment),
                false,
            ),
            (Request::Windows, Some(ProtocolCapability::Window), false),
            (
                Request::Layout {
                    columns: 2,
                    rows: 1,
                    command: vec!["nvim".to_owned()],
                },
                Some(ProtocolCapability::LayoutCommand),
                true,
            ),
            (
                Request::SetTabPosition {
                    position: TabPosition::Top,
                },
                Some(ProtocolCapability::WorkspaceControl),
                false,
            ),
            (
                Request::SemanticSnapshot {
                    deadline_unix_ms: 1,
                    pane: None,
                    window: None,
                },
                Some(ProtocolCapability::Semantic),
                false,
            ),
            (Request::Status, None, false),
        ];

        for (request, capability, hold_name_lock) in cases {
            assert_eq!(
                request.requirements(),
                RequestRequirements {
                    capability,
                    hold_name_lock,
                }
            );
        }
    }

    #[test]
    fn semantic_workspace_requests_round_trip() {
        for request in [
            Request::Layout {
                columns: 2,
                rows: 2,
                command: vec!["nvim".to_owned()],
            },
            Request::FocusPane { pane: 3 },
            Request::ClosePane { pane: 2 },
            Request::Windows,
            Request::WorkspaceContext { pane: Some(3) },
            Request::SetTabPosition {
                position: TabPosition::Top,
            },
            Request::MoveWindow {
                name: "editor".to_owned(),
                index: 0,
            },
            Request::CreateWindow {
                name: Some("editor".to_owned()),
                command: vec!["nvim".to_owned()],
                cwd: Some(PathBuf::from("/tmp/project")),
            },
            Request::SelectWindow {
                name: "editor".to_owned(),
            },
            Request::RenameWindow {
                name: "editor".to_owned(),
                new_name: "code".to_owned(),
            },
            Request::WindowPanes {
                name: "code".to_owned(),
            },
            Request::WindowLayout {
                name: "code".to_owned(),
                columns: 2,
                rows: 1,
                command: Vec::new(),
            },
            Request::CloseWindow {
                name: "code".to_owned(),
            },
            Request::MovePane {
                pane: 3,
                window: "code".to_owned(),
                vertical: false,
            },
            Request::ResizePane {
                pane: 3,
                direction: PaneDirection::Left,
                cells: 5,
            },
            Request::ToggleZoom { pane: 3 },
            Request::Attach {
                id: 7,
                socket: PathBuf::from("/tmp/attach.sock"),
                cols: 80,
                rows: 24,
                cell_width: 9,
                cell_height: 18,
                theme: TerminalTheme::default(),
            },
            Request::ResizeAttachment {
                id: 7,
                cols: 100,
                rows: 30,
                cell_width: 9,
                cell_height: 18,
            },
            Request::SemanticSnapshot {
                deadline_unix_ms: 1,
                pane: Some(3),
                window: None,
            },
        ] {
            let encoded = serde_json::to_vec(&request).unwrap();
            let decoded: Request = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                serde_json::to_value(request).unwrap()
            );
        }
    }

    #[test]
    fn named_session_response_preserves_a_null_semantic_snapshot() {
        let encoded = serde_json::to_vec(&Response {
            semantic_snapshot: Some(SemanticSnapshotResult { value: Value::Null }),
            ..Response::default()
        })
        .unwrap();
        let decoded: Response = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded.semantic_snapshot.unwrap().value, Value::Null);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn zsh_redraws_backspace_with_the_default_terminal_contract() {
        let mut session = Session::start(
            &["/bin/zsh".to_owned(), "-df".to_owned()],
            None,
            None,
            &Options {
                cols: 40,
                rows: 4,
                ..Options::default()
            },
        )
        .unwrap();

        session.wait_for_text("%", Duration::from_secs(2)).unwrap();
        session.send(b"PS1='READY> '\r").unwrap();
        session
            .wait_for_text("READY>", Duration::from_secs(2))
            .unwrap();
        session.send(b"abc\x7fX").unwrap();
        session
            .wait_for_text("READY> abX", Duration::from_secs(2))
            .unwrap();
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn embedded_session_waits_sends_resizes_and_captures_the_screen() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf ready; IFS= read -r line; printf '\\r\\ngot:%s' \"$line\"; sleep 1"
                    .to_owned(),
            ],
            None,
            None,
            &Options {
                cols: 20,
                rows: 4,
                settle: Duration::from_millis(10),
                deadline: Duration::from_secs(2),
                ..Options::default()
            },
        )
        .unwrap();

        session
            .wait_for_text("ready", Duration::from_secs(2))
            .unwrap();
        session.send(b"hello\r").unwrap();
        session
            .wait_for_text("got:hello", Duration::from_secs(2))
            .unwrap();
        assert_eq!(session.status().unwrap().state, SessionState::Running);
        session
            .wait_for_idle(Duration::from_millis(10), Duration::from_secs(2))
            .unwrap();
        session.resize(30, 5, 9, 18).unwrap();
        let shot = session
            .capture(Duration::from_millis(10), Duration::from_secs(2))
            .unwrap();

        assert_eq!((shot.shot.frame.cols, shot.shot.frame.rows), (30, 5));
        assert!(shot.shot.frame.text().contains("got:hello"));
        session.stop().unwrap();
        assert_eq!(session.status().unwrap().state, SessionState::Exited);
        assert!(session.capture(Duration::ZERO, Duration::ZERO).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn embedded_session_advertises_and_cleans_up_the_application_semantic_socket() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                format!(
                    "if [ -S \"${}\" ]; then printf semantic-ready; else printf semantic-missing; fi; sleep 10",
                    semantic::SOCKET_ENV
                ),
            ],
            None,
            None,
            &Options {
                opentui_host: true,
                ..Options::default()
            },
        )
        .unwrap();
        let path = session
            .semantic
            .as_ref()
            .unwrap()
            .path()
            .unwrap()
            .to_owned();

        session
            .wait_for_text("semantic-ready", Duration::from_secs(2))
            .unwrap();
        assert!(path.exists());
        session.stop().unwrap();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn embedded_session_reads_its_application_semantic_snapshot() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;

        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), "sleep 10".to_owned()],
            None,
            None,
            &Options {
                opentui_host: true,
                ..Options::default()
            },
        )
        .unwrap();
        let path = session
            .semantic
            .as_ref()
            .unwrap()
            .path()
            .unwrap()
            .to_owned();
        let (connected, ready) = mpsc::channel();
        let application = thread::spawn(move || {
            let mut stream = UnixStream::connect(path).unwrap();
            stream
                .write_all(
                    b"{\"type\":\"hello\",\"protocolVersion\":1,\"application\":{\"name\":\"fixture\"},\"capabilities\":[\"semantic.snapshot\"]}\n",
                )
                .unwrap();
            connected.send(()).unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            assert!(line.contains("\"type\":\"ready\""));
            line.clear();
            stream.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["type"], "semantic.snapshot");
            let response = serde_json::json!({
                "type": "result",
                "id": request["id"],
                "value": { "format": "termctrl-semantic-snapshot-v1", "nodes": [] }
            });
            serde_json::to_writer(stream.get_mut(), &response).unwrap();
            stream.get_mut().write_all(b"\n").unwrap();
            stream.get_mut().flush().unwrap();
        });
        ready.recv().unwrap();

        let result = session.semantic_snapshot(Duration::from_secs(2)).unwrap();

        assert_eq!(result["format"], "termctrl-semantic-snapshot-v1");
        session.stop().unwrap();
        application.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn capture_reports_a_deadline_instead_of_implying_idle() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "while :; do printf xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; sleep 0.001; done"
                    .to_owned(),
            ],
            None,
            None,
            &Options::default(),
        )
        .unwrap();

        let capture = session
            .capture(Duration::from_secs(1), Duration::from_millis(50))
            .unwrap();

        assert_eq!(capture.reason, CaptureReason::Deadline);
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn short_settling_waits_for_initial_output_grace() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "sleep 0.02; printf READY".to_owned(),
            ],
            None,
            None,
            &Options::default(),
        )
        .unwrap();

        let capture = session
            .capture(Duration::from_millis(10), Duration::from_secs(2))
            .unwrap();

        assert_eq!(
            capture.shot.frame.text(),
            "READY",
            "capture reason: {:?}; retained output: {:?}",
            capture.reason,
            String::from_utf8_lossy(&capture.shot.ansi)
        );
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn session_resizes_after_retained_output_is_truncated() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf '123456789'; sleep 1".to_owned(),
            ],
            None,
            None,
            &Options {
                max_bytes: 4,
                ..Options::default()
            },
        )
        .unwrap();
        session
            .wait_for_text("123456789", Duration::from_secs(2))
            .unwrap();

        assert_eq!(session.logs(true).unwrap(), b"6789");
        assert!(session.status().unwrap().logs_truncated);
        session.resize(4, 3, 9, 18).unwrap();
        let capture = session.capture(Duration::ZERO, Duration::ZERO).unwrap();
        assert_eq!((capture.shot.frame.cols, capture.shot.frame.rows), (4, 3));
        assert_eq!(capture.shot.frame.text(), "1234\n5678\n9");
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn status_preserves_the_observed_process_exit() {
        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), "exit 7".to_owned()],
            None,
            None,
            &Options::default(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = session.status().unwrap();
            if status.state == SessionState::Exited {
                break status;
            }
            assert!(Instant::now() < deadline, "child did not exit");
            thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(status.exit.unwrap().code, 7);
    }

    #[cfg(unix)]
    #[test]
    fn status_retains_canonical_launch_details() {
        let mut session = Session::start(
            &["/bin/sh".to_owned(), "-c".to_owned(), "sleep 1".to_owned()],
            Some(Path::new("/tmp")),
            None,
            &Options::default(),
        )
        .unwrap();

        let status = session.status().unwrap();
        assert_eq!(status.launch.command, ["/bin/sh", "-c", "sleep 1"]);
        assert_eq!(status.launch.cwd, std::fs::canonicalize("/tmp").unwrap());
        session.stop().unwrap();
    }

    #[test]
    fn infers_session_name_from_executable_basename() {
        assert_eq!(infer_name(&["nvim".to_owned()]).unwrap(), "nvim");
        assert_eq!(infer_name(&["/usr/bin/nvim".to_owned()]).unwrap(), "nvim");
    }

    #[test]
    fn rejects_empty_or_invalid_inferred_session_names() {
        assert!(
            infer_name(&["/".to_owned()])
                .unwrap_err()
                .to_string()
                .contains("basename is empty or invalid")
        );
        assert!(
            infer_name(&["/usr/bin/my editor".to_owned()])
                .unwrap_err()
                .to_string()
                .contains("basename \"my editor\" is invalid")
        );
        assert!(
            infer_name(&[])
                .unwrap_err()
                .to_string()
                .contains("provide a command after --")
        );
    }

    #[cfg(unix)]
    #[test]
    fn recorded_session_encodes_resize_in_its_timeline() {
        let record = std::env::temp_dir().join(format!(
            "termctrl-recorded-resize-test-{}.termctrl",
            std::process::id()
        ));
        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), "sleep 1".to_owned()],
            None,
            Some(&record),
            &Options::default(),
        )
        .unwrap();

        session.resize(100, 32, 9, 18).unwrap();
        session.stop().unwrap();
        let recording = recording::read(&record).unwrap();
        assert!(matches!(
            recording.events.last(),
            Some(recording::Entry::Resize {
                cols: 100,
                rows: 32,
                ..
            })
        ));
        let _ = std::fs::remove_file(record);
    }

    #[cfg(unix)]
    #[test]
    fn waits_for_exit_without_polling_status() {
        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), "exit 3".to_owned()],
            None,
            None,
            &Options::default(),
        )
        .unwrap();

        assert_eq!(
            session
                .wait_for_exit(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .code,
            3
        );
    }

    #[cfg(unix)]
    #[test]
    fn logs_expose_normal_screen_scrollback_and_raw_stream() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf 'one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n'; sleep 1".to_owned(),
            ],
            None,
            None,
            &Options {
                cols: 20,
                rows: 2,
                ..Options::default()
            },
        )
        .unwrap();
        session
            .wait_for_text("five", Duration::from_secs(2))
            .unwrap();

        let logs = String::from_utf8(session.logs(false).unwrap()).unwrap();
        assert!(logs.contains("one"));
        assert!(logs.contains("five"));
        assert!(
            session
                .logs(true)
                .unwrap()
                .windows(3)
                .any(|bytes| bytes == b"one")
        );
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stopping_allows_a_hangup_handler_to_run_before_forcing_exit() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "trap 'printf HUP-SEEN; exit 0' HUP; printf READY; while :; do :; done".to_owned(),
            ],
            None,
            None,
            &Options::default(),
        )
        .unwrap();
        session
            .wait_for_text("READY", Duration::from_secs(2))
            .unwrap();

        session.stop().unwrap();

        assert!(
            session
                .logs(true)
                .unwrap()
                .windows(b"HUP-SEEN".len())
                .any(|bytes| bytes == b"HUP-SEEN")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stopping_after_pty_eof_terminates_still_running_process() {
        let pid_path = std::env::temp_dir().join(format!(
            "termctrl-pty-eof-owner-test-{}.pid",
            std::process::id()
        ));
        let script = format!(
            "printf '%s' $$ > '{}'; exec >/dev/null 2>&1; sleep 30",
            pid_path.display()
        );
        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), script],
            None,
            None,
            &Options::default(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let pid = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_path) {
                break pid.parse::<i32>().unwrap();
            }
            assert!(Instant::now() < deadline, "child did not write its pid");
            thread::sleep(Duration::from_millis(10));
        };
        thread::sleep(Duration::from_millis(20));

        assert_eq!(session.status().unwrap().state, SessionState::Running);
        session.stop().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        let _ = std::fs::remove_file(pid_path);
    }

    #[cfg(unix)]
    #[test]
    fn natural_parent_exit_terminates_pty_holding_descendants() {
        let pid_path = std::env::temp_dir().join(format!(
            "termctrl-exited-owner-test-{}.pid",
            std::process::id()
        ));
        let script = format!(
            "sleep 30 & printf '%s' $! > '{}'; exit 0",
            pid_path.display()
        );
        let mut session = Session::start(
            &["sh".to_owned(), "-c".to_owned(), script],
            None,
            None,
            &Options::default(),
        )
        .unwrap();

        session
            .wait_for_exit(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let pid = std::fs::read_to_string(&pid_path)
            .unwrap()
            .parse::<i32>()
            .unwrap();

        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        let _ = std::fs::remove_file(pid_path);
    }

    #[cfg(unix)]
    #[test]
    fn process_exit_reports_exited_and_rejects_further_input() {
        let mut session = Session::start(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "sleep 30 & exit 0".to_owned(),
            ],
            None,
            None,
            &Options::default(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = session.status().unwrap();
            if status.exit.is_some() {
                break status;
            }
            assert!(Instant::now() < deadline, "parent process did not exit");
            thread::sleep(Duration::from_millis(5));
        };

        assert_eq!(status.state, SessionState::Exited);
        assert!(session.send(b"should-not-arrive").is_err());
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn daemon_start_failure_removes_bound_socket() {
        let socket = std::env::temp_dir().join(format!(
            "termctrl-failed-daemon-start-{}.sock",
            std::process::id()
        ));
        let result = serve(
            "daemon-start-failure".to_owned(),
            socket.clone(),
            vec!["/definitely/not/a/termctrl-command".to_owned()],
            None,
            None,
            Options::default(),
        );

        assert!(result.is_err());
        assert!(!socket.exists());
    }
}
