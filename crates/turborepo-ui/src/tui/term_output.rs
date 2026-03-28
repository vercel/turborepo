use std::{io::Write, mem};

use turborepo_vt100 as vt100;

use super::{
    Error,
    event::{CacheResult, Direction, OutputLogs, TaskResult},
};

pub struct TerminalOutput<W> {
    output: Vec<u8>,
    pub parser: vt100::Parser,
    pub stdin: Option<W>,
    pub status: Option<String>,
    pub output_logs: Option<OutputLogs>,
    pub task_result: Option<TaskResult>,
    pub cache_result: Option<CacheResult>,
    pub scrollback_len: u64,
    /// Pending selection start position (row, col) - set on mouse down, used on
    /// first drag
    selection_start: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Copy)]
enum LogBehavior {
    Full,
    Status,
    Nothing,
}

impl<W> TerminalOutput<W> {
    pub fn new(rows: u16, cols: u16, stdin: Option<W>, scrollback_len: u64) -> Self {
        Self {
            output: Vec::new(),
            parser: vt100::Parser::new(rows, cols, scrollback_len as usize),
            stdin,
            status: None,
            output_logs: None,
            task_result: None,
            cache_result: None,
            scrollback_len,
            selection_start: None,
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
        let normalized = normalize_newlines(bytes);
        self.parser.process(&normalized);
        self.output.extend_from_slice(&normalized);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.parser.screen().size() != (rows, cols) {
            let scrollback = self.parser.screen().scrollback();
            let scrollback_len = self.scrollback_len as usize;
            let mut new_parser = vt100::Parser::new(rows, cols, scrollback_len);
            new_parser.process(&self.output);
            new_parser.screen_mut().set_scrollback(scrollback);
            // Completely swap out the old vterm with a new correctly sized one
            mem::swap(&mut self.parser, &mut new_parser);
        }
    }

    pub fn scroll(&mut self, direction: Direction) -> Result<(), Error> {
        self.scroll_by(direction, 1)
    }

    pub fn scroll_by(&mut self, direction: Direction, magnitude: usize) -> Result<(), Error> {
        let scrollback = self.parser.screen().scrollback();
        let new_scrollback = match direction {
            Direction::Up => scrollback.saturating_add(magnitude),
            Direction::Down => scrollback.saturating_sub(magnitude),
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
                stdout.write_all("┌─".as_bytes())?;
                stdout.write_all(title.as_bytes())?;
                stdout.write_all(b"\r\n")?;
                for row in screen.rows_formatted(0, cols) {
                    stdout.write_all(&row)?;
                    stdout.write_all(b"\r\n")?;
                }
                stdout.write_all("└─ ".as_bytes())?;
                stdout.write_all(task_name.as_bytes())?;
                stdout.write_all(" ──\r\n".as_bytes())?;
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
            .is_some_and(|s| !s.is_empty())
    }

    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Result<(), Error> {
        match event.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // Clear any existing selection and store the click position for potential drag
                self.parser.screen_mut().clear_selection();
                self.selection_start = Some((event.row, event.column));
            }
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                // On first drag, start selection from the initial click position
                if let Some((start_row, start_col)) = self.selection_start.take() {
                    self.parser
                        .screen_mut()
                        .update_selection(start_row, start_col);
                }
                // Update selection end point to current drag position
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

    pub fn clear_logs(&mut self) {
        self.output.clear();

        // clear screen and reset cursor
        self.process(b"\x1bc");
    }
}

/// Ensures every `\n` (LF) is preceded by `\r` (CR).
///
/// Child processes running in a PTY may disable the kernel's ONLCR flag
/// (e.g. Node.js calling `setRawMode(true)`), which means their output
/// contains bare `\n` without `\r`. The vt100 parser treats `\n` as a
/// line feed only (cursor moves down, column unchanged), so without `\r`
/// subsequent lines start at whatever column the cursor was at, producing
/// garbled overlapping text.
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let has_bare_lf =
        bytes.windows(2).any(|w| w[0] != b'\r' && w[1] == b'\n') || bytes.first() == Some(&b'\n');

    if !has_bare_lf {
        return bytes.to_vec();
    }

    let mut result = Vec::with_capacity(bytes.len() + bytes.len() / 10);
    for (i, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' && (i == 0 || bytes[i - 1] != b'\r') {
            result.push(b'\r');
        }
        result.push(byte);
    }
    result
}

#[cfg(test)]
mod newline_tests {
    use super::*;

    #[test]
    fn no_newlines_passthrough() {
        assert_eq!(normalize_newlines(b"hello"), b"hello");
    }

    #[test]
    fn crlf_unchanged() {
        assert_eq!(normalize_newlines(b"hello\r\nworld"), b"hello\r\nworld");
    }

    #[test]
    fn bare_lf_gets_cr() {
        assert_eq!(normalize_newlines(b"hello\nworld"), b"hello\r\nworld");
    }

    #[test]
    fn leading_lf_gets_cr() {
        assert_eq!(normalize_newlines(b"\nhello"), b"\r\nhello");
    }

    #[test]
    fn mixed_lf_and_crlf() {
        assert_eq!(
            normalize_newlines(b"a\r\nb\nc\r\nd\n"),
            b"a\r\nb\r\nc\r\nd\r\n"
        );
    }

    #[test]
    fn bare_lf_causes_garbled_output_without_normalize() {
        // Simulate the exact scenario: a child process writes two lines
        // with bare \n. Without normalization, the second line starts at
        // the column where the first line ended.
        let mut parser = turborepo_vt100::Parser::new(5, 20, 0);

        // Write "hello" then bare \n then "world"
        parser.process(b"hello\nworld");
        let screen = parser.screen();

        // Without CR, "world" starts at column 5 (where "hello" ended)
        let row0 = (0..20)
            .map(|c| screen.cell(0, c).unwrap().contents())
            .collect::<String>();
        let row1 = (0..20)
            .map(|c| screen.cell(1, c).unwrap().contents())
            .collect::<String>();

        assert_eq!(row0.trim(), "hello");
        // Without normalize, "world" starts at col 5
        assert_eq!(row1.trim(), "world");
        assert_eq!(screen.cell(1, 5).unwrap().contents(), "w");
    }

    #[test]
    fn normalize_fixes_garbled_output() {
        let mut parser = turborepo_vt100::Parser::new(5, 20, 0);

        let normalized = normalize_newlines(b"hello\nworld");
        parser.process(&normalized);
        let screen = parser.screen();

        let row0 = (0..20)
            .map(|c| screen.cell(0, c).unwrap().contents())
            .collect::<String>();
        let row1 = (0..20)
            .map(|c| screen.cell(1, c).unwrap().contents())
            .collect::<String>();

        assert_eq!(row0.trim(), "hello");
        assert_eq!(row1.trim(), "world");
        // After normalize, "world" starts at col 0
        assert_eq!(screen.cell(1, 0).unwrap().contents(), "w");
    }
}
