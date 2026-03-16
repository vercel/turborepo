use std::{
    io::{self, Write},
    marker::PhantomData,
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::Local;
use owo_colors::{
    colors::{Black, Default, Red, Yellow},
    Color, OwoColorize,
};
use tracing::{field::Visit, metadata::LevelFilter, trace, Event, Level, Subscriber};
use tracing_appender::{non_blocking::NonBlocking, rolling::RollingFileAppender};
use tracing_chrome::ChromeLayer;
pub use tracing_subscriber::reload::Error;
use tracing_subscriber::{
    filter::Filtered,
    fmt::{
        self,
        format::{DefaultFields, Writer},
        FmtContext, FormatEvent, FormatFields, MakeWriter,
    },
    layer,
    prelude::*,
    registry::LookupSpan,
    reload::{self, Handle},
    EnvFilter, Layer, Registry,
};
use turborepo_ui::ColorConfig;

// a lot of types to make sure we record the right relationships

/// Where the tracing stderr layer directs output.
#[derive(Clone)]
enum WriterTarget {
    Stderr,
    File(Arc<Mutex<Box<dyn Write + Send>>>),
    Null,
}

/// A switchable writer for the tracing stderr layer.
///
/// Starts writing to stderr. When the TUI is active, the target can be
/// switched to a file (with `--verbosity`) or suppressed entirely so
/// that tracing output doesn't corrupt the alternate screen.
#[derive(Clone)]
pub struct SwitchableWriter {
    target: Arc<Mutex<WriterTarget>>,
}

/// The concrete writer returned by [`SwitchableWriter::make_writer`].
pub struct SwitchableOutput {
    target: WriterTarget,
}

impl Write for SwitchableOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &self.target {
            WriterTarget::Stderr => io::stderr().write(buf),
            WriterTarget::File(f) => f.lock().unwrap().write(buf),
            WriterTarget::Null => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &self.target {
            WriterTarget::Stderr => io::stderr().flush(),
            WriterTarget::File(f) => f.lock().unwrap().flush(),
            WriterTarget::Null => Ok(()),
        }
    }
}

impl SwitchableWriter {
    fn new() -> Self {
        Self {
            target: Arc::new(Mutex::new(WriterTarget::Stderr)),
        }
    }

    fn handle(&self) -> SwitchableWriterHandle {
        SwitchableWriterHandle {
            target: self.target.clone(),
            redirect_path: Arc::new(Mutex::new(None)),
        }
    }
}

impl<'a> MakeWriter<'a> for SwitchableWriter {
    type Writer = SwitchableOutput;

    fn make_writer(&'a self) -> Self::Writer {
        SwitchableOutput {
            target: self.target.lock().unwrap().clone(),
        }
    }
}

/// Handle for switching the tracing stderr writer at runtime.
/// Stored on [`TurboSubscriber`] and exposed via its methods.
#[derive(Clone)]
pub struct SwitchableWriterHandle {
    target: Arc<Mutex<WriterTarget>>,
    redirect_path: Arc<Mutex<Option<String>>>,
}

impl SwitchableWriterHandle {
    pub fn is_stderr(&self) -> bool {
        matches!(*self.target.lock().unwrap(), WriterTarget::Stderr)
    }

    pub fn suppress(&self) {
        *self.target.lock().unwrap() = WriterTarget::Null;
    }

    pub fn redirect_to_file(&self, writer: Box<dyn Write + Send>, path: String) {
        *self.target.lock().unwrap() = WriterTarget::File(Arc::new(Mutex::new(writer)));
        *self.redirect_path.lock().unwrap() = Some(path);
    }

    pub fn redirect_path(&self) -> Option<String> {
        self.redirect_path.lock().unwrap().clone()
    }

    pub fn restore(&self) {
        *self.target.lock().unwrap() = WriterTarget::Stderr;
        *self.redirect_path.lock().unwrap() = None;
    }
}

