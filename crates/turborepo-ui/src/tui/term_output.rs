use std::io::Write;

use turborepo_ghostty as ghostty;

use super::{
    Error,
    event::{CacheResult, Direction, OutputLogs, TaskResult},
};

pub struct TerminalOutput<W> {
    output: Vec<u8>,
    pub parser: ghostty::Parser,
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
            parser: ghostty::Parser::new(rows, cols, scrollback_len as usize),
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
        self.parser.size().unwrap_or((0, 0))
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let normalized = normalize_newlines(bytes);
        self.parser.process(&normalized);
        self.output.extend_from_slice(&normalized);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if self.size() != (rows, cols) {
            let _ = self.parser.resize(rows, cols);
        }
    }

    pub fn scroll(&mut self, direction: Direction) -> Result<(), Error> {
        self.scroll_by(direction, 1)
    }

    pub fn scroll_by(&mut self, direction: Direction, magnitude: usize) -> Result<(), Error> {
        let up = matches!(direction, Direction::Up);
        self.parser.scroll_by(up, magnitude)?;
        Ok(())
    }

    pub fn scroll_to_top(&mut self) -> Result<(), Error> {
        self.parser.scroll_to_top()?;
        Ok(())
    }

    pub fn scroll_to_bottom(&mut self) -> Result<(), Error> {
        self.parser.scroll_to_bottom()?;
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
                let screen = self
                    .parser
                    .format_screen_vt()
                    .map_err(std::io::Error::other)?;
                stdout.write_all("┌─".as_bytes())?;
                stdout.write_all(title.as_bytes())?;
                stdout.write_all(b"\r\n")?;
                stdout.write_all(&screen)?;
                if !screen.ends_with(b"\n") && !screen.ends_with(b"\r\n") {
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
        self.parser.has_selection()
    }

    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Result<(), Error> {
        match event.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                self.parser.clear_selection()?;
                self.selection_start = Some((event.row, event.column));
            }
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                if let Some((start_row, start_col)) = self.selection_start {
                    self.parser.update_selection(
                        start_row,
                        start_col,
                        event.row,
                        event.column,
                    )?;
                }
            }
            crossterm::event::MouseEventKind::ScrollDown => (),
            crossterm::event::MouseEventKind::ScrollUp => (),
            crossterm::event::MouseEventKind::Moved => (),
            crossterm::event::MouseEventKind::Down(_) => (),
            crossterm::event::MouseEventKind::Drag(_) => (),
            crossterm::event::MouseEventKind::ScrollLeft
            | crossterm::event::MouseEventKind::ScrollRight => (),
            crossterm::event::MouseEventKind::Up(_) => {
                self.selection_start = None;
            }
        }
        Ok(())
    }

    pub fn copy_selection(&mut self) -> Option<String> {
        self.parser.selected_text().ok().flatten()
    }

    pub fn clear_logs(&mut self) {
        self.output.clear();
        self.parser.reset();
    }
}

/// Ensures every `\n` (LF) is preceded by `\r` (CR).
///
/// Child processes running in a PTY may disable the kernel's ONLCR flag
/// (e.g. Node.js calling `setRawMode(true)`), which means their output
/// contains bare `\n` without `\r`. Without `\r`, subsequent lines start at
/// whatever column the cursor was at, producing garbled overlapping text.
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
    fn mouse_drag_selection_can_be_copied() -> Result<(), Error> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut output: TerminalOutput<()> = TerminalOutput::new(10, 40, None, 100);
        output.process(b"hello world\r\n");

        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;

        assert!(output.has_selection());
        assert_eq!(output.copy_selection().as_deref(), Some("hello"));
        Ok(())
    }
}
