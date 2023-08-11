use std::{fmt::Display, io::Write};

use console::StyledObject;
use tracing::error;

use crate::UI;

/// Writes messages with different prefixes, depending on log level. Note that
/// this does output the prefix when message is empty, unlike the Go
/// implementation. We do this because this behavior is what we actually
/// want for replaying logs.
pub struct PrefixedUI<W> {
    ui: UI,
    output_prefix: StyledObject<String>,
    warn_prefix: StyledObject<String>,
    output: W,
}

impl<W: Write> PrefixedUI<W> {
    pub fn new(
        ui: UI,
        output_prefix: StyledObject<String>,
        warn_prefix: StyledObject<String>,
        output: W,
    ) -> Self {
        Self {
            ui,
            output_prefix,
            warn_prefix,
            output,
        }
    }

    pub fn output(&mut self, message: impl Display) {
        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(
            self.output,
            "{}{}",
            self.ui.apply(self.output_prefix.clone()),
            message
        ) {
            error!("cannot write to logs: {:?}", err);
        }
    }

    pub fn warn(&mut self, message: impl Display) {
        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(
            self.output,
            "{}{}",
            self.ui.apply(self.warn_prefix.clone()),
            message
        ) {
            error!("cannot write to logs: {:?}", err);
        }
    }
}

/// Wraps a writer with a prefix before the actual message.
pub struct PrefixedWriter<W> {
    prefix: StyledObject<String>,
    writer: W,
}

impl<W: Write> PrefixedWriter<W> {
    pub fn new(prefix: StyledObject<String>, writer: W) -> Self {
        Self { prefix, writer }
    }
}

impl<W: Write> Write for PrefixedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let prefix = self.prefix.to_string();
        self.writer.write(prefix.as_bytes())?;
        self.writer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