/// A basic logger that logs to stderr using the TurboFormatter.
/// The first generic parameter refers to the previous layer, which
/// is in this case the default layer (`Registry`).
type StdErrLog = fmt::Layer<Registry, DefaultFields, TurboFormatter, SwitchableWriter>;
/// We filter this using an EnvFilter.
type StdErrLogFiltered = Filtered<StdErrLog, EnvFilter, Registry>;
/// When the `StdErrLogFiltered` is applied to the `Registry`, we get a
/// `StdErrLogLayered`, which forms the base for the next layer.
type StdErrLogLayered = layer::Layered<StdErrLogFiltered, Registry>;

/// A logger that spits lines into a file, using the standard formatter.
/// It is applied on top of the `StdErrLogLayered` layer.
type DaemonLog = fmt::Layer<StdErrLogLayered, DefaultFields, fmt::format::Format, NonBlocking>;
/// This layer can be reloaded. `None` means the layer is disabled.
type DaemonReload = reload::Layer<Option<DaemonLog>, StdErrLogLayered>;
/// We filter this using a custom filter that only logs events
/// - with evel `TRACE` or higher for the `turborepo` target
/// - with level `INFO` or higher for all other targets
type DaemonLogFiltered = Filtered<DaemonReload, EnvFilter, StdErrLogLayered>;
/// When the `DaemonLogFiltered` is applied to the `StdErrLogLayered`, we get a
/// `DaemonLogLayered`, which forms the base for the next layer.
type DaemonLogLayered = layer::Layered<DaemonLogFiltered, StdErrLogLayered>;

/// A logger that converts events to chrome tracing format and writes them
/// to a file. It is applied on top of the `DaemonLogLayered` layer.
type ChromeLog = ChromeLayer<DaemonLogLayered>;
/// This layer can be reloaded. `None` means the layer is disabled.
type ChromeReload = reload::Layer<Option<ChromeLog>, DaemonLogLayered>;
/// When the `ChromeLogFiltered` is applied to the `DaemonLogLayered`, we get a
/// `ChromeLogLayered`, which forms the base for the next layer.
type ChromeLogLayered = layer::Layered<ChromeReload, DaemonLogLayered>;

pub struct TurboSubscriber {
    stderr_handle: SwitchableWriterHandle,

    daemon_update: Handle<Option<DaemonLog>, StdErrLogLayered>,

    /// The non-blocking file logger only continues to log while this guard is
    /// held. We keep it here so that it doesn't get dropped.
    daemon_guard: Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,

    chrome_update: Handle<Option<ChromeLog>, DaemonLogLayered>,
    chrome_guard: Mutex<Option<tracing_chrome::FlushGuard>>,

    /// The resolved file path for chrome tracing output, if enabled.
    chrome_tracing_file: Mutex<Option<String>>,

    #[cfg(feature = "pprof")]
    pprof_guard: pprof::ProfilerGuard<'static>,
}

