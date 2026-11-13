//! Terminal output for provisioning runs. Phases return values, `render_*`
//! turns them into text, and `Console` is the only writer of stdout and
//! stderr. Tool rows, notes, step lines and the final line go to stdout; the
//! spinner, child output and failure replays go to stderr.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
    fmt::Write as _,
    io::{self, IsTerminal, Read, Write},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use turborepo_repository::toolchain::ToolchainId;
use turborepo_ui::{BOLD_GREEN, BOLD_RED, ColorConfig, GREY, YELLOW};

use super::provision::LockAction;

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
    /// The version of each tool this clone used before this run, by id: from
    /// setup's record, else the turbo.lock this run replaced. Empty when
    /// unknown, so nothing reads as changed or removed.
    pub previous: BTreeMap<String, String>,
    pub notes: Vec<Note>,
    /// Set when this run wrote turbo.lock.
    pub lock_written: Option<LockReminder>,
    pub downloaded_bytes: u64,
}

impl ToolReport {
    pub fn installed(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.outcome == ToolOutcome::Installed)
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolOutcome {
    Installed,
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

/// One line of the tool section, in display order. A version change outranks
/// how the new version arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Change {
    Upgraded,
    Downgraded,
    /// The versions do not compare, such as Rust's `stable` and `nightly`.
    Changed,
    Installed,
    Verified,
    Reused,
    Removed,
}

impl Change {
    fn label(self) -> &'static str {
        match self {
            Self::Upgraded => "Upgraded",
            Self::Downgraded => "Downgraded",
            Self::Changed => "Changed",
            Self::Installed => "Installed",
            Self::Verified => "Verified",
            Self::Reused => "Reused",
            Self::Removed => "Removed",
        }
    }

    fn between(from: &str, to: &str) -> Self {
        fn parts(version: &str) -> Option<Vec<u64>> {
            let version = version.strip_prefix('v').unwrap_or(version);
            version.split('.').map(|part| part.parse().ok()).collect()
        }
        match parts(from).zip(parts(to)).map(|(from, to)| from.cmp(&to)) {
            Some(Ordering::Less) => Self::Upgraded,
            Some(Ordering::Greater) => Self::Downgraded,
            _ => Self::Changed,
        }
    }
}

/// Why turbo.lock needs a commit: this run wrote it, or Git still reports it
/// new or modified. Printed after the final line, so it is the last thing
/// people read.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LockReminder {
    /// No committed turbo.lock exists yet.
    Created,
    /// The committed turbo.lock has changes.
    Updated,
}

#[derive(Debug)]
pub(super) enum Note {
    Offline,
    TrustedRepeat,
    NoLock,
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
    Provisioned {
        deps: DependencyReport,
        lock: Option<LockReminder>,
    },
}

impl Completed {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Reported(code) => *code,
            Self::Provisioned { deps, .. } => deps.exit_code(),
        }
    }
}

/// Each line's tools as `id version` items, grouped by what happened to them.
/// Bundled npm rides along with node, since it changes only with node.
fn tool_lines(report: &ToolReport) -> BTreeMap<Change, Vec<String>> {
    let has_node = report.rows.iter().any(|row| row.id == "node");
    let bundled = report
        .rows
        .iter()
        .find(|row| row.outcome == ToolOutcome::BundledWithNode);
    let mut lines: BTreeMap<Change, Vec<String>> = BTreeMap::new();
    for row in &report.rows {
        if row.outcome == ToolOutcome::BundledWithNode && has_node {
            continue;
        }
        let (change, mut item) = match report.previous.get(&row.id) {
            Some(from) if *from != row.version => (
                Change::between(from, &row.version),
                format!("{} {from} → {}", row.id, row.version),
            ),
            _ => (
                match row.outcome {
                    ToolOutcome::Installed => Change::Installed,
                    ToolOutcome::Verified | ToolOutcome::Reverified => Change::Verified,
                    ToolOutcome::Reused
                    | ToolOutcome::ReusedTrusted
                    | ToolOutcome::BundledWithNode => Change::Reused,
                },
                format!("{} {}", row.id, row.version),
            ),
        };
        if let Some(npm) = bundled.filter(|_| row.id == "node") {
            let _ = write!(item, " (npm {})", npm.version);
        }
        lines.entry(change).or_default().push(item);
    }
    for (id, version) in &report.previous {
        if !report.rows.iter().any(|row| row.id == *id) {
            lines
                .entry(Change::Removed)
                .or_default()
                .push(format!("{id} {version}"));
        }
    }
    lines
}

