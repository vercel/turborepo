//! Terminal output for provisioning runs. Phases return values, `render_*`
//! turns them into text, and `Console` is the only writer of stdout and
//! stderr. Tool rows, notes, step lines and the final line go to stdout; the
//! spinner, child output and failure replays go to stderr.

use std::{
    cell::RefCell,
    collections::VecDeque,
    fmt::Write as _,
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use turborepo_repository::toolchain::ToolchainId;
use turborepo_ui::{BOLD_GREEN, BOLD_RED, ColorConfig, GREY};

use super::provision::LockChange;

/// How a dependency installer's output is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    /// stderr is a terminal outside CI: a spinner, with child output buffered
    /// and replayed only on failure.
    Interactive,
    /// CI or a non-terminal stderr: child output streams live, prefixed.
    Plain,
}

impl Mode {
    pub(super) fn detect(ci: bool, stderr_is_terminal: bool) -> Self {
        if !ci && stderr_is_terminal {
            Self::Interactive
        } else {
            Self::Plain
        }
    }

    pub(super) fn current(ci: bool) -> Self {
        Self::detect(ci, io::stderr().is_terminal())
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Style {
    pub mode: Mode,
    pub color: ColorConfig,
}

impl Style {
    fn paint(&self, style: &console::Style, text: &str) -> String {
        self.color.apply(style.apply_to(text)).to_string()
    }
}

/// What tool provisioning did, in `desired` order.
#[derive(Debug, Default)]
pub(super) struct ToolReport {
    pub rows: Vec<ToolRow>,
    pub notes: Vec<Note>,
    pub downloaded_bytes: u64,
}

impl ToolReport {
    pub fn installed(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| {
                matches!(
                    row.outcome,
                    ToolOutcome::Installed | ToolOutcome::InstalledFromCache
                )
            })
            .count()
    }

    /// Bundled tools came with their owner, so they count as neither.
    pub fn reused(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| {
                matches!(
                    row.outcome,
                    ToolOutcome::Reused
                        | ToolOutcome::Verified
                        | ToolOutcome::ReusedTrusted
                        | ToolOutcome::Reverified
                )
            })
            .count()
    }
}

#[derive(Debug)]
pub(super) struct ToolRow {
    pub id: String,
    pub version: String,
    pub outcome: ToolOutcome,
    /// An additional tool's source, shown after the outcome.
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolOutcome {
    Installed,
    InstalledFromCache,
    Reused,
    /// `--force` re-hashed a healthy installed tree.
    Verified,
    /// Offline trusted repeat.
    ReusedTrusted,
    /// Offline trusted repeat with `--force`.
    Reverified,
    /// npm shipped inside managed Node.
    BundledWithNode,
}

impl ToolOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::InstalledFromCache => "installed (cached)",
            Self::Reused => "reused",
            Self::Verified => "re-verified",
            Self::ReusedTrusted => "reused (offline)",
            Self::Reverified => "re-verified (offline)",
            Self::BundledWithNode => "bundled with node",
        }
    }
}

#[derive(Debug)]
pub(super) enum Note {
    Lock {
        created: bool,
        changes: Vec<LockChange>,
    },
    Offline,
    TrustedRepeat,
    NoLock,
    Cleanup(String),
}

/// What dependency setup did, one entry per eligible toolchain in step order.
#[derive(Debug)]
pub(super) struct DependencyReport {
    pub steps: Vec<StepOutcome>,
    pub offline: bool,
}

impl DependencyReport {
    pub fn exit_code(&self) -> i32 {
        self.failures().next().map_or(0, |(_, code)| code)
    }

    fn failures(&self) -> impl Iterator<Item = (&StepOutcome, i32)> {
        self.steps.iter().filter_map(|step| match step.result {
            StepResult::Failed { code, .. } => Some((step, code)),
            _ => None,
        })
    }
}

#[derive(Debug)]
pub(super) struct StepOutcome {
    pub toolchain: ToolchainId,
    pub result: StepResult,
}

#[derive(Debug)]
pub(super) enum StepResult {
    Skipped {
        reason: String,
    },
    Installed {
        command: String,
        elapsed: Duration,
    },
    Failed {
        command: String,
        code: i32,
        elapsed: Duration,
    },
}

#[derive(Debug)]
pub(super) enum Completed {
    /// `--plan`, `--check` and "nothing to prepare" printed their own output.
    Reported(i32),
    Provisioned(DependencyReport),
}

impl Completed {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Reported(code) => *code,
            Self::Provisioned(deps) => deps.exit_code(),
        }
    }
}

