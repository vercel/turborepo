use serde::{Deserialize, Serialize};

/// Schema version written in every structured terminal frame.
pub const FORMAT_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const DEFAULT_FOREGROUND: Color = Color {
    r: 201,
    g: 209,
    b: 217,
};
pub const DEFAULT_BACKGROUND: Color = Color {
    r: 13,
    g: 17,
    b: 23,
};

impl Color {
    pub fn css(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Attributes {
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Option<Underline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Underline {
    Single,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Cell {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub width: u16,
    pub foreground: Color,
    pub background: Color,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub color: Color,
    pub blinking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct Frame {
    pub version: u8,
    pub cols: u16,
    pub rows: u16,
    pub foreground: Color,
    pub background: Color,
    pub cursor: Option<Cursor>,
    pub cells: Vec<Cell>,
}

impl Frame {
    pub fn has_visible_content(&self) -> bool {
        self.cells.iter().any(|cell| {
            (!cell.attributes.invisible && !cell.text.trim().is_empty())
                || cell.background != self.background
        })
    }

    pub fn text(&self) -> String {
        let mut rows =
            vec![vec![String::from(" "); usize::from(self.cols)]; usize::from(self.rows)];
        for cell in &self.cells {
            if cell.text.is_empty()
                || cell.attributes.invisible
                || cell.x >= self.cols
                || cell.y >= self.rows
            {
                continue;
            }
            rows[usize::from(cell.y)][usize::from(cell.x)] = cell.text.clone();
            if cell.width == 2 && cell.x + 1 < self.cols {
                rows[usize::from(cell.y)][usize::from(cell.x + 1)].clear();
            }
        }
        rows.into_iter()
            .map(|line| line.join("").trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }
}

pub(crate) fn indexed_color(index: u8) -> Color {
    const ANSI: [Color; 16] = [
        Color { r: 0, g: 0, b: 0 },
        Color {
            r: 205,
            g: 49,
            b: 49,
        },
        Color {
            r: 13,
            g: 188,
            b: 121,
        },
        Color {
            r: 229,
            g: 229,
            b: 16,
        },
        Color {
            r: 36,
            g: 114,
            b: 200,
        },
        Color {
            r: 188,
            g: 63,
            b: 188,
        },
        Color {
            r: 17,
            g: 168,
            b: 205,
        },
        Color {
            r: 229,
            g: 229,
            b: 229,
        },
        Color {
            r: 102,
            g: 102,
            b: 102,
        },
        Color {
            r: 241,
            g: 76,
            b: 76,
        },
        Color {
            r: 35,
            g: 209,
            b: 139,
        },
        Color {
            r: 245,
            g: 245,
            b: 67,
        },
        Color {
            r: 59,
            g: 142,
            b: 234,
        },
        Color {
            r: 214,
            g: 112,
            b: 214,
        },
        Color {
            r: 41,
            g: 184,
            b: 219,
        },
        Color {
            r: 255,
            g: 255,
            b: 255,
        },
    ];
    if index < 16 {
        return ANSI[usize::from(index)];
    }
    if index >= 232 {
        let value = 8 + (index - 232) * 10;
        return Color {
            r: value,
            g: value,
            b: value,
        };
    }
    let value = index - 16;
    let channel = |component: u8| {
        if component == 0 {
            0
        } else {
            55 + component * 40
        }
    };
    Color {
        r: channel(value / 36),
        g: channel((value % 36) / 6),
        b: channel(value % 6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_truecolor_backgrounds_and_text() {
        let frame = crate::shot::from_ansi(
            b"\x1b[48;2;30;34;42m\x1b[38;2;196;215;240m Hi \x1b[0m".to_vec(),
            3,
            20,
            1024,
        )
        .unwrap()
        .frame;

        assert_eq!(frame.text(), " Hi");
        assert_eq!(
            frame.cells[0].background,
            Color {
                r: 30,
                g: 34,
                b: 42
            }
        );
        assert_eq!(
            frame.cells[0].foreground,
            Color {
                r: 196,
                g: 215,
                b: 240
            }
        );
    }

    #[test]
    fn maps_xterm_color_cube_values() {
        assert_eq!(
            indexed_color(1),
            Color {
                r: 205,
                g: 49,
                b: 49
            }
        );
        assert_eq!(
            indexed_color(214),
            Color {
                r: 255,
                g: 175,
                b: 0
            }
        );
        assert_eq!(
            indexed_color(244),
            Color {
                r: 128,
                g: 128,
                b: 128
            }
        );
    }

    #[test]
    fn background_paint_is_visible_content() {
        let frame = crate::shot::from_ansi(b"\x1b[48;2;30;34;42m ".to_vec(), 1, 2, 1024)
            .unwrap()
            .frame;

        assert!(frame.has_visible_content());
    }

    #[test]
    fn text_ignores_out_of_bounds_external_cells() {
        let mut frame = Frame {
            version: FORMAT_VERSION,
            cols: 1,
            rows: 1,
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            cursor: None,
            cells: Vec::new(),
        };
        frame.cells.push(Cell {
            x: 2,
            y: 0,
            text: "x".to_owned(),
            width: 1,
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            attributes: Attributes::default(),
        });

        assert_eq!(frame.text(), "");
    }

    #[test]
    fn text_ignores_invisible_cells() {
        let frame = crate::shot::from_ansi(b"\x1b[8msecret\x1b[0mvisible".to_vec(), 1, 20, 1024)
            .unwrap()
            .frame;

        assert_eq!(frame.text(), "      visible");
    }
}
