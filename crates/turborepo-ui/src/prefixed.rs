use std::{fmt::Display, io::Write};

use console::StyledObject;

use crate::{Error, UI};

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

    pub fn output(&mut self, message: impl Display) -> Result<(), Error> {
        writeln!(
            self.output,
            "{}{}",
            self.ui.apply(self.output_prefix.clone()),
            message
        )
        .map_err(Error::CannotWriteLogs)?;

        Ok(())
    }

    pub fn warn(&mut self, message: impl Display) -> Result<(), Error> {
        writeln!(
            self.output,
            "{}{}",
            self.ui.apply(self.warn_prefix.clone()),
            message
        )
        .map_err(Error::CannotWriteLogs)?;

        Ok(())
    }
}
