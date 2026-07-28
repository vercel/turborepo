use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

use tracing::debug;
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
            turborepo_log::warn(
                turborepo_log::Source::turbo(turborepo_log::Subsystem::Logs),
                format!("error creating log file directory: {err:?}"),
            )
            .emit();
            Error::CannotWriteLogs(err)
        })?;

        let log_file = log_file_path.create().map_err(|err| {
            turborepo_log::warn(
                turborepo_log::Source::turbo(turborepo_log::Subsystem::Logs),
                format!("error creating log file: {err:?}"),
            )
            .emit();
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
                debug!(
                    "No log file or prefixed writer to write to. This should only happen when \
                     both caching is disabled and output logs are set to none."
                );

                // Returning the buffer's length so callers don't think this is a failure to
                // create the buffer
                Ok(buf.len())
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
        turborepo_log::warn(
            turborepo_log::Source::turbo(turborepo_log::Subsystem::Logs),
            format!("error opening log file: {err:?}"),
        )
        .emit();
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

    use crate::{LogWriter, logs::replay_logs};

    #[test]
    fn test_log_writer() -> Result<()> {
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        let mut display_output = Vec::new();
        let mut log_writer = LogWriter::default();

        log_writer.with_log_file(&log_file_path)?;
        log_writer.with_writer(&mut display_output);

        writeln!(log_writer, "one fish")?;
        writeln!(log_writer, "two fish")?;

        log_writer.flush()?;

        assert_eq!(String::from_utf8(display_output)?, "one fish\ntwo fish\n");

        let log_file_contents = log_file_path.read_to_string()?;
        assert_eq!(log_file_contents, "one fish\ntwo fish\n");

        Ok(())
    }

    #[test]
    fn test_replay_logs() -> Result<()> {
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        fs::write(&log_file_path, "one fish\ntwo fish\n")?;
        let mut output = Vec::new();
        replay_logs(&mut output, &log_file_path)?;
        assert_eq!(String::from_utf8(output)?, "one fish\ntwo fish\n");
        Ok(())
    }

    #[test]
    fn test_replay_logs_invalid_utf8() -> Result<()> {
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        fs::write(&log_file_path, [0, 159, 146, 150, b'\n'])?;
        let mut output = Vec::new();
        replay_logs(&mut output, &log_file_path)?;
        assert_eq!(output, [0, 159, 146, 150, b'\n']);
        Ok(())
    }
}
