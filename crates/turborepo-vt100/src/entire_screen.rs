use crate::term::BufWrite;
use std::io::Write;

pub struct EntireScreen<'a> {
    screen: &'a crate::Screen,
    size: (usize, u16),
}

impl<'a> EntireScreen<'a> {
    #[must_use]
    pub fn new(screen: &'a crate::Screen) -> Self {
        Self {
            size: screen.grid().size_with_contents(),
            screen,
        }
    }

    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&crate::Cell> {
        self.screen.grid().all_row(row).and_then(|r| r.get(col))
    }

    #[must_use]
    pub fn contents(&self) -> String {
        let mut s = String::new();
        self.screen.grid().write_full_contents(&mut s);
        s
    }

    /// Returns the formatted contents of the terminal by row,
    /// restricted to the given subset of columns.
    ///
    /// Formatting information will be included inline as terminal escape
    /// codes. The result will be suitable for feeding directly to a raw
    /// terminal parser, and will result in the same visual output.
    ///
    /// You are responsible for positioning the cursor before printing each
    /// row, and the final cursor position after displaying each row is
    /// unspecified.
    // the unwraps in this method shouldn't be reachable
    #[allow(clippy::missing_panics_doc)]
    pub fn rows_formatted(
        &self,
        start: u16,
        width: u16,
    ) -> impl Iterator<Item = Vec<u8>> + '_ {
        let mut wrapping = false;
        let grid = self.screen.grid();
        let (rows, _) = self.size();
        let default = crate::attrs::Attrs::default();
        grid.all_rows().take(rows).enumerate().map(move |(i, row)| {
            // number of rows in a grid is stored in a u16 (see Size), so
            // visible_rows can never return enough rows to overflow here
            let i = i.try_into().unwrap();
            let mut contents = vec![];
            // We don't need final cursor position as long as CRLF is used and not just LF
            let (_pos, attrs) = row.write_contents_formatted(
                &mut contents,
                start,
                width,
                i,
                wrapping,
                None,
                None,
            );
            if start == 0 && width == grid.size().cols {
                wrapping = row.wrapped();
            }
            // If the row ended in non-default attributes, then clear them
            if attrs != default {
                crate::term::ClearAttrs.write_buf(&mut contents);
            }
            contents
        })
    }

    /// Size required to render all contents
    #[must_use]
    pub fn size(&self) -> (usize, u16) {
        self.size
    }
}