/// Rows aligned on id and version, then notes, then a blank line.
pub(super) fn render_tools(report: &ToolReport, style: Style) -> String {
    let id_width = report.rows.iter().map(|row| row.id.len()).max();
    let version_width = report.rows.iter().map(|row| row.version.len()).max();
    let (id_width, version_width) = (id_width.unwrap_or(0), version_width.unwrap_or(0));
    let mut text = String::new();
    for row in &report.rows {
        let _ = write!(
            text,
            "{:<id_width$}   {:>version_width$}  {}",
            row.id,
            row.version,
            row.outcome.label()
        );
        if let Some(source) = &row.source {
            let _ = write!(text, " {}", style.paint(&GREY, &format!("({source})")));
        }
        text.push('\n');
    }
    for note in &report.notes {
        for line in render_note(note).lines() {
            let _ = writeln!(text, "{}", style.paint(&GREY, line));
        }
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

fn render_note(note: &Note) -> String {
    const COMMIT: &str = "commit it to share these versions";
    match note {
        Note::Lock { created: true, .. } => format!("turbo.lock created, {COMMIT}"),
        Note::Lock { changes, .. } => {
            let changes = changes
                .iter()
                .map(|change| match change {
                    LockChange::Locked { id, version } => format!("{id} {version} added"),
                    LockChange::Changed { id, from, to } => format!("{id} {from} → {to}"),
                    LockChange::Reselected { id, version } => format!("{id} {version} reselected"),
                    LockChange::Removed { id, .. } => format!("{id} removed"),
                })
                .collect::<Vec<_>>();
            format!("turbo.lock updated ({}), {COMMIT}", changes.join(", "))
        }
        Note::Offline => "Offline: used only installed tools and verified cached artifacts.".into(),
        Note::TrustedRepeat => "Tools: no resolution, downloads, writes, or probes.".into(),
        Note::NoLock => "No turbo.lock written (--no-lock). Clone-local selections do not \
                         establish cross-machine reproducibility or frozen readiness.\nturbo run \
                         uses these tools in this clone only."
            .into(),
        Note::Cleanup(line) => line.clone(),
    }
}

fn seconds(elapsed: Duration) -> String {
    format!("{:.1}s", elapsed.as_secs_f64())
}

/// One finished install step. Skips render through `render_skips`.
pub(super) fn render_step(outcome: &StepOutcome, style: Style) -> String {
    let (mark, command, elapsed) = match &outcome.result {
        StepResult::Skipped { .. } => return String::new(),
        StepResult::Installed { command, elapsed } => {
            (style.paint(&BOLD_GREEN, "✓"), command, elapsed)
        }
        StepResult::Failed {
            command, elapsed, ..
        } => (style.paint(&BOLD_RED, "✗"), command, elapsed),
    };
    let elapsed = style.paint(&GREY, &seconds(*elapsed));
    format!("{mark} {command}  {elapsed}\n")
}

/// Every step skipped: one line, worded as before. Otherwise one line per
/// skipped toolchain.
pub(super) fn render_skips(report: &DependencyReport, style: Style) -> String {
    let skips = report.steps.iter().filter_map(|step| match &step.result {
        StepResult::Skipped { reason } => Some((&step.toolchain, reason.as_str())),
        _ => None,
    });
    if report
        .steps
        .iter()
        .all(|step| matches!(step.result, StepResult::Skipped { .. }))
    {
        let reasons = super::dependencies::skip_reasons(skips.map(|(_, reason)| reason));
        return format!("Dependencies skipped ({reasons}).\n");
    }
    skips
        .map(|(id, reason)| {
            let line = format!("{} dependencies skipped ({reason})", id.as_str());
            format!("{}\n", style.paint(&GREY, &line))
        })
        .collect()
}

pub(super) fn render_final(deps: &DependencyReport, elapsed: Duration, style: Style) -> String {
    let failures: Vec<_> = deps.failures().collect();
    let cross = style.paint(&BOLD_RED, "✗");
    let line = match failures.as_slice() {
        [] => format!(
            "{} Ready in {}",
            style.paint(&BOLD_GREEN, "✓"),
            seconds(elapsed)
        ),
        [(step, code)] => {
            let StepResult::Failed { command, .. } = &step.result else {
                unreachable!("failures are Failed steps")
            };
            if deps.offline {
                format!(
                    "{cross} {command} failed offline (exit {code}); run turbo setup without \
                     --offline to download what it needs"
                )
            } else {
                format!("{cross} {command} failed (exit {code})")
            }
        }
        many => {
            let ids = many
                .iter()
                .map(|(step, _)| step.toolchain.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{cross} {} installers failed offline ({ids}); run turbo setup without --offline \
                 to download what they need",
                many.len()
            )
        }
    };
    format!("{line}\n")
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Writes through a writer shared with pump threads.
struct Shared(SharedWriter);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

pub(super) struct Console {
    style: Style,
    spinner: bool,
    out: RefCell<Box<dyn Write>>,
    err: SharedWriter,
}

impl Console {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            spinner: style.mode == Mode::Interactive,
            out: RefCell::new(Box::new(io::stdout())),
            err: Arc::new(Mutex::new(Box::new(io::stderr()))),
        }
    }

    /// Both streams into buffers the test reads back; never a spinner.
    #[cfg(test)]
    pub fn capture(mode: Mode) -> (Self, Captured) {
        let captured = Captured {
            out: Arc::default(),
            err: Arc::default(),
        };
        let console = Self {
            style: Style {
                mode,
                color: ColorConfig::new(true),
            },
            spinner: false,
            out: RefCell::new(Box::new(Buffer(captured.out.clone()))),
            err: Arc::new(Mutex::new(Box::new(Buffer(captured.err.clone())))),
        };
        (console, captured)
    }

    fn out(&self, text: &str) -> io::Result<()> {
        let mut out = self.out.borrow_mut();
        out.write_all(text.as_bytes())?;
        out.flush()
    }

    pub fn tools(&self, report: &ToolReport) -> io::Result<()> {
        self.out(&render_tools(report, self.style))
    }

    pub fn skips(&self, report: &DependencyReport) -> io::Result<()> {
        self.out(&render_skips(report, self.style))
    }

    /// Prints the final line of a provisioning run and returns its exit code.
    pub fn final_line(&self, completed: &Completed, elapsed: Duration) -> io::Result<i32> {
        if let Completed::Provisioned(deps) = completed {
            self.out(&render_final(deps, elapsed, self.style))?;
        }
        Ok(completed.exit_code())
    }

    /// A spinner on stderr until the returned guard drops; none outside
    /// Interactive.
    pub fn spinner(&self, message: &str) -> Spinner {
        Spinner(self.spinner.then(|| {
            let spinner = turborepo_ui::start_spinner(message);
            Box::new(move || spinner.finish_and_clear()) as Box<dyn FnOnce()>
        }))
    }

    /// Starts one dependency install. The child keeps setup's stdio unless
    /// the runner takes `InstallStep::stdio`.
    pub fn step(
        &self,
        toolchain: ToolchainId,
        program: &'static str,
        command: String,
    ) -> InstallStep<'_> {
        InstallStep {
            console: self,
            toolchain,
            program,
            command,
            started: Instant::now(),
            pump: None,
            spinner: Spinner(None),
        }
    }
}

