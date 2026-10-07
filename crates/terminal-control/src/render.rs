use std::path::Path;

use anyhow::{Context, Result};

use crate::frame::{Cell, Frame, Underline};

mod box_drawing;

const MAX_PNG_PIXELS: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Options {
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub padding: f32,
    pub font_family: String,
    pub show_cursor: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cell_width: 9.0,
            cell_height: 18.0,
            font_size: 14.0,
            padding: 18.0,
            font_family: "JetBrains Mono, SFMono-Regular, Menlo, monospace".to_owned(),
            show_cursor: true,
        }
    }
}

pub fn svg(frame: &Frame, options: &Options) -> String {
    let width = f32::from(frame.cols) * options.cell_width + options.padding * 2.0;
    let height = f32::from(frame.rows) * options.cell_height + options.padding * 2.0;
    let mut output = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect width="100%" height="100%" rx="10" fill="{}"/><g font-family="{}" font-size="{}" xml:space="preserve">"#,
        frame.background.css(),
        xml(&options.font_family),
        options.font_size,
    );
    for cell in &frame.cells {
        if cell.background != frame.background {
            output.push_str(&format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                options.padding + f32::from(cell.x) * options.cell_width,
                options.padding + f32::from(cell.y) * options.cell_height,
                f32::from(cell.width) * options.cell_width,
                options.cell_height,
                cell.background.css(),
            ));
        }
    }
    for cell in &frame.cells {
        if !cell.text.is_empty() && !cell.attributes.invisible {
            output.push_str(&graphic(cell, options).unwrap_or_else(|| text(cell, options)));
        }
    }
    if options.show_cursor
        && let Some(cursor) = &frame.cursor
    {
        let x = options.padding + f32::from(cursor.x) * options.cell_width;
        let y = options.padding + f32::from(cursor.y) * options.cell_height;
        output.push_str(&format!(
            r#"<rect x="{x}" y="{y}" width="{}" height="{}" fill="{}" opacity="0.32"/>"#,
            options.cell_width,
            options.cell_height,
            cursor.color.css(),
        ));
    }
    output.push_str("</g></svg>");
    output
}

pub fn png(svg: &str, path: &Path, pixel_ratio: f32) -> Result<()> {
    PngRenderer::new().render(svg, path, pixel_ratio)
}

pub struct PngRenderer {
    options: resvg::usvg::Options<'static>,
}

impl PngRenderer {
    pub fn new() -> Self {
        let mut options = resvg::usvg::Options::default();
        options.fontdb_mut().load_system_fonts();
        Self { options }
    }

    pub fn render(&self, svg: &str, path: &Path, pixel_ratio: f32) -> Result<()> {
        if !pixel_ratio.is_finite() || pixel_ratio <= 0.0 {
            anyhow::bail!("PNG pixel ratio must be finite and greater than zero");
        }
        let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &self.options)
            .context("parse rendered SVG")?;
        let size = tree.size().to_int_size();
        let width = ((size.width() as f64) * f64::from(pixel_ratio)).ceil();
        let height = ((size.height() as f64) * f64::from(pixel_ratio)).ceil();
        if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
            anyhow::bail!("PNG dimensions exceed the supported range");
        }
        let width = width as u32;
        let height = height as u32;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .context("PNG pixel count overflow")?;
        if pixels > MAX_PNG_PIXELS {
            anyhow::bail!("PNG would contain {pixels} pixels; limit is {MAX_PNG_PIXELS}");
        }
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).context("allocate PNG canvas")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(pixel_ratio, pixel_ratio),
            &mut pixmap.as_mut(),
        );
        pixmap.save_png(path).context("write PNG artifact")?;
        Ok(())
    }
}

impl Default for PngRenderer {
    fn default() -> Self {
        Self::new()
    }
}

