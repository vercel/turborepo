use std::{
    fmt::{Debug, Display},
    io::Write,
};

use console::{Style, StyledObject};
use tracing::error;

use crate::{ColorConfig, LineWriter};

/// Writes messages with different prefixes, depending on log level.
///
/// Note that this does output the prefix when message is empty, unlike the Go
/// implementation. We do this because this behavior is what we actually
/// want for replaying logs.
pub struct PrefixedUI<W> {
    color_config: ColorConfig,
    output_prefix: Option<StyledObject<String>>,
    warn_prefix: Option<StyledObject<String>>,
    error_prefix: Option<StyledObject<String>>,
    out: W,
    err: W,
    default_prefix: StyledObject<String>,
    include_timestamps: bool,
}

impl<W: Write> PrefixedUI<W> {
    pub fn new(color_config: ColorConfig, out: W, err: W) -> Self {
        Self {
            color_config,
            out,
            err,
            output_prefix: None,
            warn_prefix: None,
            error_prefix: None,
            default_prefix: Style::new().apply_to(String::new()),
            include_timestamps: false,
        }
    }

    pub fn with_output_prefix(mut self, output_prefix: StyledObject<String>) -> Self {
        self.output_prefix = Some(self.color_config.apply(output_prefix));
        self
    }

    pub fn with_warn_prefix(mut self, warn_prefix: StyledObject<String>) -> Self {
        self.warn_prefix = Some(self.color_config.apply(warn_prefix));
        self
    }

    pub fn with_error_prefix(mut self, error_prefix: StyledObject<String>) -> Self {
        self.error_prefix = Some(self.color_config.apply(error_prefix));
        self
    }

