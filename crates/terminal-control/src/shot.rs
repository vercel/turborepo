use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::frame::{Color, Frame, indexed_color};
use crate::terminal_core::TerminalCore;
use crate::terminal_theme::TerminalTheme;
use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use crate::semantic;

const OPENTUI_QUERY: &[u8] = b"\x1b]10;?\x07\x1b]11;?\x07";
#[cfg(test)]
const PALETTE_QUERY: &[u8] = b"\x1b]4;0;?\x07";
const KITTY_QUERY: &[u8] = b"\x1b_Gi=31337";

/// Configuration for observing one terminal shot or starting a live session.
#[derive(Clone, Debug)]
pub struct Options {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
    pub settle: Duration,
    pub deadline: Duration,
    pub input: Vec<u8>,
    pub initial_delay: Duration,
    pub wait_for: Option<String>,
    pub max_bytes: usize,
    pub opentui_host: bool,
    pub color: ColorMode,
    /// Additional environment values set in the observed terminal application.
    pub env: BTreeMap<String, String>,
    /// Whether the terminal application inherits the parent process environment.
    pub inherit_env: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            cell_width: 9,
            cell_height: 18,
            settle: Duration::from_millis(250),
            deadline: Duration::from_secs(5),
            input: Vec::new(),
            initial_delay: Duration::ZERO,
            wait_for: None,
            max_bytes: 16 * 1024 * 1024,
            opentui_host: false,
            color: ColorMode::Auto,
            env: BTreeMap::new(),
            inherit_env: true,
        }
    }
}

/// A visible terminal frame together with its source ANSI/VT stream.
#[derive(Deserialize, Serialize)]
pub struct Shot {
    pub frame: Frame,
    pub ansi: Vec<u8>,
}

/// Environment policy applied to a launched command's color configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Construct a shot by replaying an ANSI/VT byte stream into a terminal frame.
pub fn from_ansi(bytes: Vec<u8>, rows: u16, cols: u16, max_bytes: usize) -> Result<Shot> {
    validate_geometry(rows, cols)?;
    if bytes.len() > max_bytes {
        bail!("terminal input exceeds --max-bytes ({max_bytes})");
    }
    let mut terminal = TerminalCore::new(rows, cols, 0)?;
    let _responses = terminal.apply_output(&bytes);
    Ok(Shot {
        frame: terminal.frame()?,
        ansi: bytes,
    })
}

/// Observe a command launched inside a pseudo-terminal and return its settled shot.
pub fn from_command(command: &[String], cwd: Option<&Path>, options: &Options) -> Result<Shot> {
    if command.is_empty() {
        bail!("provide a command after --");
    }
    validate_geometry(options.rows, options.cols)?;
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: options.rows,
            cols: options.cols,
            pixel_width: options.cell_width,
            pixel_height: options.cell_height,
        })
        .context("open pseudo-terminal")?;
    let mut semantic = options
        .opentui_host
        .then(semantic::Host::bind)
        .transpose()?;
    let mut builder = CommandBuilder::new(&command[0]);
    builder.args(&command[1..]);
    configure_pty_environment(&mut builder, options);
    if let Some(path) = semantic.as_ref().and_then(semantic::Host::path) {
        builder.env(semantic::SOCKET_ENV, path);
    }
    if let Some(cwd) = cwd {
        builder.cwd(cwd);
    }
    let mut reader = pair.master.try_clone_reader().context("open PTY reader")?;
    let writer = pair.master.take_writer().context("open PTY writer")?;
    let mut child = pair
        .slave
        .spawn_command(builder)
        .context("spawn terminal command")?;
    drop(pair.slave);
    #[cfg(unix)]
    let process_group = child.process_id().and_then(|pid| i32::try_from(pid).ok());
    let (send, receive) = mpsc::sync_channel::<Option<Vec<u8>>>(32);
    let _reader_thread = thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    if send.send(Some(buffer[..len].to_vec())).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = send.send(None);
    });
    let result = (|| {
        let mut terminal = TerminalCore::new(options.rows, options.cols, 0)?;
        let mut ansi = Vec::new();
        let mut host = Host::new(writer, options);
        let started = Instant::now();
        let mut clock = Clock {
            started,
            deadline: started + options.deadline,
            last_output: None,
        };
        let closed = consume_until_ready(
            &receive,
            &mut terminal,
            &mut ansi,
            &mut host,
            &mut semantic,
            options,
            &mut clock,
        )?;
        if let Some(pattern) = options.wait_for.as_deref()
            && !terminal.text()?.contains(pattern)
        {
            bail!(
                "visible terminal did not include --wait-for {pattern:?} before command ended or deadline elapsed"
            );
        }
        if !closed && Instant::now() < clock.deadline && !options.input.is_empty() {
            // Once input is sent, the pre-input idle frame is no longer the shot target.
            clock.last_output = None;
            host.send(&options.input)?;
            consume_until_settled(
                &receive,
                &mut terminal,
                &mut ansi,
                &mut host,
                &mut semantic,
                options,
                &mut clock,
            )?;
        }
        Ok(Shot {
            frame: terminal.frame()?,
            ansi,
        })
    })();
    #[cfg(unix)]
    if let Some(process_group) = process_group {
        // portable-pty spawns the application as a session leader; kill its group so helpers do
        // not retain the slave PTY after a frozen shot is returned.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    drop(receive);
    let teardown_deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < teardown_deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    result
}

