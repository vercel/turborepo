use ratatui::style::{Modifier, Style};

impl tui_term::widget::Screen for crate::Screen {
    type C = crate::Cell;

    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        self.cell(row, col)
    }

    fn hide_cursor(&self) -> bool {
        self.hide_cursor()
    }

    fn cursor_position(&self) -> (u16, u16) {
        self.cursor_position()
    }
}

impl tui_term::widget::Screen for crate::EntireScreen<'_> {
    type C = crate::Cell;

    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        self.cell(row, col)
    }

    fn hide_cursor(&self) -> bool {
        true
    }

    fn cursor_position(&self) -> (u16, u16) {
        (0, 0)
    }
}

impl tui_term::widget::Cell for crate::Cell {
    fn has_contents(&self) -> bool {
        self.has_contents()
    }

    fn apply(&self, cell: &mut ratatui::buffer::Cell) {
        fill_buf_cell(self, cell);
    }
}

fn fill_buf_cell(
    screen_cell: &crate::Cell,
    buf_cell: &mut ratatui::buffer::Cell,
) {
    let fg = screen_cell.fgcolor();
    let bg = screen_cell.bgcolor();

    let contents = screen_cell.contents();
    if contents.is_empty() {
        buf_cell.set_symbol(" ");
    } else {
        buf_cell.set_symbol(contents);
    }
    let fg: Color = fg.into();
    let bg: Color = bg.into();
    let mut style = Style::reset();
    if screen_cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if screen_cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if screen_cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if screen_cell.inverse() {
        style = style.add_modifier(Modifier::REVERSED);
    }
    buf_cell.set_style(style);
    buf_cell.set_fg(fg.into());
    buf_cell.set_bg(bg.into());
}

