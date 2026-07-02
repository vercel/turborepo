use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier},
    widgets::Widget,
};

use crate::{
    convert,
    render::{CellIterator, CursorViewport, CursorVisualStyle, RenderState, RowIterator},
    style::RgbColor,
    terminal::Terminal,
};

/// Cursor information extracted during rendering.
#[derive(Debug, Clone, Default)]
pub struct CursorState {
    /// Position relative to the widget's render area where the cursor should be
    /// drawn. `None` when the cursor is hidden or outside the visible area.
    pub position: Option<Position>,
    pub style: CursorStyle,
    pub blinking: bool,
}

/// Visual style of the terminal cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Block,
    Bar,
    Underline,
}

pub struct TerminalWidget<'a, 'alloc, 'cb> {
    terminal: &'a mut Terminal<'alloc, 'cb>,
    render_state: &'a mut RenderState<'alloc>,
    focused: bool,
    cursor: CursorState,
}

impl<'a, 'alloc, 'cb> TerminalWidget<'a, 'alloc, 'cb> {
    pub fn new(
        terminal: &'a mut Terminal<'alloc, 'cb>,
        render_state: &'a mut RenderState<'alloc>,
    ) -> Self {
        Self {
            terminal,
            render_state,
            focused: false,
            cursor: CursorState::default(),
        }
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn cursor(&self) -> &CursorState {
        &self.cursor
    }
}

fn rgb_to_color(color: RgbColor) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

impl Widget for &mut TerminalWidget<'_, '_, '_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Ok(snapshot) = self.render_state.update(self.terminal) else {
            return;
        };

        let Ok(colors) = snapshot.colors() else {
            return;
        };

        let cursor_visible = snapshot.cursor_visible().unwrap_or(false);
        let cursor_viewport = snapshot.cursor_viewport().ok().flatten();
        let cursor_visual_style = snapshot.cursor_visual_style().ok();
        let cursor_blinking = snapshot.cursor_blinking().unwrap_or(false);

        self.cursor = CursorState::default();
        if self.focused
            && cursor_visible
            && let Some(CursorViewport {
                x, y, at_wide_tail, ..
            }) = cursor_viewport
            && !at_wide_tail
            && x < area.width
            && y < area.height
        {
            self.cursor.position = Some(Position::new(x, y));
            self.cursor.style = match cursor_visual_style {
                Some(CursorVisualStyle::Bar) => CursorStyle::Bar,
                Some(CursorVisualStyle::Underline) => CursorStyle::Underline,
                _ => CursorStyle::Block,
            };
            self.cursor.blinking = cursor_blinking;
        }

        let default_fg = rgb_to_color(colors.foreground);
        let default_bg = rgb_to_color(colors.background);

        let Ok(mut row_iter) = RowIterator::new() else {
            return;
        };
        let Ok(mut cell_iter) = CellIterator::new() else {
            return;
        };

        let Ok(mut row_iteration) = row_iter.update(&snapshot) else {
            return;
        };

        let mut row_idx: u16 = 0;
        while let Some(row) = row_iteration.next() {
            if row_idx >= area.height {
                break;
            }

            let row_selection = row.selection().ok().flatten();

            let Ok(mut cell_iteration) = cell_iter.update(row) else {
                row_idx += 1;
                continue;
            };

            let mut col_idx: u16 = 0;
            while let Some(cell) = cell_iteration.next() {
                if col_idx >= area.width {
                    break;
                }

                let symbol = match cell.graphemes_len() {
                    Ok(0) | Err(_) => " ".to_string(),
                    Ok(_) => match cell.graphemes() {
                        Ok(chars) => chars.into_iter().collect::<String>(),
                        Err(_) => " ".to_string(),
                    },
                };

                let fg = cell
                    .fg_color()
                    .ok()
                    .flatten()
                    .map(rgb_to_color)
                    .unwrap_or(default_fg);
                let bg = cell
                    .bg_color()
                    .ok()
                    .flatten()
                    .map(rgb_to_color)
                    .unwrap_or(default_bg);

                let cell_style = cell.style().ok();
                let mut ratatui_style = cell_style
                    .as_ref()
                    .map(|style| convert::style(style, &colors.palette))
                    .unwrap_or_default();
                ratatui_style = ratatui_style.fg(fg).bg(bg);

                if row_selection.is_some_and(|selection| {
                    col_idx >= selection.start_x && col_idx <= selection.end_x
                }) {
                    ratatui_style = ratatui_style.add_modifier(Modifier::REVERSED);
                }

                let buf_x = area.x + col_idx;
                let buf_y = area.y + row_idx;
                if buf_x < buf.area().right() && buf_y < buf.area().bottom() {
                    let buf_cell = &mut buf[(buf_x, buf_y)];
                    buf_cell.set_symbol(&symbol);
                    buf_cell.set_style(ratatui_style);
                }

                col_idx += 1;
            }

            row_idx += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, widgets::Widget};

    use super::*;
    use crate::Parser;

    /// Regression test for #11779: switching between task output buffers
    /// must fully repaint the pane.
    #[test]
    fn switching_screens_fully_clears_previous_content() {
        let (cols, rows): (u16, u16) = (20, 5);
        let backend = TestBackend::new(cols, rows);
        let mut terminal = Terminal::new(backend).expect("terminal");

        let mut parser_long = Parser::new(rows, cols, 0);
        parser_long.process(b"Line 1\r\nLine 2\r\nLine 3\r\nLine 4\r\nLine 5");
        terminal
            .draw(|frame| {
                let mut widget =
                    TerminalWidget::new(&mut parser_long.terminal, &mut parser_long.render_state);
                widget.render(frame.area(), frame.buffer_mut());
            })
            .expect("draw long");

        let mut parser_short = Parser::new(rows, cols, 0);
        parser_short.process(b"Short");
        terminal
            .draw(|frame| {
                let mut widget =
                    TerminalWidget::new(&mut parser_short.terminal, &mut parser_short.render_state);
                widget.render(frame.area(), frame.buffer_mut());
            })
            .expect("draw short");

        let buf = terminal.backend().buffer();
        let cursor_col = 5u16;
        for row in 0..rows {
            for col in 0..cols {
                let cell = &buf[ratatui::layout::Position::new(col, row)];
                let sym = cell.symbol();
                let is_text = row == 0 && col < 5;
                let is_cursor = row == 0 && col == cursor_col;
                assert!(
                    sym == " " || is_text || is_cursor,
                    "Cell ({col}, {row}) has unexpected symbol {sym:?}"
                );
            }
        }
    }
}
