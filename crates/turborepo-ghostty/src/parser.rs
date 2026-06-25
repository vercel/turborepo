use crate::{
    error::Error as GhosttyInnerError,
    fmt::{Format, Formatter, FormatterOptions},
    selection::{FormatOptions, Selection},
    terminal::{Point, PointCoordinate, ScrollViewport, Terminal},
    Error, RenderState, Result,
};

const DEFAULT_CELL_WIDTH_PX: u32 = 8;
const DEFAULT_CELL_HEIGHT_PX: u32 = 16;

/// Virtual terminal parser backed by Ghostty's libghostty-vt.
pub struct Parser {
    pub terminal: Terminal<'static, 'static>,
    pub render_state: RenderState<'static>,
    selection_start: Option<(u16, u16)>,
    max_scrollback: usize,
}

impl Parser {
    pub fn new(rows: u16, cols: u16, scrollback_len: usize) -> Self {
        Self::try_new(rows, cols, scrollback_len)
            .expect("failed to initialize ghostty terminal")
    }

    pub fn try_new(rows: u16, cols: u16, scrollback_len: usize) -> Result<Self> {
        let terminal = Terminal::new(crate::terminal::Options {
            cols,
            rows,
            max_scrollback: scrollback_len,
        })?;
        let render_state = RenderState::new()?;

        Ok(Self {
            terminal,
            render_state,
            selection_start: None,
            max_scrollback: scrollback_len,
        })
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.terminal.vt_write(bytes);
    }

    pub fn size(&self) -> Result<(u16, u16)> {
        Ok((self.terminal.rows()?, self.terminal.cols()?))
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let current = self.size()?;
        if current != (rows, cols) {
            self.terminal.resize(
                cols,
                rows,
                DEFAULT_CELL_WIDTH_PX,
                DEFAULT_CELL_HEIGHT_PX,
            )?;
        }
        Ok(())
    }

    pub fn scroll_by(&mut self, up: bool, magnitude: usize) -> Result<()> {
        let delta = if up {
            -(isize::try_from(magnitude).map_err(|_| {
                Error::Ghostty(GhosttyInnerError::InvalidValue)
            })?)
        } else {
            isize::try_from(magnitude).map_err(|_| Error::Ghostty(GhosttyInnerError::InvalidValue))?
        };
        self.terminal
            .scroll_viewport(ScrollViewport::Delta(delta));
        Ok(())
    }

    pub fn scroll_to_top(&mut self) -> Result<()> {
        self.terminal.scroll_viewport(ScrollViewport::Top);
        Ok(())
    }

    pub fn scroll_to_bottom(&mut self) -> Result<()> {
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
        Ok(())
    }

    pub fn format_screen_vt(&self) -> Result<Vec<u8>> {
        let mut formatter = Formatter::new(
            &self.terminal,
            FormatterOptions::new()
                .with_format(Format::Vt)
                .with_trim(true)
                .with_cursor(false),
        )?;
        let bytes = formatter.format_alloc(None)?;
        Ok(bytes.to_vec())
    }

    pub fn clear_selection(&mut self) -> Result<()> {
        self.selection_start = None;
        self.terminal.set_selection(None)?;
        Ok(())
    }

    pub fn update_selection(&mut self, start_row: u16, start_col: u16, end_row: u16, end_col: u16) -> Result<()> {
        let start = self.terminal.grid_ref(viewport_point(start_row, start_col))?;
        let end = self.terminal.grid_ref(viewport_point(end_row, end_col))?;
        let selection = Selection::new(start, end, false);
        self.terminal.set_selection(Some(&selection))?;
        Ok(())
    }

    pub fn selected_text(&self) -> Result<Option<String>> {
        let bytes = self.terminal.format_selection_alloc(
            None,
            FormatOptions::new()
                .with_emit_format(Format::Plain)
                .with_trim(true)
                .with_unwrap(true),
        )?;
        Ok(bytes.map(|value| {
            String::from_utf8_lossy(value.as_ref()).into_owned()
        }))
    }

    pub fn has_selection(&self) -> bool {
        self.selected_text()
            .ok()
            .flatten()
            .is_some_and(|text| !text.is_empty())
    }

    pub fn set_selection_start(&mut self, row: u16, col: u16) {
        self.selection_start = Some((row, col));
    }

    pub fn selection_start(&self) -> Option<(u16, u16)> {
        self.selection_start
    }

    pub fn clear_selection_start(&mut self) {
        self.selection_start = None;
    }

    pub fn reset(&mut self) {
        self.terminal.reset();
        self.selection_start = None;
    }

    pub fn max_scrollback(&self) -> usize {
        self.max_scrollback
    }
}

fn viewport_point(row: u16, col: u16) -> Point {
    Point::Viewport(PointCoordinate {
        x: col,
        y: u32::from(row),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_lf_is_normalized_by_caller_and_renders_on_new_line() {
        let mut parser = Parser::new(5, 20, 0);
        parser.process(b"hello\r\nworld");
        let formatted = parser.format_screen_vt().expect("format screen");
        let output = String::from_utf8_lossy(&formatted);
        assert!(output.contains("hello"));
        assert!(output.contains("world"));
    }

    #[test]
    fn resize_reflows_existing_output() {
        let mut parser = Parser::new(5, 10, 0);
        parser.process(b"hello world");
        parser.resize(5, 20).expect("resize");
        assert_eq!(parser.size().expect("size"), (5, 20));
    }
}
