use std::io::Write;

use lazy_static::lazy_static;
use termcolor::{ColorChoice, ColorSpec, StandardStream, WriteColor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UI {
    choice: ColorChoice,
}

impl UI {
    pub fn new(choice: ColorChoice) -> Self {
        Self { choice }
    }

    fn stdout(&self) -> StandardStream {
        StandardStream::stdout(self.choice)
    }

    pub fn info<S>(&self, message: S, color: Option<&ColorSpec>) -> Result<(), std::io::Error>
    where
        S: AsRef<str>,
    {
        let mut stdout = self.stdout();
        if let Some(spec) = color {
            stdout.set_color(spec)?;
        }
        writeln!(&mut stdout, "{}", message.as_ref())?;
        if color.is_some() {
            stdout.reset()?;
        }
        Ok(())
    }
}

lazy_static! {
    pub static ref GREY: ColorSpec = {
        let mut spec = ColorSpec::new();
        spec.set_dimmed(true);
        spec
    };
}