impl TurboSubscriber {
    /// Sets up the tracing subscriber, with a default stderr layer using the
    /// TurboFormatter.
    ///
    /// ## Logging behaviour:
    /// - If stdout is a terminal, we use ansi colors. Otherwise, we do not.
    /// - If the `TURBO_LOG_VERBOSITY` env var is set, it will be used to set
    ///   the verbosity level. Otherwise, the default is `WARN`. See the
    ///   documentation on the RUST_LOG env var for syntax.
    /// - If the verbosity argument (usually determined by a flag) is provided,
    ///   it overrides the default global log level. This means it overrides the
    ///   `TURBO_LOG_VERBOSITY` global setting, but not per-module settings.
    ///
    /// `TurboSubscriber` has optional loggers that can be enabled later:
    /// - `set_daemon_logger` enables logging to a file, using the standard
    ///   formatter.
    /// - `enable_chrome_tracing` enables logging to a file, using the chrome
    ///   tracing formatter.
    pub fn new_with_verbosity(verbosity: usize, color_config: &ColorConfig) -> Self {
        let level_override = match verbosity {
            0 => None,
            1 => Some(LevelFilter::INFO),
            2 => Some(LevelFilter::DEBUG),
            _ => Some(LevelFilter::TRACE),
        };

        let env_filter = |level: LevelFilter| {
            let filter = EnvFilter::builder()
                .with_default_directive(level.into())
                .with_env_var("TURBO_LOG_VERBOSITY")
                .from_env_lossy()
                .add_directive("reqwest=error".parse().unwrap())
                .add_directive("hyper=warn".parse().unwrap())
                .add_directive("h2=warn".parse().unwrap());

            if let Some(max_level) = level_override {
                filter.add_directive(max_level.into())
            } else {
                filter
            }
        };

        let switchable = SwitchableWriter::new();
        let stderr_handle = switchable.handle();

        let stderr = fmt::layer()
            .with_writer(switchable)
            .event_format(TurboFormatter::new(
                !color_config.should_strip_ansi,
                stderr_handle.clone(),
            ))
            .with_filter(env_filter(LevelFilter::WARN));

        // we set this layer to None to start with, effectively disabling it
        let (logrotate, daemon_update) = reload::Layer::new(Option::<DaemonLog>::None);
        let logrotate: DaemonLogFiltered = logrotate.with_filter(env_filter(LevelFilter::INFO));

        let (chrome, chrome_update) = reload::Layer::new(Option::<ChromeLog>::None);

        let registry = Registry::default()
            .with(stderr)
            .with(logrotate)
            .with(chrome);

        #[cfg(feature = "pprof")]
        let pprof_guard = pprof::ProfilerGuardBuilder::default()
            .frequency(1000)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .unwrap();

        registry.init();

        Self {
            stderr_handle,
            daemon_update,
            daemon_guard: Mutex::new(None),
            chrome_update,
            chrome_guard: Mutex::new(None),
            chrome_tracing_file: Mutex::new(None),
            #[cfg(feature = "pprof")]
            pprof_guard,
        }
    }

    /// Suppress the tracing stderr layer. Used when the TUI is active
    /// and verbosity is off — tracing output is silently dropped.
    pub fn suppress_stderr(&self) {
        self.stderr_handle.suppress();
    }

    /// Redirect the tracing stderr layer to a file. Used when the TUI
    /// is active but verbosity is on — tracing output goes to a file
    /// instead of corrupting the alternate screen.
    ///
    /// The file is written to `<repo_root>/.turbo/debug-logs/`.
    /// Returns the path to the log file.
    pub fn redirect_stderr_to_file(&self, repo_root: &Path) -> io::Result<String> {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let dir = repo_root.join(".turbo").join("debug-logs");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("turbo-{timestamp}.log"));
        let file = std::fs::File::create(&path)?;
        let path_str = path.to_string_lossy().into_owned();
        self.stderr_handle
            .redirect_to_file(Box::new(std::io::BufWriter::new(file)), path_str.clone());
        Ok(path_str)
    }

    /// Returns the path to the current redirect file, if active.
    pub fn stderr_redirect_path(&self) -> Option<String> {
        self.stderr_handle.redirect_path()
    }

    /// Restore the tracing stderr layer to write to stderr.
    pub fn restore_stderr(&self) {
        self.stderr_handle.restore();
    }

    /// Enables daemon logging with the specified rotation settings.
    ///
    /// Daemon logging uses the standard tracing formatter.
    #[tracing::instrument(skip(self, appender))]
    pub fn set_daemon_logger(&self, appender: RollingFileAppender) -> Result<(), Error> {
        let (file_writer, guard) = tracing_appender::non_blocking(appender);
        trace!("created non-blocking file writer");

        let layer: DaemonLog = tracing_subscriber::fmt::layer()
            .with_writer(file_writer)
            .with_ansi(false);

        self.daemon_update.reload(Some(layer))?;
        self.daemon_guard
            .lock()
            .expect("not poisoned")
            .replace(guard);

        Ok(())
    }

    /// Enables chrome tracing.
    #[tracing::instrument(skip(self, to_file))]
    pub fn enable_chrome_tracing<P: AsRef<Path>>(
        &self,
        to_file: P,
        include_args: bool,
    ) -> Result<(), Error> {
        let file_path = to_file.as_ref().to_string_lossy().to_string();

        let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
            .file(to_file)
            .include_args(include_args)
            .include_locations(true)
            .trace_style(tracing_chrome::TraceStyle::Async)
            .build();

        self.chrome_update.reload(Some(layer))?;
        self.chrome_guard
            .lock()
            .expect("not poisoned")
            .replace(guard);
        self.chrome_tracing_file
            .lock()
            .expect("not poisoned")
            .replace(file_path);

        Ok(())
    }

    /// Returns the chrome tracing output file path, if chrome tracing is
    /// enabled.
    pub fn chrome_tracing_file(&self) -> Option<String> {
        self.chrome_tracing_file
            .lock()
            .expect("not poisoned")
            .clone()
    }

    /// Flushes and closes the chrome tracing layer so the trace file is
    /// fully written. This must be called before reading the trace file
    /// for post-processing (e.g., markdown generation).
    pub fn flush_chrome_tracing(&self) -> Result<(), Error> {
        // Disable the layer by replacing it with None
        self.chrome_update.reload(None)?;
        // Drop the flush guard to finalize the file
        self.chrome_guard.lock().expect("not poisoned").take();
        Ok(())
    }
}

