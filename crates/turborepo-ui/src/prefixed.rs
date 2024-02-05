use std::{
    fmt::{Debug, Display, Formatter},
    io::Write,
};

use console::{Style, StyledObject};
use tracing::error;

use crate::UI;

/// Writes messages with different prefixes, depending on log level. Note that
/// this does output the prefix when message is empty, unlike the Go
/// implementation. We do this because this behavior is what we actually
/// want for replaying logs.
pub struct PrefixedUI<W> {
    ui: UI,
    output_prefix: Option<StyledObject<String>>,
    warn_prefix: Option<StyledObject<String>>,
    error_prefix: Option<StyledObject<String>>,
    out: W,
    err: W,
    default_prefix: StyledObject<String>,
}

impl<W: Write> PrefixedUI<W> {
    pub fn new(ui: UI, out: W, err: W) -> Self {
        Self {
            ui,
            out,
            err,
            output_prefix: None,
            warn_prefix: None,
            error_prefix: None,
            default_prefix: Style::new().apply_to(String::new()),
        }
    }

    pub fn with_output_prefix(mut self, output_prefix: StyledObject<String>) -> Self {
        self.output_prefix = Some(self.ui.apply(output_prefix));
        self
    }

    pub fn with_warn_prefix(mut self, warn_prefix: StyledObject<String>) -> Self {
        self.warn_prefix = Some(self.ui.apply(warn_prefix));
        self
    }

    pub fn with_error_prefix(mut self, error_prefix: StyledObject<String>) -> Self {
        self.error_prefix = Some(self.ui.apply(error_prefix));
        self
    }

    pub fn output(&mut self, message: impl Display) {
        self.write_line(message, Command::Output)
    }

    pub fn warn(&mut self, message: impl Display) {
        self.write_line(message, Command::Warn)
    }

    pub fn error(&mut self, message: impl Display) {
        self.write_line(message, Command::Error)
    }

    fn write_line(&mut self, message: impl Display, command: Command) {
        let prefix = match command {
            Command::Output => &self.output_prefix,
            Command::Warn => &self.warn_prefix,
            Command::Error => &self.error_prefix,
        }
        .as_ref()
        .unwrap_or(&self.default_prefix);
        let writer = match command {
            Command::Output => &mut self.out,
            Command::Warn | Command::Error => &mut self.err,
        };

        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(writer, "{}{}", prefix, message) {
            error!("cannot write to logs: {:?}", err);
        }
    }

    /// Construct a PrefixedWriter which will behave the same as `output`, but
    /// without the requirement that messages be valid UTF-8
    pub(crate) fn output_prefixed_writer(&mut self) -> PrefixedWriter<&mut W> {
        PrefixedWriter {
            prefix: self
                .output_prefix
                .as_ref()
                .map(|prefix| prefix.to_string())
                .unwrap_or_default(),
            writer: &mut self.out,
        }
    }
}

//
#[derive(Debug, Clone, Copy)]
enum Command {
    Output,
    Warn,
    Error,
}

/// Wraps a writer with a prefix before the actual message.
pub struct PrefixedWriter<W> {
    prefix: String,
    writer: W,
}

impl<W> Debug for PrefixedWriter<W> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrefixedWriter")
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl<W: Write> PrefixedWriter<W> {
    pub fn new(ui: UI, prefix: StyledObject<impl Display>, writer: W) -> Self {
        let prefix = ui.apply(prefix).to_string();
        Self { prefix, writer }
    }
}

impl<W: Write> Write for PrefixedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut is_first = true;
        for chunk in buf.split_inclusive(|c| *c == b'\r') {
            // Before we write the chunk we write the prefix as either:
            // - this is the first iteration and we haven't written the prefix
            // - the previous chunk ended with a \r and the cursor is currently as the start
            //   of the line so we want to rewrite the prefix over the existing prefix in
            //   the line
            // or if the last chunk is just a newline we can skip rewriting the prefix
            if is_first || chunk != b"\n" {
                self.writer.write_all(self.prefix.as_bytes())?;
            }
            self.writer.write_all(chunk)?;
            is_first = false;
        }
        // We do end up writing more bytes than this to the underlying writer, but we
        // cannot report this to the callers as the amount of bytes we report
        // written must be less than or equal to the number of bytes in the buffer.
        Ok(buf.len())
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
        PrefixedUI::new(ui, out, err)
            .with_output_prefix(output_prefix)
            .with_warn_prefix(warn_prefix)
            .with_error_prefix(crate::MAGENTA.apply_to("error ".to_string()))
    }

    #[test_case(false, "\u{1b}[1moutput \u{1b}[0mall good\n", Command::Output)]
    #[test_case(true, "output all good\n", Command::Output)]
    #[test_case(false, "\u{1b}[35mwarn \u{1b}[0mbe careful!\n", Command::Warn)]
    #[test_case(true, "warn be careful!\n", Command::Warn)]
    #[test_case(false, "\u{1b}[35merror \u{1b}[0mit blew up\n", Command::Error)]
    #[test_case(true, "error it blew up\n", Command::Error)]
    fn test_prefix_ui_outputs(strip_ansi: bool, expected: &str, cmd: Command) {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut prefixed_ui = prefixed_ui(&mut out, &mut err, UI::new(strip_ansi));
        match cmd {
            Command::Output => prefixed_ui.output("all good"),
            Command::Warn => prefixed_ui.warn("be careful!"),
            Command::Error => prefixed_ui.error("it blew up"),
        }

        let buffer = match cmd {
            Command::Output => out,
            Command::Warn | Command::Error => err,
        };
        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }

    #[test_case(true, "foo#build: cool!")]
    #[test_case(false, "\u{1b}[1mfoo#build: \u{1b}[0mcool!")]
    fn test_prefixed_writer(strip_ansi: bool, expected: &str) {
        let mut buffer = Vec::new();
        let mut writer = PrefixedWriter::new(
            UI::new(strip_ansi),
            crate::BOLD.apply_to("foo#build: "),
            &mut buffer,
        );
        writer.write_all(b"cool!").unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }

    #[test_case("\ra whole message \n", "turbo > \rturbo > a whole message \n" ; "basic prefix cr")]
    #[test_case("no return", "turbo > no return" ; "no return")]
    #[test_case("foo\rbar\rbaz", "turbo > foo\rturbo > bar\rturbo > baz" ; "multiple crs")]
    #[test_case("foo\r", "turbo > foo\r" ; "trailing cr")]
    #[test_case("foo\r\n", "turbo > foo\r\n" ; "no double write on crlf")]
    #[test_case("\n", "turbo > \n" ; "leading new line")]
    fn test_prefixed_writer_cr(input: &str, expected: &str) {
        let mut buffer = Vec::new();
        let mut writer = PrefixedWriter::new(
            UI::new(false),
            Style::new().apply_to("turbo > "),
            &mut buffer,
        );

        writer.write_all(input.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }
}