/// Observe piped command output and return its final rendered shot.
pub fn from_pipe_command(
    command: &[String],
    cwd: Option<&Path>,
    options: &Options,
) -> Result<Shot> {
    if command.is_empty() {
        bail!("provide a command after --");
    }
    validate_geometry(options.rows, options.cols)?;
    let mut semantic = options
        .opentui_host
        .then(semantic::Host::bind)
        .transpose()?;
    let mut builder = ProcessCommand::new(&command[0]);
    builder
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        builder.process_group(0);
    }
    configure_process_environment(&mut builder, options);
    if let Some(path) = semantic.as_ref().and_then(semantic::Host::path) {
        builder.env(semantic::SOCKET_ENV, path);
    }
    if let Some(cwd) = cwd {
        builder.current_dir(cwd);
    }
    let mut child = builder
        .spawn()
        .with_context(|| format!("spawn command {:?}", command[0]))?;
    #[cfg(unix)]
    let process_group = i32::try_from(child.id()).ok();
    let stdout = child.stdout.take().context("open command stdout")?;
    let stderr = child.stderr.take().context("open command stderr")?;
    let (send, receive) = mpsc::sync_channel::<Option<Vec<u8>>>(32);
    spawn_pipe_reader(stdout, send.clone());
    spawn_pipe_reader(stderr, send);

    let result = (|| {
        let mut terminal = TerminalCore::new(options.rows, options.cols, 0)?;
        let mut ansi = Vec::new();
        let mut normalizer = LinefeedNormalizer::default();
        let deadline = Instant::now() + options.deadline;
        let mut open_streams = 2_usize;
        let mut exited = false;
        while open_streams > 0 || !exited {
            if let Some(semantic) = &mut semantic {
                semantic.pump();
            }
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(20));
            if timeout.is_zero() {
                break;
            }
            match receive.recv_timeout(timeout) {
                Ok(Some(bytes)) => {
                    let bytes = normalizer.normalize(&bytes);
                    retain(&mut ansi, &bytes, options.max_bytes)?;
                    let _responses = terminal.apply_output(&bytes);
                }
                Ok(None) => open_streams = open_streams.saturating_sub(1),
                Err(RecvTimeoutError::Disconnected) => open_streams = 0,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if !exited {
                exited = child.try_wait().context("wait for command")?.is_some();
            }
        }

        if let Some(pattern) = options.wait_for.as_deref()
            && !terminal.text()?.contains(pattern)
        {
            bail!(
                "visible terminal did not include --wait-for {pattern:?} before command ended or deadline elapsed"
            );
        }
        Ok(Shot {
            frame: terminal.frame()?,
            ansi,
        })
    })();
    #[cfg(unix)]
    if let Some(process_group) = process_group {
        // Pipe shots own their launched command tree; do not leave diagnostic descendants alive.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn spawn_pipe_reader(
    mut reader: impl Read + Send + 'static,
    send: mpsc::SyncSender<Option<Vec<u8>>>,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    if send.send(Some(buffer[..len].to_vec())).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = send.send(None);
    });
}

#[derive(Default)]
struct LinefeedNormalizer {
    previous_was_cr: bool,
}

impl LinefeedNormalizer {
    fn normalize(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut normalized = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if byte == b'\n' && !self.previous_was_cr {
                normalized.push(b'\r');
            }
            normalized.push(byte);
            self.previous_was_cr = byte == b'\r';
        }
        normalized
    }
}