/// Injects process-level metadata events (version, platform, CPU count) into
/// a Chrome trace file. Call after `flush_chrome_tracing` and before any
/// post-processing that reads the trace.
pub fn inject_trace_metadata(trace_path: &Path, version: &str) -> std::io::Result<()> {
    use std::io::Write;

    let contents = std::fs::read(trace_path)?;

    // The trace file is a JSON array: [\n{event},\n{event},\n...\n]
    // Find the first newline after '[' to insert metadata events.
    let insert_pos = contents
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(1);

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let metadata = format!(
        "{{\"ph\":\"M\",\"pid\":1,\"name\":\"process_name\",\"args\":{{\"name\":\"turbo \
         {version}\"}}}},\n{{\"ph\":\"M\",\"pid\":1,\"name\":\"process_labels\",\"args\":{{\"\
         labels\":\"{platform}, {cpus} CPUs\"}}}},\n",
        version = version,
        platform = std::env::consts::OS,
        cpus = cpus,
    );

    let mut file = std::fs::File::create(trace_path)?;
    file.write_all(&contents[..insert_pos])?;
    file.write_all(metadata.as_bytes())?;
    file.write_all(&contents[insert_pos..])?;
    file.flush()?;

    Ok(())
}

impl Drop for TurboSubscriber {
    fn drop(&mut self) {
        // drop the guard so that the non-blocking file writer stops
        #[cfg(feature = "pprof")]
        if let Ok(report) = self.pprof_guard.report().build() {
            use std::io::Write;

            use pprof::protos::Message;

            let mut file = std::fs::File::create("pprof.pb").unwrap();
            let mut content = Vec::new();

            let Ok(profile) = report.pprof() else {
                tracing::error!("failed to generate pprof report");
                return;
            };
            if let Err(e) = profile.encode(&mut content) {
                tracing::error!("failed to encode pprof profile: {}", e);
            };
            if let Err(e) = file.write_all(&content) {
                tracing::error!("failed to write pprof profile: {}", e)
            };
        } else {
            tracing::error!("failed to generate pprof report")
        }
    }
}

/// The formatter for TURBOREPO
///
/// This is a port of the go formatter, which follows a few main rules:
/// - Errors are red
/// - Warnings are yellow
/// - Info is default
/// - Debug and trace are default, but with timestamp and level attached
///
/// This formatter does not print any information about spans, and does
/// not print any event metadata other than the message set when you
/// call `debug!(...)` or `info!(...)` etc.
pub struct TurboFormatter {
    default_ansi: bool,
    writer_handle: SwitchableWriterHandle,
}