/// Comma-separated items, wrapped to `width` with each continuation line
/// indented to `indent`, the column the first item starts at.
fn wrap(items: &[String], indent: usize, width: Option<usize>) -> String {
    let mut text = String::new();
    let mut column = indent;
    for (index, item) in items.iter().enumerate() {
        let piece = if index + 1 == items.len() {
            item.clone()
        } else {
            format!("{item},")
        };
        let length = piece.chars().count();
        if index > 0 {
            if width.is_some_and(|width| column + 1 + length > width) {
                text.push('\n');
                text.push_str(&" ".repeat(indent));
                column = indent;
            } else {
                text.push(' ');
                column += 1;
            }
        }
        text.push_str(&piece);
        column += length;
    }
    text
}

/// One `✓ Label  tools` line per outcome, then notes, then a blank line.
/// `width` is the terminal's, when stdout is one.
pub(super) fn render_tools(report: &ToolReport, style: Style, width: Option<usize>) -> String {
    let lines = tool_lines(report);
    let label_width = lines.keys().map(|change| change.label().len()).max();
    let label_width = label_width.unwrap_or(0);
    let check = style.paint(&BOLD_GREEN, "✓");
    let mut text = String::new();
    for (change, items) in &lines {
        let _ = writeln!(
            text,
            "{check} {:<label_width$}  {}",
            change.label(),
            wrap(items, 2 + label_width + 2, width)
        );
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

/// The one line that asks for action: a yellow `!` and unstyled text. The tool
/// section already lists what changed.
pub(super) fn render_lock_reminder(lock: &LockReminder, style: Style) -> String {
    let what = match lock {
        LockReminder::Created => "Created turbo.lock.",
        LockReminder::Updated => "Updated turbo.lock.",
    };
    format!(
        "{} {what} Commit it to stabilize tooling versions for the repository.\n",
        style.paint(&YELLOW, "!")
    )
}

fn render_note(note: &Note) -> String {
    match note {
        Note::Offline => "Offline: used only tools already in the tool store.".into(),
        Note::TrustedRepeat => "Tools: no resolution, downloads, writes, or probes.".into(),
        Note::NoLock => "No turbo.lock written (--no-lock). Clone-local selections do not \
                         establish cross-machine reproducibility or frozen readiness.\nturbo run \
                         uses these tools in this clone only."
            .into(),
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
    format!(
        "{mark} {command}  {}\n",
        style.paint(&GREY, &seconds(*elapsed))
    )
}

/// `– Skipped …` with the reason as grey detail: one line when every step
/// skipped, else one per skipped toolchain.
pub(super) fn render_skips(report: &DependencyReport, style: Style) -> String {
    let dash = style.paint(&GREY, "–");
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
        return format!(
            "{dash} Skipped dependencies  {}\n",
            style.paint(&GREY, &reasons)
        );
    }
    skips
        .map(|(id, reason)| {
            format!(
                "{dash} Skipped {} dependencies  {}\n",
                id.as_str(),
                style.paint(&GREY, reason)
            )
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

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).flush()
    }
}

pub(super) struct Console {
    style: Style,
    spinner: bool,
    /// stdout's terminal width, for wrapping the tool section.
    width: Option<usize>,
    out: Mutex<Box<dyn Write + Send>>,
    err: SharedWriter,
    installs: Mutex<RunningInstalls>,
}

#[derive(Default)]
struct RunningInstalls {
    running: Vec<String>,
    spinner: Spinner,
}

impl RunningInstalls {
    fn message(&self) -> String {
        self.running.join(", ")
    }
}

impl Console {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            spinner: style.mode == Mode::Interactive,
            width: console::Term::stdout()
                .size_checked()
                .map(|(_, columns)| usize::from(columns)),
            out: Mutex::new(Box::new(io::stdout())),
            err: Arc::new(Mutex::new(Box::new(io::stderr()))),
            installs: Mutex::default(),
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
            width: None,
            out: Mutex::new(Box::new(Buffer(captured.out.clone()))),
            err: Arc::new(Mutex::new(Box::new(Buffer(captured.err.clone())))),
            installs: Mutex::default(),
        };
        (console, captured)
    }

    fn out(&self, text: &str) -> io::Result<()> {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        out.write_all(text.as_bytes())?;
        out.flush()
    }

    fn installs(&self) -> std::sync::MutexGuard<'_, RunningInstalls> {
        self.installs.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn show_on_spinner(&self, command: &str) {
        let mut installs = self.installs();
        installs.running.push(command.to_owned());
        let message = installs.message();
        match installs.spinner.0 {
            None => installs.spinner = self.spinner(&message),
            Some(_) => installs.spinner.set_message(&message),
        }
    }

    fn drop_from_spinner(&self, command: &str) {
        let mut installs = self.installs();
        if let Some(index) = installs.running.iter().position(|run| run == command) {
            installs.running.remove(index);
        }
        match installs.running.is_empty() {
            true => installs.spinner.clear(),
            false => {
                let message = installs.message();
                installs.spinner.set_message(&message);
            }
        }
    }

    fn exclusively_above_spinner<T>(&self, write: impl FnOnce() -> T) -> T {
        self.installs().spinner.suspend(write)
    }

    pub fn tools(&self, report: &ToolReport) -> io::Result<()> {
        self.out(&render_tools(report, self.style, self.width))
    }

    pub fn skips(&self, report: &DependencyReport) -> io::Result<()> {
        self.out(&render_skips(report, self.style))
    }

    /// Prints the final line of a provisioning run, set apart by blank lines,
    /// and returns its exit code.
    pub fn final_line(&self, completed: &Completed, elapsed: Duration) -> io::Result<i32> {
        if let Completed::Provisioned { deps, lock } = completed {
            self.out(&format!("\n{}", render_final(deps, elapsed, self.style)))?;
            if let Some(lock) = lock {
                self.out(&format!("\n{}", render_lock_reminder(lock, self.style)))?;
            }
        }
        Ok(completed.exit_code())
    }

    /// For error paths that skip the final line: turbo.lock still needs a
    /// commit.
    pub fn lock_reminder(&self, lock: Option<&LockReminder>) -> io::Result<()> {
        lock.map_or(Ok(()), |lock| {
            self.out(&render_lock_reminder(lock, self.style))
        })
    }

    /// A spinner on stderr until the returned guard drops; none outside
    /// Interactive.
    pub fn spinner(&self, message: &str) -> Spinner {
        Spinner(self.spinner.then(|| {
            let spinner = turborepo_ui::start_spinner(message);
            let (messages, suspends) = (spinner.clone(), spinner.clone());
            ActiveSpinner {
                clear: Box::new(move || spinner.finish_and_clear()),
                set_message: Box::new(move |message| messages.set_message(message.to_owned())),
                suspend: Box::new(move |write| suspends.suspend(write)),
            }
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
            captured: None,
            on_spinner: false,
        }
    }
}

/// Clears its spinner when dropped or cleared.
#[derive(Default)]
pub(super) struct Spinner(Option<ActiveSpinner>);

struct ActiveSpinner {
    clear: Box<dyn FnOnce() + Send>,
    set_message: Box<dyn Fn(&str) + Send>,
    suspend: Suspend,
}

type Suspend = Box<dyn Fn(&mut dyn FnMut()) + Send>;

impl Spinner {
    pub(super) fn set_message(&self, message: &str) {
        if let Some(active) = &self.0 {
            (active.set_message)(message);
        }
    }

    fn suspend<T>(&self, write: impl FnOnce() -> T) -> T {
        let Some(active) = &self.0 else {
            return write();
        };
        let (mut write, mut written) = (Some(write), None);
        (active.suspend)(&mut || written = write.take().map(|write| write()));
        match written {
            Some(written) => written,
            None => unreachable!("suspend runs its closure once"),
        }
    }

    fn clear(&mut self) {
        if let Some(active) = self.0.take() {
            (active.clear)();
        }
    }
}

/// The spinner message while turbo.lock is resolved and written.
pub(super) fn lock_message<'a>(
    action: LockAction,
    tools: impl IntoIterator<Item = &'a str>,
) -> String {
    let verb = match action {
        LockAction::Create => "Creating",
        LockAction::Update => "Updating",
    };
    let tools: Vec<_> = tools.into_iter().collect();
    match tools.as_slice() {
        [] => format!("{verb} turbo.lock"),
        [one] => format!("{verb} turbo.lock for {one}"),
        [first, second] => format!("{verb} turbo.lock for {first} and {second}"),
        [rest @ .., last] => format!("{verb} turbo.lock for {}, and {last}", rest.join(", ")),
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
    /// Output a runner captured itself, handed over through `output`.
    captured: Option<Sink>,
    on_spinner: bool,
}

impl InstallStep<'_> {
    fn sink(&self) -> Sink {
        match self.console.style.mode {
            Mode::Interactive => Sink::Buffer(VecDeque::with_capacity(0)),
            Mode::Plain => Sink::Stream(self.prefixer()),
        }
    }

    fn start(&mut self) {
        if !std::mem::replace(&mut self.on_spinner, true) {
            self.console.show_on_spinner(&self.command);
        }
    }

    /// A spinner for a runner that captures its child's output itself.
    pub fn spin(&mut self) {
        self.start();
    }

    /// Output a runner captured itself, shown as piped output is: streamed
    /// in Plain, replayed only on failure in Interactive.
    pub fn output(&mut self, bytes: &[u8]) {
        let mut sink = self.sink();
        sink.write(bytes);
        self.captured = Some(sink);
    }

    /// Pipes the child's output into this step. Call once, right before
    /// spawning; the write ends then live only in the `Command`, so EOF
    /// arrives once the child and the `Command` are gone.
    pub fn stdio(&mut self) -> io::Result<ChildStdio> {
        let (mut reader, writer) = io::pipe()?;
        let stdio = ChildStdio {
            stdout: writer.try_clone()?.into(),
            stderr: writer.into(),
        };
        let sink = Arc::new(Mutex::new(Some(self.sink())));
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
        self.start();
        Ok(stdio)
    }

    /// `exit` is `None` on success, else the installer's exit code. Writes
    /// the step line and, on failure, replays buffered output first.
    pub fn finish(mut self, exit: Option<i32>) -> io::Result<StepOutcome> {
        let elapsed = self.started.elapsed();
        let sink = self.stop();
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
        self.console.exclusively_above_spinner(|| {
            if exit.is_some() || matches!(&sink, Some(Sink::Stream(_))) {
                self.replay(sink)?;
            }
            self.console.out(&render_step(&outcome, self.console.style))
        })?;
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
        if std::mem::take(&mut self.on_spinner) {
            self.console.drop_from_spinner(&self.command);
        }
        sink.or_else(|| self.captured.take())
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
        let _ = self.console.exclusively_above_spinner(|| self.replay(sink));
    }
}

const MAX_PENDING_LINE: usize = 64 << 10;

/// `{program} │ ` before every line, again after a bare `\r` so progress
/// redraws keep it.
struct LinePrefixer<W: Write> {
    inner: W,
    prefix: String,
    line_start: bool,
    pending: Vec<u8>,
}

impl<W: Write> LinePrefixer<W> {
    fn new(inner: W, program: &str) -> Self {
        Self {
            inner,
            prefix: format!("{program} │ "),
            line_start: true,
            pending: Vec::new(),
        }
    }

    /// Ends a partial last line.
    fn finish(&mut self) -> io::Result<()> {
        if !self.line_start {
            self.pending.push(b'\n');
            self.line_start = true;
        }
        self.inner.write_all(&self.pending)?;
        self.pending.clear();
        self.inner.flush()
    }
}

impl<W: Write> Write for LinePrefixer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            if self.line_start && byte != b'\n' {
                self.pending.extend_from_slice(self.prefix.as_bytes());
                self.line_start = false;
            }
            self.pending.push(byte);
            if byte == b'\n' || byte == b'\r' {
                self.line_start = true;
            }
        }
        let ended = match self.pending.len() > MAX_PENDING_LINE {
            true => Some(self.pending.len()),
            false => (self.pending.iter())
                .rposition(|byte| matches!(byte, b'\n' | b'\r'))
                .map(|end| end + 1),
        };
        if let Some(ended) = ended {
            self.inner.write_all(&self.pending[..ended])?;
            self.pending.drain(..ended);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
