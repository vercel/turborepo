use std::{
    fmt::{Debug, Display, Formatter},
    io::Write,
};

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
    out: W,
    err: W,
}

impl<W: Write> PrefixedUI<W> {
    pub fn new(
        ui: UI,
        output_prefix: StyledObject<String>,
        warn_prefix: StyledObject<String>,
        out: W,
        err: W,
    ) -> Self {
        let output_prefix = ui.apply(output_prefix);
        let warn_prefix = ui.apply(warn_prefix);
        Self {
            ui,
            output_prefix,
            warn_prefix,
            out,
            err,
        }
    }

    pub fn output(&mut self, message: impl Display) {
        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(self.out, "{}{}", self.output_prefix, message) {
            error!("cannot write to logs: {:?}", err);
        }
    }

    pub fn warn(&mut self, message: impl Display) {
        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(self.err, "{}{}", self.warn_prefix, message) {
            error!("cannot write to logs: {:?}", err);
        }
    }
}

/// Wraps a writer with a prefix before the actual message.
pub struct PrefixedWriter<W> {
    prefix: StyledObject<String>,
    writer: W,
    ui: UI,
}

impl<W> Debug for PrefixedWriter<W> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefixedWriter")
            .field("prefix", &self.prefix)
            .field("ui", &self.ui)
            .finish()
    }
}

impl<W: Write> PrefixedWriter<W> {
    pub fn new(ui: UI, prefix: StyledObject<String>, writer: W) -> Self {
        Self { ui, prefix, writer }
    }
}

impl<W: Write> Write for PrefixedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let prefix = self.prefix.clone();
        let prefix = self.ui.apply(prefix);
        let prefix_bytes_written = self.writer.write(prefix.to_string().as_bytes())?;

        Ok(prefix_bytes_written + self.writer.write(buf)?)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use super::*;

    fn prefixed_ui<W: Write>(out: W, err: W, ui: UI) -> PrefixedUI<W> {
        let output_prefix = crate::BOLD.apply_to("output ".to_string());
        let warn_prefix = crate::MAGENTA.apply_to("warn ".to_string());
        PrefixedUI::new(ui, output_prefix, warn_prefix, out, err)
    }

    #[derive(Debug, Clone, Copy)]
    enum OutputCall {
        Output,
        Warn,
    }

    #[test_case(false, "\u{1b}[1moutput \u{1b}[0mall good\n", OutputCall::Output)]
    #[test_case(true, "output all good\n", OutputCall::Output)]
    #[test_case(false, "\u{1b}[35mwarn \u{1b}[0mbe careful!\n", OutputCall::Warn)]
    #[test_case(true, "warn be careful!\n", OutputCall::Warn)]
    fn test_prefix_ui_outputs(strip_ansi: bool, expected: &str, call: OutputCall) {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut prefixed_ui = prefixed_ui(&mut out, &mut err, UI::new(strip_ansi));
        match call {
            OutputCall::Output => prefixed_ui.output("all good"),
            OutputCall::Warn => prefixed_ui.warn("be careful!"),
        }

        let buffer = match call {
            OutputCall::Output => out,
            OutputCall::Warn => err,
        };
        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }
}