impl TurboFormatter {
    pub fn new(default_ansi: bool, writer_handle: SwitchableWriterHandle) -> Self {
        Self {
            default_ansi,
            writer_handle,
        }
    }

    fn is_ansi(&self) -> bool {
        self.default_ansi && self.writer_handle.is_stderr()
    }
}

impl<S, N> FormatEvent<S, N> for TurboFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let level = event.metadata().level();
        let target = event.metadata().target();

        let is_ansi = self.is_ansi();
        match *level {
            Level::ERROR => {
                // The padding spaces are necessary to match the formatting of Go
                write_string::<Red, Black>(writer.by_ref(), is_ansi, " ERROR ")
                    .and_then(|_| write_message::<Red, Default>(writer, is_ansi, event))
            }
            Level::WARN => {
                // The padding spaces are necessary to match the formatting of Go
                write_string::<Yellow, Black>(writer.by_ref(), is_ansi, " WARNING ")
                    .and_then(|_| write_message::<Yellow, Default>(writer, is_ansi, event))
            }
            Level::INFO => write_message::<Default, Default>(writer, is_ansi, event),
            // trace and debug use the same style
            _ => {
                let now = Local::now();
                write!(
                    writer,
                    "{} [{}] {}: ",
                    // build our own timestamp to match the hashicorp/go-hclog format used by the
                    // go binary
                    now.format("%Y-%m-%dT%H:%M:%S.%3f%z"),
                    level,
                    target,
                )
                .and_then(|_| write_message::<Default, Default>(writer, is_ansi, event))
            }
        }
    }
}

/// A visitor that writes the message field of an event to the given writer.
///
/// The FG and BG type parameters are the foreground and background colors
/// to use when writing the message.
struct MessageVisitor<'a, FG: Color, BG: Color> {
    colorize: bool,
    writer: Writer<'a>,
    _fg: PhantomData<FG>,
    _bg: PhantomData<BG>,
}

impl<'a, FG: Color, BG: Color> Visit for MessageVisitor<'a, FG, BG> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            if self.colorize {
                let value = value.fg::<FG>().bg::<BG>();
                let _ = write!(self.writer, "{value:?}");
            } else {
                let _ = write!(self.writer, "{value:?}");
            }
        }
    }
}

fn write_string<FG: Color, BG: Color>(
    mut writer: Writer<'_>,
    colorize: bool,
    value: &str,
) -> Result<(), std::fmt::Error> {
    if colorize {
        let value = value.fg::<FG>().bg::<BG>();
        write!(writer, "{value} ")
    } else {
        write!(writer, "{value} ")
    }
}

