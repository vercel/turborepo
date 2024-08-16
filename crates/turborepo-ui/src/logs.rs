use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

use tracing::{debug, warn};
use turbopath::AbsoluteSystemPath;

use crate::Error;

/// Receives logs and multiplexes them to a log file and/or a prefixed
/// writer
pub struct LogWriter<W> {
    log_file: Option<BufWriter<File>>,
    writer: Option<W>,
}

/// Derive didn't work here.
/// (we don't actually need `W` to implement `Default` here)
impl<W> Default for LogWriter<W> {
    fn default() -> Self {
        Self {
            log_file: None,
            writer: None,
        }
    }
}

impl<W: Write> LogWriter<W> {
    pub fn with_log_file(&mut self, log_file_path: &AbsoluteSystemPath) -> Result<(), Error> {
        log_file_path.ensure_dir().map_err(|err| {
            warn!("error creating log file directory: {:?}", err);
            Error::CannotWriteLogs(err)
        })?;

        let log_file = log_file_path.create().map_err(|err| {
            warn!("error creating log file: {:?}", err);
            Error::CannotWriteLogs(err)
        })?;

        self.log_file = Some(BufWriter::new(log_file));

        Ok(())
    }

    pub fn with_writer(&mut self, writer: W) {
        self.writer = Some(writer);
    }
}

impl<W: Write> Write for LogWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match (&mut self.log_file, &mut self.writer) {
            (Some(log_file), Some(prefixed_writer)) => {
                let _ = prefixed_writer.write(buf)?;
                log_file.write(buf)
            }
            (Some(log_file), None) => log_file.write(buf),
            (None, Some(prefixed_writer)) => prefixed_writer.write(buf),
            (None, None) => {
                // Should this be an error or even a panic?
                debug!("no log file or prefixed writer");
                Ok(0)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(log_file) = &mut self.log_file {
            log_file.flush()?;
        }
        if let Some(prefixed_writer) = &mut self.writer {
            prefixed_writer.flush()?;
        }

        Ok(())
    }
}

pub fn replay_logs<W: Write>(
    mut output: W,
    log_file_name: &AbsoluteSystemPath,
) -> Result<(), Error> {
    debug!("start replaying logs");

    let log_file = File::open(log_file_name).map_err(|err| {
        warn!("error opening log file: {:?}", err);
        Error::CannotReadLogs(err)
    })?;

    let mut log_reader = BufReader::new(log_file);

    let mut buffer = Vec::new();
    loop {
        let num_bytes = log_reader
            .read_until(b'\n', &mut buffer)
            .map_err(Error::CannotReadLogs)?;
        if num_bytes == 0 {
            break;
        }

        // If the log file doesn't end with a newline, then we add one to ensure the
        // underlying writer receives a full line.
        if !buffer.ends_with(b"\n") {
            buffer.push(b'\n');
        }
        output.write_all(&buffer).map_err(Error::CannotReadLogs)?;

        buffer.clear();
    }

    debug!("finish replaying logs");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{
        logs::replay_logs, ColorConfig, LogWriter, PrefixedUI, PrefixedWriter, BOLD, CYAN,
    };

    #[test]
    fn test_log_writer() -> Result<()> {
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        let mut prefixed_writer_output = Vec::new();
        let mut log_writer = LogWriter::default();
        let color_config = ColorConfig::new(false);

        log_writer.with_log_file(&log_file_path)?;
        log_writer.with_writer(PrefixedWriter::new(
            color_config,
            CYAN.apply_to(">".to_string()),
            &mut prefixed_writer_output,
        ));

        writeln!(log_writer, "one fish")?;
        writeln!(log_writer, "two fish")?;
        writeln!(log_writer, "red fish")?;
        writeln!(log_writer, "blue fish")?;

        log_writer.flush()?;

        assert_eq!(
            String::from_utf8(prefixed_writer_output)?,
            "\u{1b}[36m>\u{1b}[0mone fish\n\u{1b}[36m>\u{1b}[0mtwo fish\n\u{1b}[36m>\u{1b}[0mred \
             fish\n\u{1b}[36m>\u{1b}[0mblue fish\n"
        );

        let log_file_contents = log_file_path.read_to_string()?;

        assert_eq!(
            log_file_contents,
            "one fish\ntwo fish\nred fish\nblue fish\n"
        );

        Ok(())
    }

    #[test]
    fn test_replay_logs() -> Result<()> {
        let color_config = ColorConfig::new(false);
        let mut output = Vec::new();
        let mut err = Vec::new();
        let mut prefixed_ui = PrefixedUI::new(color_config, &mut output, &mut err)
            .with_output_prefix(CYAN.apply_to(">".to_string()))
            .with_warn_prefix(BOLD.apply_to(">!".to_string()));
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        fs::write(&log_file_path, "\none fish\ntwo fish\nred fish\nblue fish")?;
        replay_logs(prefixed_ui.output_prefixed_writer(), &log_file_path)?;

        assert_eq!(
            String::from_utf8(output)?,
            "\u{1b}[36m>\u{1b}[0m\n\u{1b}[36m>\u{1b}[0mone fish\n\u{1b}[36m>\u{1b}[0mtwo \
             fish\n\u{1b}[36m>\u{1b}[0mred fish\n\u{1b}[36m>\u{1b}[0mblue fish\n"
        );

        Ok(())
    }

    #[test]
    fn test_replay_logs_invalid_utf8() -> Result<()> {
        let color_config = ColorConfig::new(true);
        let mut output = Vec::new();
        let mut err = Vec::new();
        let mut prefixed_ui = PrefixedUI::new(color_config, &mut output, &mut err)
            .with_output_prefix(CYAN.apply_to(">".to_string()))
            .with_warn_prefix(BOLD.apply_to(">!".to_string()));
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        fs::write(&log_file_path, [0, 159, 146, 150, b'\n'])?;
        replay_logs(prefixed_ui.output_prefixed_writer(), &log_file_path)?;

        assert_eq!(output, [b'>', 0, 159, 146, 150, b'\n']);
        Ok(())
    }
}
