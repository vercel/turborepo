use std::io::{Seek, SeekFrom, Write};

use turborepo_ghostty as ghostty;

use super::{
    Error,
    event::{CacheResult, Direction, OutputLogs, TaskResult},
};

/// Per-task raw output is kept in memory up to this size, then rolls over to
/// an anonymous temporary file. The terminal parser already bounds visible
/// scrollback; this bounds the retained raw stream independently so a verbose
/// task cannot grow memory without limit.
const OUTPUT_SPOOL_THRESHOLD: usize = 1024 * 1024;

pub struct TerminalOutput<W> {
    /// The complete raw (newline-normalized) byte stream, spooled through a
    /// bounded in-memory buffer that rolls over to disk.
    output: tempfile::SpooledTempFile,
    /// Total bytes written to `output` (its write position is disturbed by
    /// replay reads, so length is tracked explicitly).
    output_len: usize,
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
    pub fn new(rows: u16, cols: u16, stdin: Option<W>, scrollback_len: u64) -> Result<Self, Error> {
        Ok(Self {
            output: tempfile::spooled_tempfile(OUTPUT_SPOOL_THRESHOLD),
            output_len: 0,
            parser: ghostty::Parser::try_new(rows, cols, scrollback_len as usize)?,
            stdin,
            status: None,
            output_logs: None,
            task_result: None,
            cache_result: None,
            scrollback_len,
            selection_start: None,
        })
    }

    /// Total number of raw output bytes produced so far.
    pub fn output_len(&self) -> usize {
        self.output_len
    }

