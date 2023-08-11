use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
};

use tracing::{debug, warn};
use turbopath::AbsoluteSystemPath;

use crate::{prefixed::PrefixedUI, Error};

<<<<<<< HEAD
/// Writes to `output` with a prefix. The prefix is styled with `ui`.
#[allow(dead_code)]
pub struct PrefixedUI<D, W> {
    ui: UI,
    prefix: StyledObject<D>,
    output: W,
}

#[allow(dead_code)]
impl<D: Display + Clone, W: Write> PrefixedUI<D, W> {
    pub fn new(ui: UI, prefix: StyledObject<D>, output: W) -> Self {
        Self { ui, prefix, output }
    }

    /// Write `message` to `output` with the prefix. Note that this
    /// does output the prefix when message is empty, unlike the Go
    /// implementation. We do this because this behavior is what we actually
    /// want for replaying logs.
    pub fn output(&mut self, message: impl Display) -> Result<(), Error> {
        writeln!(
            self.output,
            "{}{}",
            self.ui.apply(self.prefix.clone()),
            message
        )
        .map_err(Error::CannotWriteLogs)?;

        Ok(())
    }
}

pub fn replay_logs<W: Write>(
    output: &mut PrefixedUI<W>,
    log_file_name: &AbsoluteSystemPath,
) -> Result<(), Error> {
    debug!("start replaying logs");

    let log_file = File::open(log_file_name).map_err(|err| {
        warn!("error opening log file: {:?}", err);
        Error::CannotReadLogs(err)
    })?;

    let log_reader = BufReader::new(log_file);

    for line in log_reader.lines() {
        let line = line.map_err(Error::CannotReadLogs)?;
        output.output(line);
    }

    debug!("finish replaying logs");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{
        log_replayer::{replay_logs, PrefixedUI},
        CYAN, UI,
    };

    #[test]
    fn test_replay_logs() -> Result<()> {
        let ui = UI::new(false);
        let mut output = Vec::new();
        let mut prefixed_ui = PrefixedUI::new(ui, CYAN.apply_to(">"), &mut output);
        let dir = tempdir()?;
        let log_file_path = AbsoluteSystemPathBuf::try_from(dir.path().join("test.txt"))?;
        fs::write(&log_file_path, "\none fish\ntwo fish\nred fish\nblue fish")?;
        replay_logs(&mut prefixed_ui, &log_file_path)?;

        assert_eq!(
            String::from_utf8(output)?,
            "\u{1b}[36m>\u{1b}[0m\n\u{1b}[36m>\u{1b}[0mone fish\n\u{1b}[36m>\u{1b}[0mtwo \
             fish\n\u{1b}[36m>\u{1b}[0mred fish\n\u{1b}[36m>\u{1b}[0mblue fish\n"
        );

        Ok(())
    }
}
