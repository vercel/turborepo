use std::{marker::PhantomData, sync::Mutex};

use chrono::Local;
use owo_colors::{
    colors::{Black, Default, Red, Yellow},
    Color, OwoColorize,
};
use tracing::{field::Visit, metadata::LevelFilter, trace, Event, Level, Subscriber};
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling::RollingFileAppender,
};
use tracing_subscriber::{
    filter::Filtered,
    fmt::{
        self,
        format::{DefaultFields, Writer},
        FmtContext, FormatEvent, FormatFields,
    },
    prelude::*,
    registry::LookupSpan,
    reload::{self, Error, Handle},
    EnvFilter, Layer, Registry,
};

use crate::ui::UI;

type StdOutLog = Filtered<
    tracing_subscriber::fmt::Layer<Registry, DefaultFields, TurboFormatter>,
    EnvFilter,
    Registry,
>;

type DaemonLog = tracing_subscriber::fmt::Layer<
    Layered,
    DefaultFields,
    tracing_subscriber::fmt::format::Format,
    NonBlocking,
>;

type Layered = tracing_subscriber::layer::Layered<StdOutLog, Registry>;

pub struct TurboSubscriber {
    update: Handle<Option<DaemonLog>, Layered>,

    /// The non-blocking file logger only continues to log while this guard is
    /// held. We keep it here so that it doesn't get dropped.
    guard: Mutex<Option<WorkerGuard>>,

    #[cfg(feature = "tracing-chrome")]
    chrome_guard: tracing_chrome::FlushGuard,
}

impl TurboSubscriber {
    /// Sets up the tracing subscriber, with a default stdout layer using the
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
    /// Returns a `reload::Handle` that can be used to reload the subscriber.
    /// This allows us to register additional layers after setup, for example
    /// when configuring logrotation in the daemon.
    pub fn new_with_verbosity(verbosity: usize, ui: &UI) -> Self {
        let level_override = match verbosity {
            0 => None,
            1 => Some(LevelFilter::INFO),
            2 => Some(LevelFilter::DEBUG),
            _ => Some(LevelFilter::TRACE),
        };

        let filter = EnvFilter::builder()
            .with_default_directive(LevelFilter::WARN.into())
            .with_env_var("TURBO_LOG_VERBOSITY")
            .from_env_lossy();

        let filter = if let Some(max_level) = level_override {
            filter.add_directive(max_level.into())
        } else {
            filter
        };

        let stdout = fmt::layer()
            .event_format(TurboFormatter::new_with_ansi(!ui.should_strip_ansi))
            .with_filter(filter);

        // we set this layer to None to start with, effectively disabling it
        let (logrotate, update) = reload::Layer::new(Option::<DaemonLog>::None);

        let registry = Registry::default().with(stdout).with(logrotate);

        #[cfg(feature = "tracing-chrome")]
        let (registry, chrome_guard) = {
            let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
                .file("./tracing.json")
                .build();
            (registry.with(chrome_layer), guard)
        };

        registry.init();

        Self {
            update,
            guard: Mutex::new(None),
            #[cfg(feature = "tracing-chrome")]
            chrome_guard,
        }
    }

    /// Enables daemon logging with the specified rotation settings.
    ///
    /// Daemon logging uses the standard tracing formatter.
    #[tracing::instrument(skip(self))]
    pub fn set_daemon_logger(&self, appender: RollingFileAppender) -> Result<(), Error> {
        let (file_writer, guard) = tracing_appender::non_blocking(appender);
        trace!("created non-blocking file writer");

        let layer = tracing_subscriber::fmt::layer()
            .with_writer(file_writer)
            .with_ansi(false);

        self.update.reload(Some(layer))?;
        self.guard.lock().expect("not poisoned").replace(guard);

        Ok(())
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
                write_string::<Red, Black>(writer.by_ref(), self.is_ansi, level.as_str())
                    .and_then(|_| write_message::<Red, Default>(writer, self.is_ansi, event))
            }
            Level::WARN => {
                write_string::<Yellow, Black>(writer.by_ref(), self.is_ansi, level.as_str())
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
