use std::{collections::BTreeMap, io::Write};

use ratatui::widgets::{Block, Borders, Widget};
use tui_term::{vt100, widget::PseudoTerminal};

use super::Error;

pub struct TerminalPane<W> {
    tasks: BTreeMap<String, TerminalOutput<W>>,
    displayed: Option<String>,
    rows: u16,
    cols: u16,
}

struct TerminalOutput<W> {
    rows: u16,
    cols: u16,
    parser: vt100::Parser,
    stdin: Option<W>,
}
impl<W> TerminalPane<W> {
    pub fn new(rows: u16, cols: u16, tasks: impl IntoIterator<Item = (String, Option<W>)>) -> Self {
        // We trim 2 from rows and cols as we use them for borders
        let rows = rows.saturating_sub(2);
        let cols = cols.saturating_sub(2);
        Self {
            tasks: tasks
                .into_iter()
                .map(|(name, stdin)| (name, TerminalOutput::new(rows, cols, stdin)))
                .collect(),
            displayed: None,
            rows,
            cols,
        }
    }

    pub fn process_output(&mut self, task: &str, output: &[u8]) -> Result<(), Error> {
        let task = self.task_mut(task)?;
        task.parser.process(output);
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), Error> {
        let changed = self.rows != rows || self.cols != cols;
        self.rows = rows;
        self.cols = cols;
        if changed {
            // Eagerly resize currently displayed terminal
            if let Some(task_name) = self.displayed.as_deref() {
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
        self.displayed = Some(task.into());

        Ok(())
    }

    fn selected(&self) -> Option<(&String, &TerminalOutput<W>)> {
        let task_name = self.displayed.as_deref()?;
        self.tasks.get_key_value(task_name)
    }

    fn task_mut(&mut self, task: &str) -> Result<&mut TerminalOutput<W>, Error> {
        self.tasks.get_mut(task).ok_or_else(|| Error::TaskNotFound {
            name: task.to_string(),
        })
    }
}

impl<W: Write> TerminalPane<W> {
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

impl<W> TerminalOutput<W> {
    fn new(rows: u16, cols: u16, stdin: Option<W>) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 1024),
            stdin,
            rows,
            cols,
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if self.rows != rows || self.cols != cols {
            self.parser.set_size(rows, cols);
        }
        self.rows = rows;
        self.cols = cols;
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
        let term = PseudoTerminal::new(screen).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {task_name} >")),
        );
        term.render(area, buf)
    }
}

#[cfg(test)]
mod test {
    // Used by assert_buffer_eq
    #[allow(unused_imports)]
    use indoc::indoc;
    use ratatui::{assert_buffer_eq, buffer::Buffer, layout::Rect, style::Style};

    use super::*;

    #[test]
    fn test_basic() {
        let mut pane: TerminalPane<()> = TerminalPane::new(6, 8, vec![("foo".into(), None)]);
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
                "┌ foo >┐",
                "│3     │",
                "│4     │",
                "│5     │",
                "│█     │",
                "└──────┘",
            ])
        );
    }
}