fn graphic(cell: &Cell, options: &Options) -> Option<String> {
    let mut chars = cell.text.chars();
    let char = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let x = options.padding + f32::from(cell.x) * options.cell_width;
    let y = options.padding + f32::from(cell.y) * options.cell_height;
    let width = options.cell_width * f32::from(cell.width);
    let height = options.cell_height;
    let rect = |x: f32, y: f32, width: f32, height: f32, opacity: Option<f32>| {
        format!(
            r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" fill="{}"{}/>"#,
            cell.foreground.css(),
            opacity.map_or_else(String::new, |value| format!(r#" opacity="{value}""#)),
        )
    };
    let stroke_width = width.min(height) * 0.08;
    let stroke_rect = |left: f32, top: f32, wide: f32, tall: f32| {
        format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="{stroke_width}"/>"#,
            x + width * left,
            y + height * top,
            width * wide,
            height * tall,
            cell.foreground.css(),
        )
    };
    let single = |left: f32, top: f32, wide: f32, tall: f32| {
        rect(
            x + width * left,
            y + height * top,
            width * wide,
            height * tall,
            None,
        )
    };
    let circle = |center_x: f32, center_y: f32, radius: f32| {
        format!(
            r#"<circle cx="{}" cy="{}" r="{}" fill="{}"/>"#,
            x + width * center_x,
            y + height * center_y,
            radius,
            cell.foreground.css(),
        )
    };
    let ring = |center_x: f32, center_y: f32, radius: f32| {
        format!(
            r#"<circle cx="{}" cy="{}" r="{}" fill="none" stroke="{}" stroke-width="{stroke_width}"/>"#,
            x + width * center_x,
            y + height * center_y,
            radius,
            cell.foreground.css(),
        )
    };
    let diamond = |scale: f32, filled: bool| {
        let center_x = x + width * 0.5;
        let center_y = y + height * 0.5;
        let half_width = width * 0.42 * scale;
        let half_height = height * 0.36 * scale;
        let points = format!(
            "{center_x},{} {},{center_y} {center_x},{} {},{center_y}",
            center_y - half_height,
            center_x + half_width,
            center_y + half_height,
            center_x - half_width,
        );
        if filled {
            return format!(
                r#"<polygon points="{points}" fill="{}"/>"#,
                cell.foreground.css()
            );
        }
        format!(
            r#"<polygon points="{points}" fill="none" stroke="{}" stroke-width="{stroke_width}"/>"#,
            cell.foreground.css(),
        )
    };
    let codepoint = char as u32;
    if (0x2800..=0x28ff).contains(&codepoint) {
        return Some(graphic_opacity(
            cell,
            braille_dots(codepoint - 0x2800, &circle, width.min(height) * 0.09),
        ));
    }
    if let Some(rects) = box_drawing::rects(char, width, height) {
        return Some(graphic_opacity(
            cell,
            rects
                .into_iter()
                .map(|rect| single(rect.left, rect.top, rect.width, rect.height))
                .collect(),
        ));
    }
    let graphic = match char {
        '█' => single(0.0, 0.0, 1.0, 1.0),
        '▀' => single(0.0, 0.0, 1.0, 0.5),
        '▄' => single(0.0, 0.5, 1.0, 0.5),
        '▌' => single(0.0, 0.0, 0.5, 1.0),
        '▐' => single(0.5, 0.0, 0.5, 1.0),
        '▁' => single(0.0, 7.0 / 8.0, 1.0, 1.0 / 8.0),
        '▂' => single(0.0, 6.0 / 8.0, 1.0, 2.0 / 8.0),
        '▃' => single(0.0, 5.0 / 8.0, 1.0, 3.0 / 8.0),
        '▅' => single(0.0, 3.0 / 8.0, 1.0, 5.0 / 8.0),
        '▆' => single(0.0, 2.0 / 8.0, 1.0, 6.0 / 8.0),
        '▇' => single(0.0, 1.0 / 8.0, 1.0, 7.0 / 8.0),
        '▏' => single(0.0, 0.0, 1.0 / 8.0, 1.0),
        '▎' => single(0.0, 0.0, 2.0 / 8.0, 1.0),
        '▍' => single(0.0, 0.0, 3.0 / 8.0, 1.0),
        '▋' => single(0.0, 0.0, 5.0 / 8.0, 1.0),
        '▊' => single(0.0, 0.0, 6.0 / 8.0, 1.0),
        '▉' => single(0.0, 0.0, 7.0 / 8.0, 1.0),
        '▔' => single(0.0, 0.0, 1.0, 1.0 / 8.0),
        '▖' => single(0.0, 0.5, 0.5, 0.5),
        '▗' => single(0.5, 0.5, 0.5, 0.5),
        '▘' => single(0.0, 0.0, 0.5, 0.5),
        '▝' => single(0.5, 0.0, 0.5, 0.5),
        '▚' => single(0.0, 0.0, 0.5, 0.5) + &single(0.5, 0.5, 0.5, 0.5),
        '▞' => single(0.5, 0.0, 0.5, 0.5) + &single(0.0, 0.5, 0.5, 0.5),
        '▙' => single(0.0, 0.0, 0.5, 1.0) + &single(0.5, 0.5, 0.5, 0.5),
        '▛' => single(0.0, 0.0, 0.5, 1.0) + &single(0.5, 0.0, 0.5, 0.5),
        '▜' => single(0.5, 0.0, 0.5, 1.0) + &single(0.0, 0.0, 0.5, 0.5),
        '▟' => single(0.5, 0.0, 0.5, 1.0) + &single(0.0, 0.5, 0.5, 0.5),
        '▣' => stroke_rect(0.18, 0.18, 0.64, 0.64) + &single(0.38, 0.38, 0.24, 0.24),
        '■' => single(0.1, 0.18, 0.8, 0.64),
        '⬝' => single(0.32, 0.38, 0.36, 0.28),
        '◆' => diamond(1.0, true),
        '◇' => diamond(1.0, false),
        '◈' => diamond(1.0, false) + &diamond(0.42, true),
        '⬥' => diamond(0.82, true),
        '⬩' | '⬪' | '⬖' => diamond(0.52, true),
        '●' => circle(0.5, 0.52, width.min(height) * 0.32),
        '○' => ring(0.5, 0.52, width.min(height) * 0.32),
        '◉' | '◍' => {
            ring(0.5, 0.52, width.min(height) * 0.32) + &circle(0.5, 0.52, width.min(height) * 0.15)
        }
        '◔' => ring(0.5, 0.52, width.min(height) * 0.32) + &single(0.5, 0.2, 0.32, 0.32),
        _ => return None,
    };
    Some(graphic_opacity(cell, graphic))
}

fn graphic_opacity(cell: &Cell, graphic: String) -> String {
    if cell.attributes.faint {
        format!(r#"<g opacity="0.55">{graphic}</g>"#)
    } else {
        graphic
    }
}

fn braille_dots(pattern: u32, circle: &impl Fn(f32, f32, f32) -> String, radius: f32) -> String {
    [
        (0x01, 0.34, 0.2),
        (0x02, 0.34, 0.38),
        (0x04, 0.34, 0.56),
        (0x40, 0.34, 0.74),
        (0x08, 0.66, 0.2),
        (0x10, 0.66, 0.38),
        (0x20, 0.66, 0.56),
        (0x80, 0.66, 0.74),
    ]
    .into_iter()
    .filter(|(bit, _, _)| pattern & bit != 0)
    .map(|(_, x, y)| circle(x, y, radius))
    .collect::<String>()
}

fn text(cell: &Cell, options: &Options) -> String {
    let x = options.padding + f32::from(cell.x) * options.cell_width;
    let y = options.padding + f32::from(cell.y) * options.cell_height + options.cell_height * 0.78;
    let decorations = [
        cell.attributes
            .underline
            .map(|Underline::Single| "underline"),
        cell.attributes.strikethrough.then_some("line-through"),
        cell.attributes.overline.then_some("overline"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    format!(
        r#"<text x="{x}" y="{y}" fill="{}"{}{}{}{}>{}</text>"#,
        cell.foreground.css(),
        if cell.attributes.bold {
            " font-weight=\"700\""
        } else {
            ""
        },
        if cell.attributes.italic {
            " font-style=\"italic\""
        } else {
            ""
        },
        if cell.attributes.faint {
            " opacity=\"0.55\""
        } else {
            ""
        },
        if decorations.is_empty() {
            String::new()
        } else {
            format!(" text-decoration=\"{decorations}\"")
        },
        xml(&cell.text),
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Attributes, Color, Frame, Underline};

    #[test]
    fn rejects_png_allocations_above_the_pixel_limit() {
        let path = std::env::temp_dir().join("termctrl-oversized-png-test.png");
        let error = PngRenderer::new()
            .render(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#,
                &path,
                10_000.0,
            )
            .unwrap_err();

        assert!(error.to_string().contains("limit"));
        assert!(!path.exists());
    }

    #[test]
    fn emits_background_and_text_styles_in_svg() {
        let frame = Frame {
            version: 1,
            cols: 4,
            rows: 1,
            foreground: Color {
                r: 255,
                g: 255,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: vec![crate::frame::Cell {
                x: 0,
                y: 0,
                text: "Hi".to_owned(),
                width: 2,
                foreground: Color { r: 1, g: 2, b: 3 },
                background: Color { r: 4, g: 5, b: 6 },
                attributes: Attributes {
                    bold: true,
                    underline: Some(Underline::Single),
                    ..Attributes::default()
                },
            }],
        };

        let output = svg(&frame, &Options::default());

        assert!(output.contains("#040506"));
        assert!(output.contains("#010203"));
        assert!(output.contains("font-weight=\"700\""));
        assert!(output.contains("text-decoration=\"underline\""));
    }

    #[test]
    fn renders_block_elements_as_geometry_instead_of_font_glyphs() {
        let frame = Frame {
            version: 1,
            cols: 1,
            rows: 1,
            foreground: Color {
                r: 255,
                g: 255,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: vec![crate::frame::Cell {
                x: 0,
                y: 0,
                text: "▀".to_owned(),
                width: 1,
                foreground: Color {
                    r: 255,
                    g: 255,
                    b: 255,
                },
                background: Color { r: 0, g: 0, b: 0 },
                attributes: Attributes::default(),
            }],
        };

        let output = svg(&frame, &Options::default());

        assert!(output.contains("height=\"9\""));
        assert!(!output.contains(">▀</text>"));
    }

    #[test]
    fn renders_horizontal_box_drawing_as_contiguous_geometry() {
        let frame = Frame {
            version: 1,
            cols: 2,
            rows: 1,
            foreground: Color {
                r: 166,
                g: 184,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: ["━", "━"]
                .into_iter()
                .enumerate()
                .map(|(x, text)| crate::frame::Cell {
                    x: x as u16,
                    y: 0,
                    text: text.to_owned(),
                    width: 1,
                    foreground: Color {
                        r: 166,
                        g: 184,
                        b: 255,
                    },
                    background: Color { r: 0, g: 0, b: 0 },
                    attributes: Attributes {
                        faint: true,
                        ..Attributes::default()
                    },
                })
                .collect(),
        };

        let output = svg(&frame, &Options::default());

        assert!(!output.contains(">━</text>"));
        assert_eq!(output.matches("<rect").count(), 3);
        assert_eq!(output.matches("<g opacity=\"0.55\">").count(), 2);
    }

    #[test]
    fn renders_opencode_box_drawing_as_geometry() {
        let glyphs = ["│", "┃", "╹", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼"];
        let frame = Frame {
            version: 1,
            cols: glyphs.len() as u16,
            rows: 1,
            foreground: Color {
                r: 166,
                g: 184,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: glyphs
                .into_iter()
                .enumerate()
                .map(|(x, text)| crate::frame::Cell {
                    x: x as u16,
                    y: 0,
                    text: text.to_owned(),
                    width: 1,
                    foreground: Color {
                        r: 166,
                        g: 184,
                        b: 255,
                    },
                    background: Color { r: 0, g: 0, b: 0 },
                    attributes: Attributes::default(),
                })
                .collect(),
        };

        let output = svg(&frame, &Options::default());

        for glyph in glyphs {
            assert!(!output.contains(&format!(">{glyph}</text>")));
        }
    }

    #[test]
    fn renders_opencode_status_glyphs_as_geometry() {
        let frame = Frame {
            version: 1,
            cols: 16,
            rows: 1,
            foreground: Color {
                r: 80,
                g: 140,
                b: 220,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: [
                "■", "⬝", "▣", "◆", "◇", "◈", "⬥", "⬩", "⬪", "⬖", "●", "○", "◉", "◍", "◔",
            ]
            .into_iter()
            .enumerate()
            .map(|(x, text)| crate::frame::Cell {
                x: x as u16,
                y: 0,
                text: text.to_owned(),
                width: 1,
                foreground: Color {
                    r: 80,
                    g: 140,
                    b: 220,
                },
                background: Color { r: 0, g: 0, b: 0 },
                attributes: Attributes::default(),
            })
            .collect(),
        };

        let output = svg(&frame, &Options::default());

        for text in [
            "■", "⬝", "▣", "◆", "◇", "◈", "⬥", "⬩", "⬪", "⬖", "●", "○", "◉", "◍", "◔",
        ] {
            assert!(!output.contains(&format!(">{text}</text>")));
        }
    }

    #[test]
    fn renders_braille_spinner_frames_as_geometry() {
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame = Frame {
            version: 1,
            cols: frames.len() as u16,
            rows: 1,
            foreground: Color {
                r: 180,
                g: 130,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: frames
                .into_iter()
                .enumerate()
                .map(|(x, text)| crate::frame::Cell {
                    x: x as u16,
                    y: 0,
                    text: text.to_owned(),
                    width: 1,
                    foreground: Color {
                        r: 180,
                        g: 130,
                        b: 255,
                    },
                    background: Color { r: 0, g: 0, b: 0 },
                    attributes: Attributes::default(),
                })
                .collect(),
        };

        let output = svg(&frame, &Options::default());

        assert!(output.contains("<circle"));
        for text in frames {
            assert!(!output.contains(&format!(">{text}</text>")));
        }
    }

    #[test]
    fn renders_shade_characters_as_font_glyphs() {
        let frame = Frame {
            version: 1,
            cols: 3,
            rows: 1,
            foreground: Color {
                r: 255,
                g: 255,
                b: 255,
            },
            background: Color { r: 0, g: 0, b: 0 },
            cursor: None,
            cells: ["░", "▒", "▓"]
                .into_iter()
                .enumerate()
                .map(|(x, text)| crate::frame::Cell {
                    x: x as u16,
                    y: 0,
                    text: text.to_owned(),
                    width: 1,
                    foreground: Color {
                        r: 255,
                        g: 255,
                        b: 255,
                    },
                    background: Color { r: 0, g: 0, b: 0 },
                    attributes: Attributes::default(),
                })
                .collect(),
        };

        let output = svg(&frame, &Options::default());

        assert!(output.contains(">░</text>"));
        assert!(output.contains(">▒</text>"));
        assert!(output.contains(">▓</text>"));
        assert!(!output.contains("opacity=\"0.25\""));
        assert!(!output.contains("opacity=\"0.5\""));
        assert!(!output.contains("opacity=\"0.75\""));
    }
}
