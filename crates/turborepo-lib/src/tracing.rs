use std::{io::Stderr, marker::PhantomData, path::Path, sync::Mutex};

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

/// Note that we cannot express the type of `std::io::stderr` directly, so
/// use zero-size wrapper to call the function.
struct StdErrWrapper {}

impl<'a> MakeWriter<'a> for StdErrWrapper {
    type Writer = Stderr;

    fn make_writer(&'a self) -> Self::Writer {
        std::io::stderr()
    }
}

/// A basic logger that logs to stderr using the TurboFormatter.
/// The first generic parameter refers to the previous layer, which
/// is in this case the default layer (`Registry`).
type StdErrLog = fmt::Layer<Registry, DefaultFields, TurboFormatter, StdErrWrapper>;
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
    daemon_update: Handle<Option<DaemonLog>, StdErrLogLayered>,

    /// The non-blocking file logger only continues to log while this guard is
    /// held. We keep it here so that it doesn't get dropped.
    daemon_guard: Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,

    chrome_update: Handle<Option<ChromeLog>, DaemonLogLayered>,
    chrome_guard: Mutex<Option<tracing_chrome::FlushGuard>>,

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
    /// - If the verbosity argument (usually detemined by a flag) is provided,
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

        let stderr = fmt::layer()
            .with_writer(StdErrWrapper {})
            .event_format(TurboFormatter::new_with_ansi(
                !color_config.should_strip_ansi,
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
            daemon_update,
            daemon_guard: Mutex::new(None),
            chrome_update,
            chrome_guard: Mutex::new(None),
            #[cfg(feature = "pprof")]
            pprof_guard,
        }
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

        Ok(())
    }
}

impl Drop for TurboSubscriber {
    fn drop(&mut self) {
        // drop the guard so that the non-blocking file writer stops
        #[cfg(feature = "pprof")]
        if let Ok(report) = self.pprof_guard.report().build() {
            use std::io::Write; // only import trait if we need it

            use prost::Message;

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
    is_ansi: bool,
}

impl TurboFormatter {
    pub fn new_with_ansi(is_ansi: bool) -> Self {
        Self { is_ansi }
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

        match *level {
            Level::ERROR => {
                // The padding spaces are necessary to match the formatting of Go
                write_string::<Red, Black>(writer.by_ref(), self.is_ansi, " ERROR ")
                    .and_then(|_| write_message::<Red, Default>(writer, self.is_ansi, event))
            }
            Level::WARN => {
                // The padding spaces are necessary to match the formatting of Go
                write_string::<Yellow, Black>(writer.by_ref(), self.is_ansi, " WARNING ")
                    .and_then(|_| write_message::<Yellow, Default>(writer, self.is_ansi, event))
            }
            Level::INFO => write_message::<Default, Default>(writer, self.is_ansi, event),
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
                .and_then(|_| write_message::<Default, Default>(writer, self.is_ansi, event))
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
                let _ = write!(self.writer, "{:?}", value);
            } else {
                let _ = write!(self.writer, "{:?}", value);
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
        write!(writer, "{} ", value)
    } else {
        write!(writer, "{} ", value)
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
