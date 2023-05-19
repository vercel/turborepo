use std::{fmt::Write, io::Write as _, marker::PhantomData, thread, time::Instant};

use tracing::{
    field::{display, Visit},
    span, Subscriber,
};
use tracing_subscriber::{fmt::MakeWriter, registry::LookupSpan, Layer};

use crate::tracing::{TraceRow, TraceValue};

/// A tracing layer that writes raw trace data to a writer. The data format is
/// defined by [FullTraceRow].
pub struct RawTraceLayer<
    W: for<'span> MakeWriter<'span> + 'static,
    S: Subscriber + for<'a> LookupSpan<'a>,
> {
    make_writer: W,
    start: Instant,
    _phantom: PhantomData<fn(S)>,
}

impl<W: for<'span> MakeWriter<'span>, S: Subscriber + for<'a> LookupSpan<'a>> RawTraceLayer<W, S> {
    pub fn new(make_writer: W) -> Self {
        Self {
            make_writer,
            start: Instant::now(),
            _phantom: PhantomData,
        }
    }

    fn write(&self, data: TraceRow<'_>) {
        let mut writer = self.make_writer.make_writer();
        // Small rows can be serialized on stack
        match postcard::to_vec::<_, 4096>(&data) {
            Ok(buf) => {
                let _ = writer.write_all(&buf);
            }
            Err(_) => {
                // Fallback to heap allocated buffer
                let buf = postcard::to_allocvec(&data).unwrap();
                let _ = writer.write_all(&buf);
            }
        }
    }
}

impl<W: for<'span> MakeWriter<'span>, S: Subscriber + for<'a> LookupSpan<'a>> Layer<S>
    for RawTraceLayer<W, S>
{
    fn on_new_span(
        &self,
        attrs: &span::Attributes<'_>,
        id: &span::Id,
        ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let ts = self.start.elapsed().as_micros() as u64;
        let mut values = ValuesVisitor::new();
        attrs.values().record(&mut values);
        self.write(TraceRow::Start {
            ts,
            id: id.into_u64(),
            parent: if attrs.is_contextual() {
                ctx.current_span().id().map(|p| p.into_u64())
            } else {
                attrs.parent().map(|p| p.into_u64())
            },
            name: attrs.metadata().name(),
            target: attrs.metadata().target(),
            values: values.values,
        });
    }

    fn on_close(&self, id: span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let ts = self.start.elapsed().as_micros() as u64;
        self.write(TraceRow::End {
            ts,
            id: id.into_u64(),
        });
    }

    fn on_enter(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let ts = self.start.elapsed().as_micros() as u64;
        let thread_id = thread::current().id().as_u64().into();
        self.write(TraceRow::Enter {
            ts,
            id: id.into_u64(),
            thread_id,
        });
    }

    fn on_exit(&self, id: &span::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let ts = self.start.elapsed().as_micros() as u64;
        self.write(TraceRow::Exit {
            ts,
            id: id.into_u64(),
        });
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        let ts = self.start.elapsed().as_micros() as u64;
        let mut values = ValuesVisitor::new();
        event.record(&mut values);
        self.write(TraceRow::Event {
            ts,
            parent: if event.is_contextual() {
                ctx.current_span().id().map(|p| p.into_u64())
            } else {
                event.parent().map(|p| p.into_u64())
            },
            values: values.values,
        });
    }
}

struct ValuesVisitor {
    values: Vec<(String, TraceValue<'static>)>,
}

impl ValuesVisitor {
    fn new() -> Self {
        Self { values: Vec::new() }
    }
}

impl Visit for ValuesVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let mut str = String::new();
        let _ = write!(str, "{:?}", value);
        self.values
            .push((field.name().to_string(), TraceValue::String(str.into())));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.values
            .push((field.name().to_string(), TraceValue::Float(value)));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.values
            .push((field.name().to_string(), TraceValue::Int(value)));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.values
            .push((field.name().to_string(), TraceValue::UInt(value)));
    }

    fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
        self.record_debug(field, &value)
    }

    fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
        self.record_debug(field, &value)
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.values
            .push((field.name().to_string(), TraceValue::Bool(value)));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.values.push((
            field.name().to_string(),
            TraceValue::String(value.to_string().into()),
        ));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_debug(field, &display(value))
    }
}