    pub fn with_timestamps(mut self, include_timestamps: bool) -> Self {
        self.include_timestamps = include_timestamps;
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

    fn format_prefix(&self, prefix: &StyledObject<String>) -> String {
        if self.include_timestamps {
            let timestamp = chrono::Local::now().format("%H:%M:%S%.3f");
            let grey_timestamp = self
                .color_config
                .apply(crate::GREY.apply_to(format!("[{timestamp}]")));
            format!("{grey_timestamp} {prefix}")
        } else {
            prefix.to_string()
        }
    }

    fn write_line(&mut self, message: impl Display, command: Command) {
        let prefix = match command {
            Command::Output => &self.output_prefix,
            Command::Warn => &self.warn_prefix,
            Command::Error => &self.error_prefix,
        }
        .as_ref()
        .unwrap_or(&self.default_prefix);
        let formatted_prefix = self.format_prefix(prefix);
        let writer = match command {
            Command::Output => &mut self.out,
            Command::Warn | Command::Error => &mut self.err,
        };

        // There's no reason to propagate this error
        // because we don't want our entire program to crash
        // due to a log failure.
        if let Err(err) = writeln!(writer, "{formatted_prefix}{message}") {
            error!("cannot write to logs: {:?}", err);
        }
    }

    /// Construct a PrefixedWriter which will behave the same as `output`, but
    /// without the requirement that messages be valid UTF-8
    pub fn output_prefixed_writer(&mut self) -> PrefixedWriter<&mut W> {
        if self.include_timestamps {
            PrefixedWriter::new_with_timestamps(
                self.color_config,
                self.output_prefix
                    .clone()
                    .unwrap_or_else(|| Style::new().apply_to(String::new())),
                &mut self.out,
            )
        } else {
            PrefixedWriter::new(
                self.color_config,
                self.output_prefix
                    .clone()
                    .unwrap_or_else(|| Style::new().apply_to(String::new())),
                &mut self.out,
            )
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
    inner: LineWriter<PrefixedWriterInner<W>>,
}

impl<W: Write> PrefixedWriter<W> {
    pub fn new(color_config: ColorConfig, prefix: StyledObject<impl Display>, writer: W) -> Self {
        Self {
            inner: LineWriter::new(PrefixedWriterInner::new(
                color_config,
                prefix,
                writer,
                false,
            )),
        }
    }

    pub fn new_with_timestamps(
        color_config: ColorConfig,
        prefix: StyledObject<impl Display>,
        writer: W,
    ) -> Self {
        Self {
            inner: LineWriter::new(PrefixedWriterInner::new(color_config, prefix, writer, true)),
        }
    }
}

impl<W: Write> Write for PrefixedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Wraps a writer so that a prefix will be added at the start of each line.
/// Expects to only be called with complete lines.
struct PrefixedWriterInner<W> {
    prefix: String,
    writer: W,
    include_timestamps: bool,
    color_config: ColorConfig,
}

impl<W: Write> PrefixedWriterInner<W> {
    pub fn new(
        color_config: ColorConfig,
        prefix: StyledObject<impl Display>,
        writer: W,
        include_timestamps: bool,
    ) -> Self {
        let prefix = color_config.apply(prefix).to_string();
        Self {
            prefix,
            writer,
            include_timestamps,
            color_config,
        }
    }

    fn current_prefix(&self) -> String {
        if self.include_timestamps {
            let timestamp = chrono::Local::now().format("%H:%M:%S%.3f");
            let grey_timestamp = self
                .color_config
                .apply(crate::GREY.apply_to(format!("[{timestamp}]")));
            format!("{grey_timestamp} {}", self.prefix)
        } else {
            self.prefix.clone()
        }
    }
}

impl<W: Write> Write for PrefixedWriterInner<W> {
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
                let prefix = self.current_prefix();
                self.writer.write_all(prefix.as_bytes())?;
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

    fn prefixed_ui<W: Write>(out: W, err: W, color_config: ColorConfig) -> PrefixedUI<W> {
        let output_prefix = crate::BOLD.apply_to("output ".to_string());
        let warn_prefix = crate::MAGENTA.apply_to("warn ".to_string());
        PrefixedUI::new(color_config, out, err)
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

        let mut prefixed_ui = prefixed_ui(&mut out, &mut err, ColorConfig::new(strip_ansi));
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
        let mut writer = PrefixedWriterInner::new(
            ColorConfig::new(strip_ansi),
            crate::BOLD.apply_to("foo#build: "),
            &mut buffer,
            false,
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
        let mut writer = PrefixedWriterInner::new(
            ColorConfig::new(false),
            Style::new().apply_to("turbo > "),
            &mut buffer,
            false,
        );

        writer.write_all(input.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }

    #[test_case(&["foo"], "" ; "no newline")]
    #[test_case(&["\n"], "\n" ; "one newline")]
    #[test_case(&["foo\n"], "foo\n" ; "single newline")]
    #[test_case(&["foo ", "bar ", "baz\n"], "foo bar baz\n" ; "building line")]
    #[test_case(&["multiple\nlines\nin\none"], "multiple\nlines\nin\n" ; "multiple lines")]
    fn test_line_writer(inputs: &[&str], expected: &str) {
        let mut buffer = Vec::new();
        let mut writer = LineWriter::new(&mut buffer);
        for input in inputs {
            writer.write_all(input.as_bytes()).unwrap();
        }

        assert_eq!(String::from_utf8(buffer).unwrap(), expected);
    }

    #[test]
    fn test_prefixed_writer_split_lines() {
        let mut buffer = Vec::new();
        let mut writer = PrefixedWriter::new(
            ColorConfig::new(false),
            Style::new().apply_to("turbo > "),
            &mut buffer,
        );

        writer.write_all(b"not a line yet").unwrap();
        writer
            .write_all(b", now\nbut \ranother one starts")
            .unwrap();
        writer.write_all(b" done\n").unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(
            String::from_utf8(buffer).unwrap(),
            "turbo > not a line yet, now\nturbo > but \rturbo > another one starts done\nturbo > \
             \n"
        );
    }

    #[test]
    fn test_prefixed_writer_with_timestamps() {
        let mut buffer = Vec::new();
        let mut writer = PrefixedWriter::new_with_timestamps(
            ColorConfig::new(true),
            Style::new().apply_to("task: "),
            &mut buffer,
        );

        writer.write_all(b"hello world\n").unwrap();
        let output = String::from_utf8(buffer).unwrap();
        // Verify the timestamp format: [HH:MM:SS.mmm] task: message
        assert!(
            output.starts_with('['),
            "expected output to start with timestamp bracket, got: {output}"
        );
        assert!(
            output.contains("] task: hello world"),
            "expected output to contain timestamp and prefix, got: {output}"
        );
    }

    #[test]
    fn test_prefixed_writer_with_timestamps_colored() {
        let mut buffer = Vec::new();
        let mut writer = PrefixedWriter::new_with_timestamps(
            ColorConfig::new(false),
            Style::new().apply_to("task: "),
            &mut buffer,
        );

        writer.write_all(b"hello world\n").unwrap();
        let output = String::from_utf8(buffer).unwrap();
        // Verify the timestamp has gray ANSI codes (dim = \x1b[2m)
        assert!(
            output.contains("\x1b[2m"),
            "expected output to contain gray/dim ANSI code, got: {output}"
        );
        assert!(
            output.contains("task: hello world"),
            "expected output to contain prefix and message, got: {output}"
        );
    }

    #[test]
    fn test_prefixed_ui_with_timestamps() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut ui = PrefixedUI::new(ColorConfig::new(true), &mut out, &mut err)
            .with_output_prefix(crate::BOLD.apply_to("prefix ".to_string()))
            .with_timestamps(true);

        ui.output("test message");

        let output = String::from_utf8(out).unwrap();
        // Verify the timestamp format: [HH:MM:SS.mmm] prefix message
        assert!(
            output.starts_with('['),
            "expected output to start with timestamp bracket, got: {output}"
        );
        assert!(
            output.contains("] prefix test message"),
            "expected output to contain timestamp and prefix, got: {output}"
        );
    }

    #[test]
    fn test_prefixed_ui_with_timestamps_colored() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut ui = PrefixedUI::new(ColorConfig::new(false), &mut out, &mut err)
            .with_output_prefix(crate::BOLD.apply_to("prefix ".to_string()))
            .with_timestamps(true);

        ui.output("test message");

        let output = String::from_utf8(out).unwrap();
        // Verify the timestamp has gray ANSI codes (dim = \x1b[2m)
        assert!(
            output.contains("\x1b[2m"),
            "expected output to contain gray/dim ANSI code, got: {output}"
        );
        assert!(
            output.contains("prefix ") && output.contains("test message"),
            "expected output to contain prefix and message, got: {output}"
        );
    }
}
