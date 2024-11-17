use std::{io::Write, mem};

use turborepo_vt100 as vt100;

use super::{
    event::{CacheResult, Direction, OutputLogs, TaskResult},
    Error,
};

const SCROLLBACK_LEN: usize = 1024;

pub struct TerminalOutput<W> {
    output: Vec<u8>,
    pub parser: vt100::Parser,
    pub stdin: Option<W>,
    pub status: Option<String>,
    pub output_logs: Option<OutputLogs>,
    pub task_result: Option<TaskResult>,
    pub cache_result: Option<CacheResult>,
}

#[derive(Debug, Clone, Copy)]
enum LogBehavior {
    Full,
    Status,
    Nothing,
}

impl<W> TerminalOutput<W> {
    pub fn new(rows: u16, cols: u16, stdin: Option<W>) -> Self {
        Self {
            output: Vec::new(),
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LEN),
            stdin,
            status: None,
            output_logs: None,
            task_result: None,
            cache_result: None,
        }
    }

    pub fn title(&self, task_name: &str) -> String {
        match self.status.as_deref() {
            Some(status) => format!(" {task_name} > {status} "),
            None => format!(" {task_name} > "),
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.output.extend_from_slice(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.parser.screen().size() != (rows, cols) {
            let scrollback = self.parser.screen().scrollback();
            let mut new_parser = vt100::Parser::new(rows, cols, SCROLLBACK_LEN);
            new_parser.process(&self.output);
            new_parser.screen_mut().set_scrollback(scrollback);
            // Completely swap out the old vterm with a new correctly sized one
            mem::swap(&mut self.parser, &mut new_parser);
        }
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

    fn persist_behavior(&self) -> LogBehavior {
        match self.output_logs.unwrap_or(OutputLogs::Full) {
            OutputLogs::Full => LogBehavior::Full,
            OutputLogs::None => LogBehavior::Nothing,
            OutputLogs::HashOnly => LogBehavior::Status,
            OutputLogs::NewOnly => {
                if matches!(self.cache_result, Some(super::event::CacheResult::Miss),) {
                    LogBehavior::Full
                } else {
                    LogBehavior::Status
                }
            }
            OutputLogs::ErrorsOnly => {
                if matches!(self.task_result, Some(TaskResult::Failure)) {
                    LogBehavior::Full
                } else {
                    LogBehavior::Nothing
                }
            }
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn persist_screen(&self, task_name: &str) -> std::io::Result<()> {
        let mut stdout = std::io::stdout().lock();
        let title = self.title(task_name);
        match self.persist_behavior() {
            LogBehavior::Full => {
                let screen = self.parser.entire_screen();
                let (_, cols) = screen.size();
                stdout.write_all("┌".as_bytes())?;
                stdout.write_all(title.as_bytes())?;
                stdout.write_all(b"\r\n")?;
                for row in screen.rows_formatted(0, cols) {
                    stdout.write_all("│ ".as_bytes())?;
                    stdout.write_all(&row)?;
                    stdout.write_all(b"\r\n")?;
                }
                stdout.write_all("└────>\r\n".as_bytes())?;
            }
            LogBehavior::Status => {
                stdout.write_all(title.as_bytes())?;
                stdout.write_all(b"\r\n")?;
            }
            LogBehavior::Nothing => (),
        }
        Ok(())
    }

    pub fn has_selection(&self) -> bool {
        self.parser
            .screen()
            .selected_text()
            .map_or(false, |s| !s.is_empty())
    }

    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Result<(), Error> {
        match event.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // We need to update the vterm so we don't continue to render the selection
                self.parser.screen_mut().clear_selection();
            }
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                // Update selection of underlying parser
                self.parser
                    .screen_mut()
                    .update_selection(event.row, event.column);
            }
            // Scrolling is handled elsewhere
            crossterm::event::MouseEventKind::ScrollDown => (),
            crossterm::event::MouseEventKind::ScrollUp => (),
            // I think we can ignore this?
            crossterm::event::MouseEventKind::Moved => (),
            // Don't care about other mouse buttons
            crossterm::event::MouseEventKind::Down(_) => (),
            crossterm::event::MouseEventKind::Drag(_) => (),
            // We don't support horizontal scroll
            crossterm::event::MouseEventKind::ScrollLeft
            | crossterm::event::MouseEventKind::ScrollRight => (),
            // Cool, person stopped holding down mouse
            crossterm::event::MouseEventKind::Up(_) => (),
        }
        Ok(())
    }

    pub fn copy_selection(&self) -> Option<String> {
        self.parser.screen().selected_text()
    }
}
