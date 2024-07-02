use std::io::Write;

use turborepo_vt100 as vt100;

use super::{app::Direction, Error};

const FOOTER_TEXT_ACTIVE: &str = "Press`Ctrl-Z` to stop interacting.";
const FOOTER_TEXT_INACTIVE: &str = "Press `Enter` to interact.";

pub struct TerminalOutput<W> {
    rows: u16,
    cols: u16,
    pub parser: vt100::Parser,
    pub stdin: Option<W>,
    pub status: Option<String>,
}

impl<W> TerminalOutput<W> {
    pub fn new(rows: u16, cols: u16, stdin: Option<W>) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 1024),
            stdin,
            rows,
            cols,
            status: None,
        }
    }

    pub fn title(&self, task_name: &str) -> String {
        match self.status.as_deref() {
            Some(status) => format!(" {task_name} > {status} "),
            None => format!(" {task_name} > "),
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.rows != rows || self.cols != cols {
            self.parser.screen_mut().set_size(rows, cols);
        }
        self.rows = rows;
        self.cols = cols;
    }

    pub fn scroll(&mut self, direction: Direction) -> Result<(), Error> {
        let scrollback = self.parser.screen().scrollback();
        let new_scrollback = match direction {
            Direction::Up => scrollback + 1,
            Direction::Down => scrollback.saturating_sub(1),
        };
        self.parser.screen_mut().set_scrollback(new_scrollback);
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn persist_screen(&self, task_name: &str) -> std::io::Result<()> {
        let screen = self.parser.entire_screen();
        let title = self.title(task_name);
        let mut stdout = std::io::stdout().lock();
        stdout.write_all("┌".as_bytes())?;
        stdout.write_all(title.as_bytes())?;
        stdout.write_all(b"\r\n")?;
        for row in screen.rows_formatted(0, self.cols) {
            stdout.write_all("│ ".as_bytes())?;
            stdout.write_all(&row)?;
            stdout.write_all(b"\r\n")?;
        }
        stdout.write_all("└────>\r\n".as_bytes())?;

        Ok(())
    }
}