/// Writes the message field of an event to the given writer.
fn write_message<FG: Color, BG: Color>(
    mut writer: Writer<'_>,
    colorize: bool,
    event: &Event,
) -> Result<(), std::fmt::Error> {
    let mut visitor = MessageVisitor::<FG, BG> {
        colorize,
        writer: writer.by_ref(),
        _fg: PhantomData,
        _bg: PhantomData,
    };
    event.record(&mut visitor);
    writeln!(writer)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write},
        sync::{Arc, Mutex},
    };

    use super::*;

    #[test]
    fn switchable_writer_starts_as_stderr() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();
        assert!(handle.is_stderr());
        assert!(handle.redirect_path().is_none());
    }

    #[test]
    fn suppress_switches_to_null() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        handle.suppress();

        assert!(!handle.is_stderr());
        assert!(handle.redirect_path().is_none());
    }

    #[test]
    fn suppress_drops_writes() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();
        handle.suppress();

        let mut output = writer.make_writer();
        let result = output.write(b"should be dropped");
        assert_eq!(result.unwrap(), 17);
        // No panic, no error — bytes are silently consumed.
    }

    #[test]
    fn redirect_to_file_captures_writes() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let buffer = Arc::new(Mutex::new(Cursor::new(Vec::new())));
        handle.redirect_to_file(
            Box::new(CursorWriter(buffer.clone())),
            "/fake/path.log".to_string(),
        );

        assert!(!handle.is_stderr());
        assert_eq!(handle.redirect_path().unwrap(), "/fake/path.log");

        let mut output = writer.make_writer();
        output.write_all(b"hello file").unwrap();
        output.flush().unwrap();

        let content = buffer.lock().unwrap().get_ref().clone();
        assert_eq!(content, b"hello file");
    }

    #[test]
    fn restore_returns_to_stderr() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        handle.suppress();
        assert!(!handle.is_stderr());

        handle.restore();
        assert!(handle.is_stderr());
        assert!(handle.redirect_path().is_none());
    }

    #[test]
    fn restore_clears_redirect_path() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let buffer = Arc::new(Mutex::new(Cursor::new(Vec::new())));
        handle.redirect_to_file(Box::new(CursorWriter(buffer)), "/some/path.log".to_string());
        assert!(handle.redirect_path().is_some());

        handle.restore();
        assert!(handle.redirect_path().is_none());
    }

    #[test]
    fn handle_clone_shares_state() {
        let writer = SwitchableWriter::new();
        let handle1 = writer.handle();
        let handle2 = handle1.clone();

        handle1.suppress();
        assert!(!handle2.is_stderr());

        handle2.restore();
        assert!(handle1.is_stderr());
    }

    #[test]
    fn formatter_reports_ansi_only_when_stderr() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let formatter = TurboFormatter::new(true, handle.clone());
        assert!(formatter.is_ansi());

        handle.suppress();
        assert!(!formatter.is_ansi());

        handle.restore();
        assert!(formatter.is_ansi());
    }

    #[test]
    fn formatter_respects_default_ansi_false() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let formatter = TurboFormatter::new(false, handle);
        // Even on stderr, if default_ansi is false, no ANSI.
        assert!(!formatter.is_ansi());
    }

    #[test]
    fn formatter_no_ansi_when_redirected_to_file() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let formatter = TurboFormatter::new(true, handle.clone());
        assert!(formatter.is_ansi());

        let buffer = Arc::new(Mutex::new(Cursor::new(Vec::new())));
        handle.redirect_to_file(Box::new(CursorWriter(buffer)), "/tmp/test.log".to_string());
        assert!(!formatter.is_ansi());
    }

    #[test]
    fn redirect_to_file_then_write_then_read_back() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("test.log");

        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let file = std::fs::File::create(&log_path).unwrap();
        handle.redirect_to_file(
            Box::new(std::io::BufWriter::new(file)),
            log_path.to_string_lossy().into_owned(),
        );

        let mut output = writer.make_writer();
        output.write_all(b"line one\nline two\n").unwrap();
        output.flush().unwrap();
        // Drop to flush BufWriter
        drop(output);
        // Switch away so the file Arc is released
        handle.restore();

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(content, "line one\nline two\n");
    }

    #[test]
    fn switching_targets_mid_stream() {
        let writer = SwitchableWriter::new();
        let handle = writer.handle();

        let buf1 = Arc::new(Mutex::new(Cursor::new(Vec::new())));
        let buf2 = Arc::new(Mutex::new(Cursor::new(Vec::new())));

        // Start redirected to buf1
        handle.redirect_to_file(Box::new(CursorWriter(buf1.clone())), "buf1".to_string());
        writer.make_writer().write_all(b"to buf1").unwrap();

        // Switch to buf2
        handle.redirect_to_file(Box::new(CursorWriter(buf2.clone())), "buf2".to_string());
        writer.make_writer().write_all(b"to buf2").unwrap();

        // Suppress
        handle.suppress();
        writer.make_writer().write_all(b"dropped").unwrap();

        assert_eq!(buf1.lock().unwrap().get_ref().as_slice(), b"to buf1");
        assert_eq!(buf2.lock().unwrap().get_ref().as_slice(), b"to buf2");
    }

    // Wrapper to make Arc<Mutex<Cursor>> implement Write.
    struct CursorWriter(Arc<Mutex<Cursor<Vec<u8>>>>);

    impl Write for CursorWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.lock().unwrap().flush()
        }
    }
}
