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

pub struct TurboFormatter;

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
        // Format values from the event's's metadata:
        let metadata = event.metadata();
        match metadata.level() {
            &Level::ERROR => write!(
                writer,
                "{} ",
                metadata.level().to_string().bg::<Red>().fg::<Black>()
            )
            .and_then(|_| write_message::<Red, Default>(writer, event)),
            &Level::WARN => write!(
                writer,
                "{} ",
                metadata.level().to_string().bg::<Yellow>().fg::<Black>(),
            )
            .and_then(|_| write_message::<Yellow, Default>(writer, event)),
            &Level::INFO => write_message::<Default, Default>(writer, event),
            // trace and debug use the same style
            _ => {
                let now = Local::now();
                write!(
                    writer,
                    "{} [{}] {}: ",
                    // build our own timestamp to match the hashicorp/go-hclog format used by the
                    // go binary
                    now.format("%Y-%m-%dT%H:%M:%S.%3f%z"),
                    metadata.level(),
                    metadata.target(),
                )
                .and_then(|_| write_message::<Default, Default>(writer, event))
            }
        }
    }
}

struct MessageVisitor<'a, FG: Color, BG: Color> {
    writer: Writer<'a>,
    _fg: PhantomData<FG>,
    _bg: PhantomData<BG>,
}

impl<'a, FG: Color, BG: Color> Visit for MessageVisitor<'a, FG, BG> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let value = value.fg::<FG>().bg::<BG>();
            let _ = write!(self.writer, "{:?}", value);
        }
    }
}

/// Writes the message field of an event to the given writer.
fn write_message<FG: Color, BG: Color>(
    mut writer: Writer<'_>,
    event: &Event,
) -> Result<(), std::fmt::Error> {
    let mut visitor = MessageVisitor::<FG, BG> {
        writer: writer.by_ref(),
        _fg: PhantomData,
        _bg: PhantomData,
    };
    event.record(&mut visitor);
    writeln!(writer)
}
