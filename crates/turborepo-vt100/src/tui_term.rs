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

impl<'a> tui_term::widget::Screen for crate::EntireScreen<'a> {
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

    buf_cell.set_symbol(screen_cell.contents());
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