/// Clears its spinner when dropped or cleared.
pub(super) struct Spinner(Option<Box<dyn FnOnce()>>);

impl Spinner {
    fn clear(&mut self) {
        if let Some(clear) = self.0.take() {
            clear();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
pub(super) struct Captured {
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
}

#[cfg(test)]
impl Captured {
    pub fn out(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    #[cfg(unix)]
    pub fn err(&self) -> String {
        String::from_utf8_lossy(&self.err.lock().unwrap()).into_owned()
    }
}

#[cfg(test)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The child's stdout and stderr, both the write end of one pipe so the
/// child's own interleaving survives.
pub(super) struct ChildStdio {
    stdout: Stdio,
    stderr: Stdio,
}

impl ChildStdio {
    pub fn apply(self, command: &mut Command) -> &mut Command {
        command.stdout(self.stdout).stderr(self.stderr)
    }
}

/// Retained Interactive output; enough for any installer's error report.
const MAX_BUFFER: usize = 4 << 20;
/// How long a pipe may stay quiet after the child exits before setup stops
/// reading: a grandchild that inherited it must not hang setup.
const GRACE: Duration = Duration::from_millis(100);
/// Upper bound on reading after exit, for a grandchild that keeps writing.
const DRAIN_LIMIT: Duration = Duration::from_secs(1);

enum Sink {
    Buffer(VecDeque<u8>),
    Stream(LinePrefixer<Shared>),
}

impl Sink {
    fn write(&mut self, bytes: &[u8]) {
        match self {
            Self::Buffer(buffer) => {
                buffer.extend(bytes);
                let excess = buffer.len().saturating_sub(MAX_BUFFER);
                buffer.drain(..excess);
            }
            Self::Stream(prefixer) => {
                let _ = prefixer.write_all(bytes);
            }
        }
    }
}

struct Pump {
    sink: Arc<Mutex<Option<Sink>>>,
    /// One message per chunk read; disconnected at EOF.
    progress: mpsc::Receiver<()>,
}

/// One running dependency installer, consumed by `finish`. Dropped without
/// `finish` (an error path), it replays buffered output so the error that
/// follows keeps its context.
pub(super) struct InstallStep<'a> {
    console: &'a Console,
    toolchain: ToolchainId,
    program: &'static str,
    command: String,
    started: Instant,
    pump: Option<Pump>,
    spinner: Spinner,
}

impl InstallStep<'_> {
    /// Pipes the child's output into this step. Call once, right before
    /// spawning; the write ends then live only in the `Command`, so EOF
    /// arrives once the child and the `Command` are gone.
    pub fn stdio(&mut self) -> io::Result<ChildStdio> {
        let (mut reader, writer) = io::pipe()?;
        let stdio = ChildStdio {
            stdout: writer.try_clone()?.into(),
            stderr: writer.into(),
        };
        let sink = match self.console.style.mode {
            Mode::Interactive => Sink::Buffer(VecDeque::with_capacity(0)),
            Mode::Plain => Sink::Stream(self.prefixer()),
        };
        let sink = Arc::new(Mutex::new(Some(sink)));
        let (progress, received) = mpsc::channel();
        let pumped = sink.clone();
        std::thread::spawn(move || {
            let mut chunk = [0; 8192];
            loop {
                let read = match reader.read(&mut chunk) {
                    Ok(0) => return,
                    Ok(read) => read,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => return,
                };
                let mut sink = pumped.lock().unwrap_or_else(|e| e.into_inner());
                // Taken by `finish`: the step is over.
                let Some(sink) = sink.as_mut() else { return };
                sink.write(&chunk[..read]);
                let _ = progress.send(());
            }
        });
        self.pump = Some(Pump {
            sink,
            progress: received,
        });
        self.spinner = self.console.spinner(&self.command);
        Ok(stdio)
    }

    /// `exit` is `None` on success, else the installer's exit code. Writes
    /// the step line and, on failure, replays buffered output first.
    pub fn finish(mut self, exit: Option<i32>) -> io::Result<StepOutcome> {
        let elapsed = self.started.elapsed();
        let sink = self.stop();
        if exit.is_some() {
            self.replay(sink)?;
        }
        let command = std::mem::take(&mut self.command);
        let result = match exit {
            None => StepResult::Installed { command, elapsed },
            Some(code) => StepResult::Failed {
                command,
                code,
                elapsed,
            },
        };
        let outcome = StepOutcome {
            toolchain: self.toolchain.clone(),
            result,
        };
        self.console
            .out(&render_step(&outcome, self.console.style))?;
        Ok(outcome)
    }

    fn prefixer(&self) -> LinePrefixer<Shared> {
        LinePrefixer::new(Shared(self.console.err.clone()), self.program)
    }

    /// Stops reading the child's output and clears the spinner.
    fn stop(&mut self) -> Option<Sink> {
        let sink = self.pump.take().and_then(|pump| {
            let deadline = Instant::now() + DRAIN_LIMIT;
            while pump.progress.recv_timeout(GRACE).is_ok() && Instant::now() < deadline {}
            pump.sink.lock().unwrap_or_else(|e| e.into_inner()).take()
        });
        self.spinner.clear();
        sink
    }

    fn replay(&self, sink: Option<Sink>) -> io::Result<()> {
        match sink {
            Some(Sink::Buffer(buffer)) => {
                let mut prefixer = self.prefixer();
                let (front, back) = buffer.as_slices();
                prefixer.write_all(front)?;
                prefixer.write_all(back)?;
                prefixer.finish()
            }
            Some(Sink::Stream(mut prefixer)) => prefixer.finish(),
            None => Ok(()),
        }
    }
}

impl Drop for InstallStep<'_> {
    fn drop(&mut self) {
        let sink = self.stop();
        let _ = self.replay(sink);
    }
}

/// `{program} │ ` before every line, again after a bare `\r` so progress
/// redraws keep it.
struct LinePrefixer<W: Write> {
    inner: W,
    prefix: String,
    line_start: bool,
}

impl<W: Write> LinePrefixer<W> {
    fn new(inner: W, program: &str) -> Self {
        Self {
            inner,
            prefix: format!("{program} │ "),
            line_start: true,
        }
    }

    /// Ends a partial last line.
    fn finish(&mut self) -> io::Result<()> {
        if !self.line_start {
            self.inner.write_all(b"\n")?;
            self.line_start = true;
        }
        self.inner.flush()
    }
}

impl<W: Write> Write for LinePrefixer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = Vec::with_capacity(buf.len() + self.prefix.len());
        for &byte in buf {
            if self.line_start && byte != b'\n' {
                out.extend_from_slice(self.prefix.as_bytes());
                self.line_start = false;
            }
            out.push(byte);
            if byte == b'\n' || byte == b'\r' {
                self.line_start = true;
            }
        }
        self.inner.write_all(&out)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