/// Represents a foreground or background color for cells.
/// Intermediate translation layer between
/// [`vt100::Screen`] and [`ratatui::style::Color`]
#[allow(dead_code)]
enum Color {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

impl From<crate::Color> for Color {
    fn from(value: crate::Color) -> Self {
        match value {
            crate::Color::Default => Self::Reset,
            crate::Color::Idx(i) => Self::Indexed(i),
            crate::Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

impl From<Color> for crate::Color {
    fn from(value: Color) -> Self {
        match value {
            Color::Reset => Self::Default,
            Color::Black => Self::Idx(0),
            Color::Red => Self::Idx(1),
            Color::Green => Self::Idx(2),
            Color::Yellow => Self::Idx(3),
            Color::Blue => Self::Idx(4),
            Color::Magenta => Self::Idx(5),
            Color::Cyan => Self::Idx(6),
            Color::Gray => Self::Idx(7),
            Color::DarkGray => Self::Idx(8),
            Color::LightRed => Self::Idx(9),
            Color::LightGreen => Self::Idx(10),
            Color::LightYellow => Self::Idx(11),
            Color::LightBlue => Self::Idx(12),
            Color::LightMagenta => Self::Idx(13),
            Color::LightCyan => Self::Idx(14),
            Color::White => Self::Idx(15),
            Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
            Color::Indexed(i) => Self::Idx(i),
        }
    }
}

impl From<Color> for ratatui::style::Color {
    fn from(value: Color) -> Self {
        match value {
            Color::Reset => Self::Reset,
            Color::Black => Self::Black,
            Color::Red => Self::Red,
            Color::Green => Self::Green,
            Color::Yellow => Self::Yellow,
            Color::Blue => Self::Blue,
            Color::Magenta => Self::Magenta,
            Color::Cyan => Self::Cyan,
            Color::Gray => Self::Gray,
            Color::DarkGray => Self::DarkGray,
            Color::LightRed => Self::LightRed,
            Color::LightGreen => Self::LightGreen,
            Color::LightYellow => Self::LightYellow,
            Color::LightBlue => Self::LightBlue,
            Color::LightMagenta => Self::LightMagenta,
            Color::LightCyan => Self::LightCyan,
            Color::White => Self::White,
            Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
            Color::Indexed(i) => Self::Indexed(i),
        }
    }
}

#[cfg(test)]
mod test {
    use ratatui::{
        Terminal, backend::TestBackend, buffer::Buffer, layout::Rect,
        widgets::Widget,
    };
    use tui_term::widget::PseudoTerminal;

    use super::fill_buf_cell;

    #[test]
    fn empty_cell_produces_space() {
        let parser = crate::Parser::new(2, 4, 0);
        let screen = parser.screen();
        // Cell at (0,0) has no content since nothing was written
        let screen_cell = screen.cell(0, 0).unwrap();
        assert!(!screen_cell.has_contents());

        let mut buf_cell = ratatui::buffer::Cell::EMPTY;
        fill_buf_cell(screen_cell, &mut buf_cell);

        // Must produce a space, not an empty string. An empty string has
        // zero width and ratatui's diff algorithm will emit it as a no-op,
        // leaving stale content from the previous frame visible.
        assert_eq!(buf_cell.symbol(), " ");
    }

    #[test]
    fn cell_with_content_preserves_symbol() {
        let mut parser = crate::Parser::new(2, 10, 0);
        parser.process(b"Hello");
        let screen = parser.screen();
        let screen_cell = screen.cell(0, 0).unwrap();
        assert!(screen_cell.has_contents());

        let mut buf_cell = ratatui::buffer::Cell::EMPTY;
        fill_buf_cell(screen_cell, &mut buf_cell);

        assert_eq!(buf_cell.symbol(), "H");
    }

    #[test]
    fn cell_with_bold_sets_modifier() {
        let mut parser = crate::Parser::new(2, 10, 0);
        parser.process(b"\x1b[1mX");
        let screen = parser.screen();
        let screen_cell = screen.cell(0, 0).unwrap();

        let mut buf_cell = ratatui::buffer::Cell::EMPTY;
        fill_buf_cell(screen_cell, &mut buf_cell);

        assert_eq!(buf_cell.symbol(), "X");
        assert!(buf_cell.modifier.contains(ratatui::style::Modifier::BOLD));
    }

    /// Regression test for #11779: switching between task output buffers
    /// must fully repaint the pane. When a shorter output replaces a longer
    /// one, every cell in the area must be a proper space — not an empty
    /// string that ratatui's diff treats as zero-width.
    #[test]
    fn switching_screens_fully_clears_previous_content() {
        let (cols, rows): (u16, u16) = (20, 5);
        let backend = TestBackend::new(cols, rows);
        let mut terminal = Terminal::new(backend).unwrap();

        // First draw: a screen with content on every row
        let mut parser_long = crate::Parser::new(rows, cols, 0);
        parser_long
            .process(b"Line 1\r\nLine 2\r\nLine 3\r\nLine 4\r\nLine 5");
        terminal
            .draw(|f| {
                let pt = PseudoTerminal::new(parser_long.screen());
                pt.render(f.area(), f.buffer_mut());
            })
            .unwrap();

        // Second draw: a screen with only one line of content
        let mut parser_short = crate::Parser::new(rows, cols, 0);
        parser_short.process(b"Short");
        terminal
            .draw(|f| {
                let pt = PseudoTerminal::new(parser_short.screen());
                pt.render(f.area(), f.buffer_mut());
            })
            .unwrap();

        // After the second draw, the backend buffer must contain no remnants
        // of the first draw. Every cell outside "Short" and the cursor
        // should be a space.
        let buf = terminal.backend().buffer();
        let cursor_col = 5u16; // cursor sits right after "Short"
        for row in 0..rows {
            for col in 0..cols {
                let cell = &buf[ratatui::layout::Position::new(col, row)];
                let sym = cell.symbol();
                let is_text = row == 0 && col < 5;
                let is_cursor = row == 0 && col == cursor_col;
                assert!(
                    sym == " " || is_text || is_cursor,
                    "Cell ({col}, {row}) has unexpected symbol {sym:?} — \
                     stale content from previous frame leaked through"
                );
            }
        }
    }
}
