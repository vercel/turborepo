use std::marker::PhantomData;

use chrono::Local;
use owo_colors::{
    colors::{Black, Default, Red, Yellow},
    Color, OwoColorize,
};
use tracing::{field::Visit, Event, Level, Subscriber};
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
};

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
