use crate::style::{self, Underline};
use ratatui::style::{Color, Modifier, Style};

pub fn rgb_color(c: style::RgbColor) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

fn resolve_color(color: &style::StyleColor, palette: &[style::RgbColor; 256]) -> Option<Color> {
    match color {
        style::StyleColor::None => None,
        style::StyleColor::Rgb(c) => Some(rgb_color(*c)),
        style::StyleColor::Palette(idx) => Some(rgb_color(palette[idx.0 as usize])),
    }
}

pub fn style(s: &style::Style, palette: &[style::RgbColor; 256]) -> Style {
    let mut result = Style::default();

    if let Some(fg) = resolve_color(&s.fg_color, palette) {
        result = result.fg(fg);
    }
    if let Some(bg) = resolve_color(&s.bg_color, palette) {
        result = result.bg(bg);
    }

    let mut mods = Modifier::empty();
    if s.bold {
        mods |= Modifier::BOLD;
    }
    if s.italic {
        mods |= Modifier::ITALIC;
    }
    if s.faint {
        mods |= Modifier::DIM;
    }
    if s.blink {
        mods |= Modifier::SLOW_BLINK;
    }
    if s.inverse {
        mods |= Modifier::REVERSED;
    }
    if s.invisible {
        mods |= Modifier::HIDDEN;
    }
    if s.strikethrough {
        mods |= Modifier::CROSSED_OUT;
    }
    if !matches!(s.underline, Underline::None) {
        mods |= Modifier::UNDERLINED;
    }

    result.add_modifier(mods)
}