    /// Reads the raw (newline-normalized) output bytes from `offset` to the
    /// end. Used to backfill streamed logs when the user switches from the
    /// TUI to streaming mid-run. Reads come from the in-memory buffer or the
    /// backing temp file, so replay never requires retaining the whole
    /// stream in memory.
    pub fn read_output_from(&mut self, offset: usize) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.output_len.saturating_sub(offset));
        let result = self
            .output
            .seek(SeekFrom::Start(offset as u64))
            .and_then(|_| {
                // Restore the write position so later writes keep appending.
                let result = std::io::Read::read_to_end(&mut self.output, &mut buf);
                let _ = self.output.seek(SeekFrom::Start(self.output_len as u64));
                result
            });
        if let Err(err) = result {
            // Replay is best-effort; a failed spool read must not break the
            // TUI.
            tracing::debug!("failed to read spooled task output: {err}");
        }
        buf
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
        if let Err(err) = self.output.write_all(&normalized) {
            // Spool writes are in-memory or to an anonymous temp file; a
            // failure means replay data is lost, never that the TUI breaks.
            tracing::debug!("failed to spool task output: {err}");
        }
        self.output_len += normalized.len();
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

    /// Whether a mouse-driven selection is in progress (the button is still
    /// held down).
    pub fn is_selecting(&self) -> bool {
        self.selection_start.is_some()
    }

    /// Clears the selection highlight and any pending selection anchor.
    pub fn clear_selection(&mut self) -> Result<(), Error> {
        self.parser.clear_selection()?;
        self.selection_start = None;
        Ok(())
    }

    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Result<(), Error> {
        self.handle_mouse_with_scroll(event, None)
    }

    pub fn handle_mouse_with_scroll(
        &mut self,
        event: crossterm::event::MouseEvent,
        selection_scroll: Option<Direction>,
    ) -> Result<(), Error> {
        match event.kind {
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // Shift held at click time means the user is preventing the
                // copy before it starts, so don't anchor a selection. Most
                // terminals never deliver shifted mouse events (shift
                // bypasses mouse capture for native selection), but honor
                // them when they do arrive.
                let selection_start = (!event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT))
                .then_some((event.row, event.column));
                if let Some((row, column)) = selection_start {
                    self.parser.begin_selection(row, column)?;
                } else {
                    self.parser.clear_selection()?;
                }
                self.selection_start = selection_start;
            }
            crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                if self.selection_start.is_some() {
                    if let Some(direction) = selection_scroll {
                        self.scroll(direction)?;
                    }
                    self.parser.update_selection_end(event.row, event.column)?;
                }
            }
            crossterm::event::MouseEventKind::ScrollDown => (),
            crossterm::event::MouseEventKind::ScrollUp => (),
            // Hover means the button is up; drop any stale drag anchor from
            // a release that the terminal never delivered to us.
            crossterm::event::MouseEventKind::Moved => {
                self.cancel_selection_drag();
            }
            crossterm::event::MouseEventKind::Down(_) => (),
            crossterm::event::MouseEventKind::Drag(_) => (),
            crossterm::event::MouseEventKind::ScrollLeft
            | crossterm::event::MouseEventKind::ScrollRight => (),
            crossterm::event::MouseEventKind::Up(_) => {
                self.cancel_selection_drag();
            }
        }
        Ok(())
    }

    pub fn continue_selection_drag(
        &mut self,
        direction: Direction,
        row: u16,
        column: u16,
    ) -> Result<(), Error> {
        if self.selection_start.is_none() {
            return Ok(());
        }
        self.scroll(direction)?;
        self.parser.update_selection_end(row, column)?;
        Ok(())
    }

    pub fn cancel_selection_drag(&mut self) {
        self.selection_start = None;
        self.parser.cancel_incomplete_selection();
    }

    #[cfg(test)]
    pub(crate) fn has_pending_selection_anchor(&self) -> bool {
        self.selection_start.is_some()
    }

    pub fn copy_selection(&mut self) -> Option<String> {
        self.parser.selected_text().ok().flatten()
    }

    pub fn clear_logs(&mut self) {
        // A fresh spool releases any rolled-over temp file.
        self.output = tempfile::spooled_tempfile(OUTPUT_SPOOL_THRESHOLD);
        self.output_len = 0;
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
mod spool_tests {
    use super::*;

    /// Output far beyond the spool threshold must roll to disk and stay
    /// byte-complete on replay, including partial replays from a watermark.
    #[test]
    fn rolled_output_replays_completely() -> Result<(), Error> {
        let mut output = TerminalOutput::<std::io::Empty>::new(24, 80, None, 1000)?;

        let mut expected = Vec::new();
        // 2 MiB of line output, past the 1 MiB spool threshold.
        for i in 0..32768u32 {
            expected.extend_from_slice(format!("line number {i} of the build log\r\n").as_bytes());
        }
        output.process(&expected);

        assert!(
            output.output.is_rolled(),
            "output beyond the threshold must be spooled to disk"
        );
        assert_eq!(output.output_len(), expected.len());
        assert_eq!(output.read_output_from(0), expected);

        let watermark = 500_000;
        assert_eq!(output.read_output_from(watermark), expected[watermark..]);

        Ok(())
    }

    /// Output under the threshold stays in memory (no temp file).
    #[test]
    fn small_output_stays_in_memory() -> Result<(), Error> {
        let mut output = TerminalOutput::<std::io::Empty>::new(24, 80, None, 1000)?;
        output.process(b"hello\r\n");
        assert!(!output.output.is_rolled());
        assert_eq!(output.read_output_from(0), b"hello\r\n");
        Ok(())
    }

    /// Writing more output after a replay must append, not clobber.
    #[test]
    fn replay_does_not_disturb_appends() -> Result<(), Error> {
        let mut output = TerminalOutput::<std::io::Empty>::new(24, 80, None, 1000)?;
        output.process(b"first\r\n");
        assert_eq!(output.read_output_from(0), b"first\r\n");
        output.process(b"second\r\n");
        assert_eq!(output.read_output_from(0), b"first\r\nsecond\r\n");
        // Offset 6 is the trailing newline of \"first\"; the second write
        // begins at offset 7.
        assert_eq!(output.read_output_from(7), b"second\r\n");
        Ok(())
    }
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

        let mut output: TerminalOutput<()> =
            TerminalOutput::new(10, 40, None, 100).expect("terminal output");
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
        assert!(output.is_selecting());
        assert_eq!(output.copy_selection().as_deref(), Some("hello"));

        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;
        // Release ends the drag. The selection itself survives at this
        // layer so `App` can copy it before clearing the highlight.
        assert!(!output.is_selecting());
        assert!(output.has_selection());
        Ok(())
    }

    #[test]
    fn click_without_drag_has_no_selection() -> Result<(), Error> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut output: TerminalOutput<()> = TerminalOutput::new(10, 40, None, 100)?;
        output.process(b"hello world\r\n");
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;

        assert!(!output.has_selection());
        assert_eq!(output.copy_selection(), None);
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;
        assert_eq!(output.parser.selection_start(), None);
        Ok(())
    }

    #[test]
    fn mouse_down_pins_anchor_before_output_arrives() -> Result<(), Error> {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut output: TerminalOutput<()> = TerminalOutput::new(2, 40, None, 100)?;
        output.process(b"anchor\r\nnext");
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;
        output.process(b"\r\nnew");
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 2,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::empty(),
        })?;

        assert!(
            output
                .copy_selection()
                .is_some_and(|text| text.starts_with("anchor"))
        );
        Ok(())
    }

    #[test]
    fn selection_tick_without_anchor_does_not_scroll() -> Result<(), Error> {
        let mut output: TerminalOutput<()> = TerminalOutput::new(2, 40, None, 100)?;
        output.process(b"zero\r\none\r\ntwo\r\nthree");
        output.scroll_to_top()?;
        let before = output
            .parser
            .terminal
            .scrollbar()
            .expect("scrollbar")
            .offset;

        output.continue_selection_drag(Direction::Down, 1, 0)?;

        assert_eq!(
            output
                .parser
                .terminal
                .scrollbar()
                .expect("scrollbar")
                .offset,
            before
        );
        Ok(())
    }

    #[test]
    fn shift_on_click_does_not_start_selection() -> Result<(), Error> {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut output: TerminalOutput<()> = TerminalOutput::new(10, 40, None, 100)?;
        output.process(b"hello world\r\n");

        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::SHIFT,
        })?;
        assert!(!output.is_selecting());

        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })?;
        assert!(!output.has_selection());
        Ok(())
    }

    #[test]
    fn release_with_shift_keeps_selection() -> Result<(), Error> {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut output: TerminalOutput<()> = TerminalOutput::new(10, 40, None, 100)?;
        output.process(b"hello world\r\n");

        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })?;
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })?;
        assert!(output.has_selection());

        // The drag ends but the selection stays for a later `c` copy.
        output.handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: KeyModifiers::SHIFT,
        })?;
        assert!(output.has_selection());
        assert!(!output.is_selecting());
        Ok(())
    }
}