pub(crate) fn configure_pty_environment(builder: &mut CommandBuilder, options: &Options) {
    if !options.inherit_env {
        builder.env_clear();
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    match options.color {
        ColorMode::Auto => {}
        ColorMode::Always => {
            builder.env_remove("NO_COLOR");
            builder.env("FORCE_COLOR", "1");
            builder.env("CLICOLOR", "1");
            builder.env("CLICOLOR_FORCE", "1");
        }
        ColorMode::Never => {
            builder.env("NO_COLOR", "1");
            builder.env("FORCE_COLOR", "0");
            builder.env("CLICOLOR", "0");
            builder.env("CLICOLOR_FORCE", "0");
        }
    }
    for (key, value) in &options.env {
        builder.env(key, value);
    }
    // This endpoint belongs to the immediate Terminal Control host and must never leak
    // into nested or non-hosted launches. Hosted call sites set their fresh path later.
    builder.env_remove(semantic::SOCKET_ENV);
}

fn configure_process_environment(builder: &mut ProcessCommand, options: &Options) {
    if !options.inherit_env {
        builder.env_clear();
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    match options.color {
        ColorMode::Auto => {}
        ColorMode::Always => {
            builder.env_remove("NO_COLOR");
            builder.env("FORCE_COLOR", "1");
            builder.env("CLICOLOR", "1");
            builder.env("CLICOLOR_FORCE", "1");
        }
        ColorMode::Never => {
            builder.env("NO_COLOR", "1");
            builder.env("FORCE_COLOR", "0");
            builder.env("CLICOLOR", "0");
            builder.env("CLICOLOR_FORCE", "0");
        }
    }
    builder.envs(&options.env);
    builder.env_remove(semantic::SOCKET_ENV);
}

pub(crate) fn validate_geometry(rows: u16, cols: u16) -> Result<()> {
    if rows == 0 || cols == 0 {
        bail!("terminal dimensions must be greater than zero");
    }
    Ok(())
}

fn consume_until_ready(
    receive: &mpsc::Receiver<Option<Vec<u8>>>,
    terminal: &mut TerminalCore,
    ansi: &mut Vec<u8>,
    host: &mut Host,
    semantic: &mut Option<semantic::Host>,
    options: &Options,
    clock: &mut Clock,
) -> Result<bool> {
    let mut closed = false;
    let delay_end = (clock.started + options.initial_delay).min(clock.deadline);
    while !closed && Instant::now() < delay_end {
        closed = matches!(
            receive_chunk(
                receive,
                terminal,
                ansi,
                host,
                semantic,
                options.max_bytes,
                clock,
            )?,
            Chunk::Closed
        );
    }
    if let Some(pattern) = &options.wait_for {
        while !closed && Instant::now() < clock.deadline && !terminal.text()?.contains(pattern) {
            closed = matches!(
                receive_chunk(
                    receive,
                    terminal,
                    ansi,
                    host,
                    semantic,
                    options.max_bytes,
                    clock,
                )?,
                Chunk::Closed
            );
        }
    }
    if closed || Instant::now() >= clock.deadline {
        return Ok(closed);
    }
    if !options.input.is_empty() && (options.wait_for.is_some() || !options.initial_delay.is_zero())
    {
        return Ok(false);
    }
    consume_until_settled(receive, terminal, ansi, host, semantic, options, clock)
}

enum Chunk {
    Output,
    Timeout,
    Closed,
}

struct Clock {
    started: Instant,
    deadline: Instant,
    last_output: Option<Instant>,
}

fn receive_chunk(
    receive: &mpsc::Receiver<Option<Vec<u8>>>,
    terminal: &mut TerminalCore,
    ansi: &mut Vec<u8>,
    host: &mut Host,
    semantic: &mut Option<semantic::Host>,
    max_bytes: usize,
    clock: &mut Clock,
) -> Result<Chunk> {
    if let Some(semantic) = semantic {
        semantic.pump();
    }
    let timeout = clock
        .deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(20));
    if timeout.is_zero() {
        return Ok(Chunk::Timeout);
    }
    match receive.recv_timeout(timeout) {
        Ok(Some(bytes)) => {
            retain(ansi, &bytes, max_bytes)?;
            respond_to_output(terminal, host, &bytes)?;
            clock.last_output = Some(Instant::now());
            Ok(Chunk::Output)
        }
        Ok(None) | Err(RecvTimeoutError::Disconnected) => Ok(Chunk::Closed),
        Err(RecvTimeoutError::Timeout) => Ok(Chunk::Timeout),
    }
}

pub(crate) fn respond_to_output(
    terminal: &mut TerminalCore,
    host: &mut Host,
    output: &[u8],
) -> Result<Vec<u8>> {
    let ghostty_response = terminal.apply_output(output);
    if host.is_enabled() {
        host.respond(output)
    } else {
        if !ghostty_response.is_empty() && !host.send_if_open(&ghostty_response)? {
            return Ok(Vec::new());
        }
        Ok(ghostty_response)
    }
}

fn consume_until_settled(
    receive: &mpsc::Receiver<Option<Vec<u8>>>,
    terminal: &mut TerminalCore,
    ansi: &mut Vec<u8>,
    host: &mut Host,
    semantic: &mut Option<semantic::Host>,
    options: &Options,
    clock: &mut Clock,
) -> Result<bool> {
    loop {
        match receive_chunk(
            receive,
            terminal,
            ansi,
            host,
            semantic,
            options.max_bytes,
            clock,
        )? {
            Chunk::Output => {}
            Chunk::Closed => return Ok(true),
            Chunk::Timeout => {
                if Instant::now() >= clock.deadline {
                    return Ok(false);
                }
            }
        }
        if clock
            .last_output
            .is_some_and(|last| last.elapsed() >= options.settle)
        {
            return Ok(false);
        }
    }
}

pub(crate) fn retain(ansi: &mut Vec<u8>, bytes: &[u8], max_bytes: usize) -> Result<()> {
    if ansi
        .len()
        .checked_add(bytes.len())
        .is_none_or(|total| total > max_bytes)
    {
        bail!("terminal output exceeds --max-bytes ({max_bytes})");
    }
    ansi.extend_from_slice(bytes);
    Ok(())
}

pub(crate) struct Host {
    writer: Box<dyn Write + Send>,
    enabled: bool,
    opentui_replied: bool,
    kitty_replied: bool,
    probe: Vec<u8>,
    color_probe: Vec<u8>,
    pixel_width: u32,
    pixel_height: u32,
    theme: TerminalTheme,
}

impl Host {
    pub(crate) fn new(writer: Box<dyn Write + Send>, options: &Options) -> Self {
        Self::new_with_theme(writer, options, TerminalTheme::default())
    }

    pub(crate) fn new_with_theme(
        writer: Box<dyn Write + Send>,
        options: &Options,
        theme: TerminalTheme,
    ) -> Self {
        Self {
            writer,
            enabled: options.opentui_host,
            opentui_replied: false,
            kitty_replied: false,
            probe: Vec::new(),
            color_probe: Vec::new(),
            pixel_width: u32::from(options.cols) * u32::from(options.cell_width),
            pixel_height: u32::from(options.rows) * u32::from(options.cell_height),
            theme,
        }
    }

    pub(crate) fn send(&mut self, input: &[u8]) -> Result<()> {
        self.writer
            .write_all(input)
            .context("send terminal input")?;
        self.writer.flush().context("flush terminal input")
    }

    pub(crate) fn send_if_open(&mut self, input: &[u8]) -> Result<bool> {
        match self.send(input) {
            Ok(()) => Ok(true),
            Err(error) if terminal_input_closed(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
        self.pixel_width = u32::from(cols) * u32::from(cell_width);
        self.pixel_height = u32::from(rows) * u32::from(cell_height);
    }

    pub(crate) fn set_theme(&mut self, theme: TerminalTheme) {
        self.theme = theme;
    }

    pub(crate) fn respond(&mut self, output: &[u8]) -> Result<Vec<u8>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let mut response = Vec::new();
        self.probe.extend_from_slice(output);
        self.color_probe.extend_from_slice(output);
        for query in take_color_queries(&mut self.color_probe) {
            match query {
                ColorQuery::Foreground => {
                    append_color_response(&mut response, "10", self.theme.foreground)
                }
                ColorQuery::Background => {
                    append_color_response(&mut response, "11", self.theme.background)
                }
                ColorQuery::Palette(index) => {
                    let color = self
                        .theme
                        .ansi
                        .get(usize::from(index))
                        .copied()
                        .unwrap_or_else(|| indexed_color(index));
                    append_color_response(&mut response, &format!("4;{index}"), color);
                }
            }
        }
        if !self.opentui_replied
            && self
                .probe
                .windows(OPENTUI_QUERY.len())
                .any(|window| window == OPENTUI_QUERY)
        {
            response.extend_from_slice(
                format!(
                        "\x1bP>|termctrl {}\x1b\\\x1b[1;1R\x1b[?1016;0$y\x1b[?2027;0$y\x1b[?2031;2$y\x1b[?1004;1$y\x1b[?2004;2$y\x1b[?2026;2$y\x1b[?0u\x1b[1;1R\x1b[1;1R\x1b[4;{};{}t\x1b[?6c",
                        env!("CARGO_PKG_VERSION"),
                        self.pixel_height,
                        self.pixel_width,
                )
                .as_bytes(),
            );
            self.opentui_replied = true;
        }
        if !self.kitty_replied
            && self
                .probe
                .windows(KITTY_QUERY.len())
                .any(|window| window == KITTY_QUERY)
        {
            response.extend_from_slice(b"\x1b_Gi=31337;EINVAL:graphics unavailable\x1b\\");
            self.kitty_replied = true;
        }
        if !response.is_empty() && !self.send_if_open(&response)? {
            response.clear();
        }
        if self.probe.len() > 64 {
            self.probe.drain(..self.probe.len() - 64);
        }
        Ok(response)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorQuery {
    Foreground,
    Background,
    Palette(u8),
}

fn append_color_response(response: &mut Vec<u8>, selector: &str, color: Color) {
    response.extend_from_slice(
        format!(
            "\x1b]{selector};rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
            color.r, color.r, color.g, color.g, color.b, color.b,
        )
        .as_bytes(),
    );
}

fn take_color_queries(probe: &mut Vec<u8>) -> Vec<ColorQuery> {
    let mut queries = Vec::new();
    let mut index = 0;
    while index < probe.len() {
        let prefix = if probe[index..].starts_with(b"\x1b]") {
            2
        } else if probe[index] == 0x9d {
            1
        } else {
            index += 1;
            continue;
        };
        let start = index + prefix;
        let Some((end, terminator)) = color_osc_end(probe, start) else {
            probe.drain(..index);
            return queries;
        };
        if let Ok(content) = std::str::from_utf8(&probe[start..end]) {
            let mut parts = content.split(';');
            match parts.next() {
                Some("10") if parts.next() == Some("?") => queries.push(ColorQuery::Foreground),
                Some("11") if parts.next() == Some("?") => queries.push(ColorQuery::Background),
                Some("4") => {
                    while let (Some(index), Some("?")) = (parts.next(), parts.next()) {
                        if let Ok(index) = index.parse() {
                            queries.push(ColorQuery::Palette(index));
                        }
                    }
                }
                _ => {}
            }
        }
        index = end + terminator;
    }
    probe.clear();
    queries
}

fn color_osc_end(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            0x07 | 0x9c => return Some((index, 1)),
            0x1b if bytes.get(index + 1) == Some(&b'\\') => return Some((index, 2)),
            _ => index += 1,
        }
    }
    None
}

fn terminal_input_closed(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let Some(error) = cause.downcast_ref::<std::io::Error>() else {
            return false;
        };
        if matches!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::UnexpectedEof
        ) {
            return true;
        }
        #[cfg(unix)]
        if error.raw_os_error() == Some(libc::EIO) {
            return true;
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Writer(Arc<Mutex<Vec<u8>>>);

    impl Write for Writer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn classifies_only_closed_terminal_input_errors_as_nonfatal() {
        assert!(terminal_input_closed(&anyhow::Error::new(
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed")
        )));
        assert!(!terminal_input_closed(&anyhow::Error::new(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied")
        )));
        #[cfg(unix)]
        assert!(terminal_input_closed(&anyhow::Error::new(
            std::io::Error::from_raw_os_error(libc::EIO)
        )));
    }

    #[test]
    fn normalizes_plain_line_feeds_for_pipe_output() {
        let mut normalizer = LinefeedNormalizer::default();
        assert_eq!(normalizer.normalize(b"one\n"), b"one\r\n");
        assert_eq!(normalizer.normalize(b"two\r"), b"two\r");
        assert_eq!(normalizer.normalize(b"\nthree"), b"\nthree");
    }

    #[cfg(unix)]
    #[test]
    fn pipe_command_captures_non_tty_output() {
        let captured = from_pipe_command(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "printf 'one\\ntwo\\n'".to_owned(),
            ],
            None,
            &Options {
                cols: 20,
                rows: 4,
                cell_width: 9,
                cell_height: 18,
                settle: Duration::ZERO,
                deadline: Duration::from_secs(2),
                input: Vec::new(),
                initial_delay: Duration::ZERO,
                wait_for: None,
                max_bytes: 1024,
                opentui_host: false,
                color: ColorMode::Auto,
                env: BTreeMap::new(),
                inherit_env: true,
            },
        )
        .unwrap();

        assert_eq!(captured.frame.text(), "one\ntwo");
        assert!(captured.ansi.windows(2).any(|window| window == b"\r\n"));
    }

    #[cfg(unix)]
    #[test]
    fn only_opentui_host_commands_receive_the_application_semantic_socket() {
        let inherited_socket = "/tmp/outer-termctrl-semantic.sock";
        let command = [
            "sh".to_owned(),
            "-c".to_owned(),
            format!(
                "if [ -S \"${}\" ]; then printf semantic-ready; else printf semantic-missing; fi",
                semantic::SOCKET_ENV
            ),
        ];

        let plain_pty = from_command(
            &command,
            None,
            &Options {
                settle: Duration::from_millis(10),
                deadline: Duration::from_secs(2),
                env: BTreeMap::from([(
                    semantic::SOCKET_ENV.to_owned(),
                    inherited_socket.to_owned(),
                )]),
                ..Options::default()
            },
        )
        .unwrap();
        let plain_pipe = from_pipe_command(
            &command,
            None,
            &Options {
                deadline: Duration::from_secs(2),
                env: BTreeMap::from([(
                    semantic::SOCKET_ENV.to_owned(),
                    inherited_socket.to_owned(),
                )]),
                ..Options::default()
            },
        )
        .unwrap();
        let opentui_pty = from_command(
            &command,
            None,
            &Options {
                settle: Duration::from_millis(10),
                deadline: Duration::from_secs(2),
                opentui_host: true,
                env: BTreeMap::from([(
                    semantic::SOCKET_ENV.to_owned(),
                    inherited_socket.to_owned(),
                )]),
                ..Options::default()
            },
        )
        .unwrap();
        let opentui_pipe = from_pipe_command(
            &command,
            None,
            &Options {
                deadline: Duration::from_secs(2),
                opentui_host: true,
                env: BTreeMap::from([(
                    semantic::SOCKET_ENV.to_owned(),
                    inherited_socket.to_owned(),
                )]),
                ..Options::default()
            },
        )
        .unwrap();

        assert_eq!(plain_pty.frame.text(), "semantic-missing");
        assert_eq!(plain_pipe.frame.text(), "semantic-missing");
        assert_eq!(opentui_pty.frame.text(), "semantic-ready");
        assert_eq!(opentui_pipe.frame.text(), "semantic-ready");
    }

    #[cfg(unix)]
    #[test]
    fn pipe_shot_terminates_descendant_processes() {
        let captured = from_pipe_command(
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                "sleep 30 & printf '%s' \"$!\"".to_owned(),
            ],
            None,
            &Options {
                deadline: Duration::from_millis(50),
                ..Options::default()
            },
        )
        .unwrap();
        let pid = captured.frame.text().parse::<i32>().unwrap();
        thread::sleep(Duration::from_millis(20));

        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[test]
    fn responds_to_split_opentui_query_with_requested_geometry() {
        let result = Arc::new(Mutex::new(Vec::new()));
        let mut host = Host::new(
            Box::new(Writer(result.clone())),
            &Options {
                cols: 100,
                rows: 24,
                cell_width: 9,
                cell_height: 20,
                settle: Duration::ZERO,
                deadline: Duration::ZERO,
                input: Vec::new(),
                initial_delay: Duration::ZERO,
                wait_for: None,
                max_bytes: 1,
                opentui_host: true,
                color: ColorMode::Auto,
                env: BTreeMap::new(),
                inherit_env: true,
            },
        );

        host.respond(b"\x1b]10;?\x07").unwrap();
        host.respond(b"\x1b]11;?\x07").unwrap();
        host.respond(b"\x1b]4;0;?\x07").unwrap();
        host.respond(b"\x1b_Gi=31337,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\")
            .unwrap();

        let output = String::from_utf8(result.lock().unwrap().clone()).unwrap();
        assert!(output.contains("\x1b[4;480;900t"));
        assert!(output.contains("\x1b]4;0;rgb:0000/0000/0000\x1b\\"));
        assert!(output.contains("\x1b_Gi=31337;EINVAL:graphics unavailable\x1b\\"));
    }

    #[test]
    fn opentui_host_replies_with_the_inherited_theme() {
        let result = Arc::new(Mutex::new(Vec::new()));
        let theme = crate::terminal_theme::TerminalTheme {
            foreground: crate::frame::Color { r: 1, g: 2, b: 3 },
            background: crate::frame::Color { r: 4, g: 5, b: 6 },
            ansi: std::array::from_fn(|index| crate::frame::Color {
                r: index as u8,
                g: 7,
                b: 8,
            }),
        };
        let mut host = Host::new_with_theme(
            Box::new(Writer(result.clone())),
            &Options {
                opentui_host: true,
                ..Options::default()
            },
            theme,
        );

        host.respond(OPENTUI_QUERY).unwrap();
        host.respond(PALETTE_QUERY).unwrap();
        host.respond(b"\x1b]4;1;?;15;?\x07\x1b]10;?\x07").unwrap();

        let output = String::from_utf8(result.lock().unwrap().clone()).unwrap();
        assert!(output.contains("\x1b]10;rgb:0101/0202/0303"));
        assert!(output.contains("\x1b]11;rgb:0404/0505/0606"));
        assert!(output.contains("\x1b]4;0;rgb:0000/0707/0808"));
        assert!(output.contains("\x1b]4;1;rgb:0101/0707/0808"));
        assert!(output.contains("\x1b]4;15;rgb:0f0f/0707/0808"));
        assert_eq!(output.matches("\x1b]10;rgb:0101/0202/0303").count(), 2);
    }

    #[test]
    fn opentui_host_does_not_duplicate_ghostty_responses() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut host = Host::new(
            Box::new(Writer(Arc::clone(&written))),
            &Options {
                opentui_host: true,
                ..Options::default()
            },
        );
        let mut terminal = TerminalCore::new(24, 80, 0).unwrap();

        let response = respond_to_output(&mut terminal, &mut host, OPENTUI_QUERY).unwrap();
        let foreground_responses = response
            .windows(b"\x1b]10;".len())
            .filter(|window| *window == b"\x1b]10;")
            .count();

        assert_eq!(foreground_responses, 1);
        assert_eq!(*written.lock().unwrap(), response);
    }

    #[test]
    fn rejects_zero_terminal_geometry_before_parsing() {
        assert!(from_ansi(Vec::new(), 0, 1, 1).is_err());
        assert!(from_ansi(Vec::new(), 1, 0, 1).is_err());
    }

    #[test]
    fn ghostty_ansi_shots_preserve_frame_v1_colors_and_attributes() {
        let shot = from_ansi(
            b"\x1b[1;3;4;38;5;214;48;2;30;34;42mwide: ".to_vec(),
            2,
            20,
            1024,
        )
        .unwrap();

        assert_eq!(shot.frame.text(), "wide:");
        assert_eq!(
            shot.frame.cells[0].foreground,
            crate::frame::indexed_color(214)
        );
        assert_eq!(
            shot.frame.cells[0].background,
            crate::frame::Color {
                r: 30,
                g: 34,
                b: 42,
            }
        );
        assert!(shot.frame.cells[0].attributes.bold);
        assert!(shot.frame.cells[0].attributes.italic);
        assert_eq!(
            shot.frame.cells[0].attributes.underline,
            Some(crate::frame::Underline::Single)
        );
    }

    #[test]
    fn retain_allows_appending_exactly_to_max_bytes() {
        let mut buffer = b"abc".to_vec();

        retain(&mut buffer, b"de", 5).unwrap();

        assert_eq!(buffer, b"abcde");
    }

    #[test]
    fn retain_rejects_over_max_bytes_without_mutating_buffer() {
        let mut buffer = b"abc".to_vec();

        let err = retain(&mut buffer, b"def", 5).unwrap_err();

        assert_eq!(err.to_string(), "terminal output exceeds --max-bytes (5)");
        assert_eq!(buffer, b"abc");
    }
}
