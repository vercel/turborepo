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
        if let Err(err) = writeln!(
            self.out,
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
            self.err,
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
    use super::*;

    #[test]
    fn test_outputs_writes_to_out() {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let output_prefix = crate::BOLD.apply_to("output".to_string());
        let warn_prefix = crate::MAGENTA.apply_to("warn".to_string());
        {
            let mut prefixed_ui = PrefixedUI::new(
                UI::new(false),
                output_prefix,
                warn_prefix,
                &mut out,
                &mut err,
            );
            prefixed_ui.output("all good");
        }

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\u{1b}[1moutput\u{1b}[0mall good\n",
        );
    }

    #[test]
    fn test_warn_writes_to_err() {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let output_prefix = crate::BOLD.apply_to("output".to_string());
        let warn_prefix = crate::MAGENTA.apply_to("warn".to_string());
        {
            let mut prefixed_ui = PrefixedUI::new(
                UI::new(false),
                output_prefix,
                warn_prefix,
                &mut out,
                &mut err,
            );
            prefixed_ui.warn("be careful!");
        }

        assert_eq!(
            String::from_utf8(err).unwrap(),
            "\u{1b}[35mwarn\u{1b}[0mbe careful!\n"
        );
    }
}
