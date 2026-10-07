#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

pub(super) fn rects(glyph: char, cell_width: f32, cell_height: f32) -> Option<Vec<Rect>> {
    let light = cell_width.min(cell_height) * 0.08;
    let heavy = cell_width.min(cell_height) * 0.12;
    let horizontal = |stroke: f32| {
        let height = stroke / cell_height;
        Rect {
            left: 0.0,
            top: (1.0 - height) / 2.0,
            width: 1.0,
            height,
        }
    };
    let vertical = |stroke: f32| {
        let width = stroke / cell_width;
        Rect {
            left: (1.0 - width) / 2.0,
            top: 0.0,
            width,
            height: 1.0,
        }
    };
    let up = |stroke: f32| {
        let width = stroke / cell_width;
        let height = stroke / cell_height;
        Rect {
            left: (1.0 - width) / 2.0,
            top: 0.0,
            width,
            height: 0.5 + height / 2.0,
        }
    };
    let down = |stroke: f32| {
        let width = stroke / cell_width;
        let height = stroke / cell_height;
        Rect {
            left: (1.0 - width) / 2.0,
            top: 0.5 - height / 2.0,
            width,
            height: 0.5 + height / 2.0,
        }
    };
    let left = |stroke: f32| {
        let width = stroke / cell_width;
        let height = stroke / cell_height;
        Rect {
            left: 0.0,
            top: (1.0 - height) / 2.0,
            width: 0.5 + width / 2.0,
            height,
        }
    };
    let right = |stroke: f32| {
        let width = stroke / cell_width;
        let height = stroke / cell_height;
        Rect {
            left: 0.5 - width / 2.0,
            top: (1.0 - height) / 2.0,
            width: 0.5 + width / 2.0,
            height,
        }
    };

    Some(match glyph {
        '─' => vec![horizontal(light)],
        '━' => vec![horizontal(heavy)],
        '│' => vec![vertical(light)],
        '┃' => vec![vertical(heavy)],
        '╹' => vec![up(heavy)],
        '┌' => vec![right(light), down(light)],
        '┐' => vec![left(light), down(light)],
        '└' => vec![right(light), up(light)],
        '┘' => vec![left(light), up(light)],
        '├' => vec![vertical(light), right(light)],
        '┤' => vec![vertical(light), left(light)],
        '┬' => vec![horizontal(light), down(light)],
        '┴' => vec![horizontal(light), up(light)],
        '┼' => vec![horizontal(light), vertical(light)],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_physical_stroke_weight_across_axes() {
        let horizontal = rects('─', 9.0, 18.0).unwrap()[0];
        let vertical = rects('│', 9.0, 18.0).unwrap()[0];

        assert_eq!(horizontal.height * 18.0, vertical.width * 9.0);
    }

    #[test]
    fn reaches_cell_edges_for_contiguous_lines() {
        let horizontal = rects('━', 9.0, 18.0).unwrap()[0];
        let vertical = rects('┃', 9.0, 18.0).unwrap()[0];

        assert_eq!((horizontal.left, horizontal.width), (0.0, 1.0));
        assert_eq!((vertical.top, vertical.height), (0.0, 1.0));
    }

    #[test]
    fn falls_back_for_non_box_glyphs() {
        assert_eq!(rects('a', 9.0, 18.0), None);
    }
}
