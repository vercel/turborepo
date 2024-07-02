use std::{collections::BTreeMap, io::Write};

use ratatui::{
    style::Style,
    text::Line,
    widgets::{Block, Borders, Widget},
};
use tracing::debug;
use tui_term::widget::PseudoTerminal;

use super::{app::Direction, Error, TerminalOutput};

const FOOTER_TEXT_ACTIVE: &str = "Press`Ctrl-Z` to stop interacting.";
const FOOTER_TEXT_INACTIVE: &str = "Press `Enter` to interact.";

pub struct TerminalPane<'a, W> {
    logs_output: &'a TerminalOutput<W>,
    displayed_task: Option<String>,
    rows: u16,
    cols: u16,
    highlight: bool,
}

impl<'a, W> TerminalPane<'a, W> {
    pub fn new(rows: u16, cols: u16, highlight: bool, logs_output: &'a TerminalOutput<W>) -> Self {
        // We trim 2 from rows and cols as we use them for borders
        let rows = rows.saturating_sub(2);
        let cols = cols.saturating_sub(2);
        Self {
            displayed_task: None,
            rows,
            cols,
            highlight,
            logs_output,
        }
    }

    pub fn highlight(&mut self, highlight: bool) {
        self.highlight = highlight;
    }

    pub fn process_output(&mut self, task: &str, output: &[u8]) -> Result<(), Error> {
        let task = self
            .task_mut(task)
            .inspect_err(|_| debug!("cannot find task on process output"))?;
        task.parser.process(output);
        Ok(())
    }

    pub fn has_stdin(&self, task: &str) -> Option<W> {
        &self.logs_output.stdin
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), Error> {
        let changed = self.rows != rows || self.cols != cols;
        self.rows = rows;
        self.cols = cols;
        if changed {
            // Eagerly resize currently displayed terminal
            if let Some(task_name) = self.displayed_task.as_deref() {
                let task = self
                    .tasks
                    .get_mut(task_name)
                    .expect("displayed should always point to valid task");
                task.resize(rows, cols);
            }
        }

        Ok(())
    }

    pub fn select(&mut self, task: &str) -> Result<(), Error> {
        let rows = self.rows;
        let cols = self.cols;
        {
            let terminal = self.task_mut(task)?;
            terminal.resize(rows, cols);
        }
        self.displayed_task = Some(task.into());

        Ok(())
    }

    pub fn set_status(&mut self, task: &str, status: String) -> Result<(), Error> {
        let task = self.task_mut(task)?;
        task.status = Some(status);
        Ok(())
    }

    pub fn scroll(&mut self, task: &str, direction: Direction) -> Result<(), Error> {
        let task = self.task_mut(task)?;
        let scrollback = task.parser.screen().scrollback();
        let new_scrollback = match direction {
            Direction::Up => scrollback + 1,
            Direction::Down => scrollback.saturating_sub(1),
        };
        task.parser.screen_mut().set_scrollback(new_scrollback);
        Ok(())
    }

    /// Persist all task output to the terminal
    pub fn persist_tasks(&mut self, started_tasks: &[&str]) -> std::io::Result<()> {
        for (task_name, task) in started_tasks
            .iter()
            .copied()
            .filter_map(|started_task| (Some(started_task)).zip(self.tasks.get(started_task)))
        {
            task.persist_screen(task_name)?;
        }
        Ok(())
    }

    pub fn term_size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    fn selected(&self) -> Option<(&String, &TerminalOutput<W>)> {
        let task_name = self.displayed_task.as_deref()?;
        self.tasks.get_key_value(task_name)
    }

    fn task_mut(&mut self, task: &str) -> Result<&mut TerminalOutput<W>, Error> {
        self.tasks.get_mut(task).ok_or_else(|| Error::TaskNotFound {
            name: task.to_string(),
        })
    }
}

impl<W: Write> TerminalPane<W> {
    /// Insert a stdin to be associated with a task
    pub fn insert_stdin(&mut self, task_name: &str, stdin: Option<W>) -> Result<(), Error> {
        let task = self.task_mut(task_name)?;
        task.stdin = stdin;
        Ok(())
    }

    pub fn process_input(&mut self, task: &str, input: &[u8]) -> Result<(), Error> {
        let task_output = self.task_mut(task)?;
        if let Some(stdin) = &mut task_output.stdin {
            stdin.write_all(input).map_err(|e| Error::Stdin {
                name: task.into(),
                e,
            })?;
        }
        Ok(())
    }
}

impl<W> Widget for &TerminalPane<W> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let Some((task_name, task)) = self.selected() else {
            return;
        };
        let screen = task.parser.screen();
        let mut block = Block::default()
            .borders(Borders::LEFT)
            .title(task.title(task_name));
        if self.highlight {
            block = block.title_bottom(Line::from(FOOTER_TEXT_ACTIVE).centered());
            block = block.border_style(Style::new().fg(ratatui::style::Color::Yellow));
        } else {
            block = block.title_bottom(Line::from(FOOTER_TEXT_INACTIVE).centered());
        }
        let term = PseudoTerminal::new(screen).block(block);
        term.render(area, buf)
    }
}

#[cfg(test)]
mod test {
    // Used by assert_buffer_eq
    #[allow(unused_imports)]
    use indoc::indoc;
    use ratatui::{assert_buffer_eq, buffer::Buffer, layout::Rect};

    use super::*;

    #[test]
    fn test_basic() {
        let mut pane: TerminalPane<()> = TerminalPane::new(6, 8, vec!["foo".into()], false);
        pane.select("foo").unwrap();
        pane.process_output("foo", b"1\r\n2\r\n3\r\n4\r\n5\r\n")
            .unwrap();

        let area = Rect::new(0, 0, 8, 6);
        let mut buffer = Buffer::empty(area);
        pane.render(area, &mut buffer);
        // Reset style change of the cursor
        buffer.set_style(Rect::new(1, 4, 1, 1), Style::reset());
        assert_buffer_eq!(
            buffer,
            Buffer::with_lines(vec![
                "│ foo > ",
                "│3      ",
                "│4      ",
                "│5      ",
                "│█      ",
                "│Press `",
            ])
        );
    }
}
