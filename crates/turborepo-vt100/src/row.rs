use std::mem;

use crate::term::BufWrite as _;

#[derive(Clone, Debug)]
pub struct Row {
    cells: Vec<crate::Cell>,
    // Indicates if the next row is wrapped contents from this row
    wrapped: bool,
}

impl Row {
    pub fn new(cols: u16) -> Self {
        Self {
            cells: vec![crate::Cell::new(); usize::from(cols)],
            wrapped: false,
        }
    }

    pub fn cols(&self) -> u16 {
        self.cells
            .len()
            .try_into()
            // we limit the number of cols to a u16 (see Size)
            .unwrap()
    }

    // Returns the number of columns excluding any trailing cells without content
    pub fn cols_with_content(&self) -> u16 {
        self.cells
            .iter()
            .enumerate()
            .rev()
            // Iterate backwards through cells until we find one with content
            .skip_while(|(_, cell)| !cell.has_contents())
            // Add one to index to get length
            .map(|(i, _)| i + 1)
            .next()
            .unwrap_or_default()
            .try_into()
            .expect("number of cells is limited to u16")
    }

    pub fn clear(&mut self, attrs: crate::attrs::Attrs) {
        for cell in &mut self.cells {
            cell.clear(attrs);
        }
        self.wrapped = false;
    }

    pub fn is_blank(&self) -> bool {
        self.cells().all(|cell| !cell.has_contents())
    }

    fn cells(&self) -> impl Iterator<Item = &crate::Cell> {
        self.cells.iter()
    }

    pub(crate) fn cells_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut crate::Cell> {
        self.cells.iter_mut()
    }

    pub fn get(&self, col: u16) -> Option<&crate::Cell> {
        self.cells.get(usize::from(col))
    }

    pub fn get_mut(&mut self, col: u16) -> Option<&mut crate::Cell> {
        self.cells.get_mut(usize::from(col))
    }

    pub fn insert(&mut self, i: u16, cell: crate::Cell) {
        self.cells.insert(usize::from(i), cell);
        self.wrapped = false;
    }

    pub fn remove(&mut self, i: u16) {
        self.clear_wide(i);
        self.cells.remove(usize::from(i));
        self.wrapped = false;
    }

    pub fn erase(&mut self, i: u16, attrs: crate::attrs::Attrs) {
        let wide = self.cells[usize::from(i)].is_wide();
        self.clear_wide(i);
        self.cells[usize::from(i)].clear(attrs);
        if i == self.cols() - if wide { 2 } else { 1 } {
            self.wrapped = false;
        }
    }

    pub fn truncate(&mut self, len: u16) {
        self.cells.truncate(usize::from(len));
        self.wrapped = false;
        if let Some(last_cell) = self.cells.last_mut() {
            if last_cell.is_wide() {
                last_cell.clear(*last_cell.attrs());
            }
        }
    }

    /// Resize a row to the provided length.
    ///
    /// Returns any cells that no longer fit in the row
    #[must_use]
    pub fn resize(
        &mut self,
        len: u16,
        cell: crate::Cell,
    ) -> Option<std::vec::Drain<crate::Cell>> {
        if len < self.cols_with_content() {
            // need to trim the row
            self.wrapped = true;
            // first drain and drop any trailing empty cells
            Some(self.cells.drain(usize::from(len)..))
        } else {
            self.cells.resize(usize::from(len), cell);
            None
        }
    }

    /// Handles overflow from a previous row while leaving the row at len
    ///
    /// Any cells that do not fit in the row will be added to the provided overflow buffer
    pub fn handle_overflow(
        &mut self,
        len: u16,
        overflow_buffer: &mut Vec<crate::Cell>,
    ) {
        // Trim any trailing empty cells from the row
        let cells_with_content = self.cols_with_content();
        self.truncate(cells_with_content);
        // Swapping is more efficient than multiple insertions
        mem::swap(&mut self.cells, overflow_buffer);
        let len = usize::from(len);
        match len.cmp(&self.cells.len()) {
            std::cmp::Ordering::Less => {
                // We need to trim some cells and put them into the overflow buffer
                let num_cells_to_cut = self.cells.len() - len;
                // TODO: maybe a more efficient way to bulk insert here
                for cell in self.cells.drain(num_cells_to_cut..) {
                    overflow_buffer.insert(0, cell);
                }
            }
            std::cmp::Ordering::Greater => {
                // We can add some of the original cells back
                let num_cells_to_add = (len - self.cells.len())
                    // but not more cells that are still in the overflow
                    .min(overflow_buffer.len());
                self.cells.extend(overflow_buffer.drain(..num_cells_to_add));
            }
            std::cmp::Ordering::Equal => (),
        }
        debug_assert!(
            self.cols() <= len.try_into().unwrap(),
            "resize should only add empty cells"
        );
        self.cells.resize(len, crate::Cell::new());
        debug_assert_eq!(
            self.cols(),
            len.try_into().unwrap(),
            "row should be left with an appropriate length"
        );
        self.wrap(!overflow_buffer.is_empty());
    }

    /// Reclaims cells in a row the result from wrapping and places them into the buffer
    ///
    /// Should only be called on rows that contain wrapped cells i.e. the previous row is wrapped
    pub fn reclaim_wrapped_cells(
        &mut self,
        available_space: u16,
        buffer: &mut Vec<crate::Cell>,
    ) {
        // We only want to give cells that have content
        let cells_to_give = self.cols_with_content().min(available_space);
        buffer.extend(self.cells.drain(..usize::from(cells_to_give)));
    }

    // Appends cells in given buffer to the current row
    pub fn append_cells(
        &mut self,
        len: usize,
        buffer: &mut Vec<crate::Cell>,
    ) {
        // Safeguard against panic from drain range going out of bounds
        let len = len.min(buffer.len());
        self.cells.extend(buffer.drain(..len));
    }

    pub fn wrap(&mut self, wrap: bool) {
        self.wrapped = wrap;
    }

    pub fn wrapped(&self) -> bool {
        self.wrapped
    }

    pub fn clear_wide(&mut self, col: u16) {
        let cell = &self.cells[usize::from(col)];
        let other = if cell.is_wide() {
            &mut self.cells[usize::from(col + 1)]
        } else if cell.is_wide_continuation() {
            &mut self.cells[usize::from(col - 1)]
        } else {
            return;
        };
        other.clear(*other.attrs());
    }

    pub fn write_contents(
        &self,
        contents: &mut String,
        start: u16,
        width: u16,
        wrapping: bool,
    ) {
        let mut prev_was_wide = false;

        let mut prev_col = start;
        for (col, cell) in self
            .cells()
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            if cell.has_contents() {
                for _ in 0..(col - prev_col) {
                    contents.push(' ');
                }
                prev_col += col - prev_col;

                contents.push_str(&cell.contents());
                prev_col += if cell.is_wide() { 2 } else { 1 };
            }
        }
        if prev_col == start && wrapping {
            contents.push('\n');
        }
    }

    pub fn write_contents_formatted(
        &self,
        contents: &mut Vec<u8>,
        start: u16,
        width: u16,
        row: u16,
        wrapping: bool,
        prev_pos: Option<crate::grid::Pos>,
        prev_attrs: Option<crate::attrs::Attrs>,
    ) -> (crate::grid::Pos, crate::attrs::Attrs) {
        let mut prev_was_wide = false;
        let default_cell = crate::Cell::new();

        let mut prev_pos = prev_pos.unwrap_or_else(|| {
            if wrapping {
                crate::grid::Pos {
                    row: row - 1,
                    col: self.cols(),
                }
            } else {
                crate::grid::Pos { row, col: start }
            }
        });
        let mut prev_attrs = prev_attrs.unwrap_or_default();

        let first_cell = &self.cells[usize::from(start)];
        if wrapping && first_cell == &default_cell {
            let default_attrs = default_cell.attrs();
            if &prev_attrs != default_attrs {
                default_attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *default_attrs;
            }
            contents.push(b' ');
            crate::term::Backspace.write_buf(contents);
            crate::term::EraseChar::new(1).write_buf(contents);
            prev_pos = crate::grid::Pos { row, col: 0 };
        }

        let mut erase: Option<(u16, &crate::attrs::Attrs)> = None;
        for (col, cell) in self
            .cells()
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            let pos = crate::grid::Pos { row, col };

            if let Some((prev_col, attrs)) = erase {
                if cell.has_contents() || cell.attrs() != attrs {
                    let new_pos = crate::grid::Pos { row, col: prev_col };
                    if wrapping
                        && prev_pos.row + 1 == new_pos.row
                        && prev_pos.col >= self.cols()
                    {
                        if new_pos.col > 0 {
                            contents.extend(
                                " ".repeat(usize::from(new_pos.col))
                                    .as_bytes(),
                            );
                        } else {
                            contents.extend(b" ");
                            crate::term::Backspace.write_buf(contents);
                        }
                    } else {
                        crate::term::MoveFromTo::new(prev_pos, new_pos)
                            .write_buf(contents);
                    }
                    prev_pos = new_pos;
                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }
                    crate::term::EraseChar::new(pos.col - prev_col)
                        .write_buf(contents);
                    erase = None;
                }
            }

            if cell != &default_cell {
                let attrs = cell.attrs();
                if cell.has_contents() {
                    if pos != prev_pos {
                        if !wrapping
                            || prev_pos.row + 1 != pos.row
                            || prev_pos.col
                                < self.cols() - u16::from(cell.is_wide())
                            || pos.col != 0
                        {
                            crate::term::MoveFromTo::new(prev_pos, pos)
                                .write_buf(contents);
                        }
                        prev_pos = pos;
                    }

                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }

                    prev_pos.col += if cell.is_wide() { 2 } else { 1 };
                    let cell_contents = cell.contents();
                    contents.extend(cell_contents.as_bytes());
                } else if erase.is_none() {
                    erase = Some((pos.col, attrs));
                }
            }
        }
        if let Some((prev_col, attrs)) = erase {
            let new_pos = crate::grid::Pos { row, col: prev_col };
            if wrapping
                && prev_pos.row + 1 == new_pos.row
                && prev_pos.col >= self.cols()
            {
                if new_pos.col > 0 {
                    contents.extend(
                        " ".repeat(usize::from(new_pos.col)).as_bytes(),
                    );
                } else {
                    contents.extend(b" ");
                    crate::term::Backspace.write_buf(contents);
                }
            } else {
                crate::term::MoveFromTo::new(prev_pos, new_pos)
                    .write_buf(contents);
            }
            prev_pos = new_pos;
            if &prev_attrs != attrs {
                attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *attrs;
            }
            crate::term::ClearRowForward.write_buf(contents);
        }

        (prev_pos, prev_attrs)
    }

    // while it's true that most of the logic in this is identical to
    // write_contents_formatted, i can't figure out how to break out the
    // common parts without making things noticeably slower.
    pub fn write_contents_diff(
        &self,
        contents: &mut Vec<u8>,
        prev: &Self,
        start: u16,
        width: u16,
        row: u16,
        wrapping: bool,
        prev_wrapping: bool,
        mut prev_pos: crate::grid::Pos,
        mut prev_attrs: crate::attrs::Attrs,
    ) -> (crate::grid::Pos, crate::attrs::Attrs) {
        let mut prev_was_wide = false;

        let first_cell = &self.cells[usize::from(start)];
        let prev_first_cell = &prev.cells[usize::from(start)];
        if wrapping
            && !prev_wrapping
            && first_cell == prev_first_cell
            && prev_pos.row + 1 == row
            && prev_pos.col
                >= self.cols() - u16::from(prev_first_cell.is_wide())
        {
            let first_cell_attrs = first_cell.attrs();
            if &prev_attrs != first_cell_attrs {
                first_cell_attrs
                    .write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *first_cell_attrs;
            }
            let mut cell_contents = prev_first_cell.contents();
            let need_erase = if cell_contents.is_empty() {
                cell_contents = " ".to_string();
                true
            } else {
                false
            };
            contents.extend(cell_contents.as_bytes());
            crate::term::Backspace.write_buf(contents);
            if prev_first_cell.is_wide() {
                crate::term::Backspace.write_buf(contents);
            }
            if need_erase {
                crate::term::EraseChar::new(1).write_buf(contents);
            }
            prev_pos = crate::grid::Pos { row, col: 0 };
        }

        let mut erase: Option<(u16, &crate::attrs::Attrs)> = None;
        for (col, (cell, prev_cell)) in self
            .cells()
            .zip(prev.cells())
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            let pos = crate::grid::Pos { row, col };

            if let Some((prev_col, attrs)) = erase {
                if cell.has_contents() || cell.attrs() != attrs {
                    let new_pos = crate::grid::Pos { row, col: prev_col };
                    if wrapping
                        && prev_pos.row + 1 == new_pos.row
                        && prev_pos.col >= self.cols()
                    {
                        if new_pos.col > 0 {
                            contents.extend(
                                " ".repeat(usize::from(new_pos.col))
                                    .as_bytes(),
                            );
                        } else {
                            contents.extend(b" ");
                            crate::term::Backspace.write_buf(contents);
                        }
                    } else {
                        crate::term::MoveFromTo::new(prev_pos, new_pos)
                            .write_buf(contents);
                    }
                    prev_pos = new_pos;
                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }
                    crate::term::EraseChar::new(pos.col - prev_col)
                        .write_buf(contents);
                    erase = None;
                }
            }

            if cell != prev_cell {
                let attrs = cell.attrs();
                if cell.has_contents() {
                    if pos != prev_pos {
                        if !wrapping
                            || prev_pos.row + 1 != pos.row
                            || prev_pos.col
                                < self.cols() - u16::from(cell.is_wide())
                            || pos.col != 0
                        {
                            crate::term::MoveFromTo::new(prev_pos, pos)
                                .write_buf(contents);
                        }
                        prev_pos = pos;
                    }

                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }

                    prev_pos.col += if cell.is_wide() { 2 } else { 1 };
                    contents.extend(cell.contents().as_bytes());
                } else if erase.is_none() {
                    erase = Some((pos.col, attrs));
                }
            }
        }
        if let Some((prev_col, attrs)) = erase {
            let new_pos = crate::grid::Pos { row, col: prev_col };
            if wrapping
                && prev_pos.row + 1 == new_pos.row
                && prev_pos.col >= self.cols()
            {
                if new_pos.col > 0 {
                    contents.extend(
                        " ".repeat(usize::from(new_pos.col)).as_bytes(),
                    );
                } else {
                    contents.extend(b" ");
                    crate::term::Backspace.write_buf(contents);
                }
            } else {
                crate::term::MoveFromTo::new(prev_pos, new_pos)
                    .write_buf(contents);
            }
            prev_pos = new_pos;
            if &prev_attrs != attrs {
                attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *attrs;
            }
            crate::term::ClearRowForward.write_buf(contents);
        }

        // if this row is going from wrapped to not wrapped, we need to erase
        // and redraw the last character to break wrapping. if this row is
        // wrapped, we need to redraw the last character without erasing it to
        // position the cursor after the end of the line correctly so that
        // drawing the next line can just start writing and be wrapped.
        if (!self.wrapped && prev.wrapped) || (!prev.wrapped && self.wrapped)
        {
            let end_pos = if self.cells[usize::from(self.cols() - 1)]
                .is_wide_continuation()
            {
                crate::grid::Pos {
                    row,
                    col: self.cols() - 2,
                }
            } else {
                crate::grid::Pos {
                    row,
                    col: self.cols() - 1,
                }
            };
            crate::term::MoveFromTo::new(prev_pos, end_pos)
                .write_buf(contents);
            prev_pos = end_pos;
            if !self.wrapped {
                crate::term::EraseChar::new(1).write_buf(contents);
            }
            let end_cell = &self.cells[usize::from(end_pos.col)];
            if end_cell.has_contents() {
                let attrs = end_cell.attrs();
                if &prev_attrs != attrs {
                    attrs.write_escape_code_diff(contents, &prev_attrs);
                    prev_attrs = *attrs;
                }
                contents.extend(end_cell.contents().as_bytes());
                prev_pos.col += if end_cell.is_wide() { 2 } else { 1 };
            }
        }

        (prev_pos, prev_attrs)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_resize_expand() {
        let mut row = Row::new(10);
        {
            let result = row.resize(12, crate::Cell::new());
            assert!(result.is_none());
        }
        assert_eq!(row.cols(), 12);
    }

    #[test]
    fn test_resize_shrink() {
        let mut row = row_with_contents(6, "foobar");
        {
            let Some(overflow) = row.resize(3, crate::Cell::new()) else {
                panic!("there should be overflow");
            };
            let overflow = overflow.collect::<Vec<_>>();
            assert_eq!(overflow, cell_vec("bar"));
        }
        assert_eq!(row.cols(), 3);
    }

    #[test]
    fn test_resize_shrink_empty() {
        let mut row = row_with_contents(6, "foo");
        {
            let result = row.resize(3, crate::Cell::new());
            assert!(result.is_none());
        }
        assert_eq!(row.cols(), 3);
    }

    #[test]
    fn test_handle_no_overflow() {
        let mut overflow = cell_vec("foo");
        let mut row = row_with_contents(6, "bar");
        row.handle_overflow(6, &mut overflow);
        assert!(overflow.is_empty(), "expected no overflow: {overflow:?}");
        assert!(!row.wrapped());
        let mut contents = String::new();
        row.write_contents(&mut contents, 0, row.cols(), false);
        assert_eq!(contents, "foobar");
    }

    #[test]
    fn test_handle_exact_overflow() {
        let mut overflow = cell_vec("foobar");
        let mut row = row_with_contents(6, "baz");
        row.handle_overflow(6, &mut overflow);
        assert_eq!(overflow.len(), 3);
        assert!(row.wrapped());
        let mut contents = String::new();
        row.write_contents(&mut contents, 0, row.cols(), false);
        assert_eq!(contents, "foobar");
    }

    #[test]
    fn test_handle_additional_overflow() {
        let mut overflow = cell_vec("foobar");
        let mut row = row_with_contents(6, "baz");
        row.handle_overflow(3, &mut overflow);
        assert_eq!(overflow.len(), 6);
        assert!(row.wrapped());
        let mut contents = String::new();
        row.write_contents(&mut contents, 0, row.cols(), false);
        assert_eq!(contents, "foo");
    }

    #[test]
    fn test_reclaim_whitespace() {
        let mut buffer = cell_vec("foo");
        let mut row = row_with_contents(6, "bar");
        row.reclaim_wrapped_cells(6, &mut buffer);
        assert_eq!(buffer, cell_vec("foobar"));
    }

    #[test]
    fn test_append_cells_avoid_overtake() {
        let mut buffer = cell_vec("bar");
        let mut row = row_with_contents(3, "foo");
        row.append_cells(6, &mut buffer);
        let mut contents = String::new();
        row.write_contents(&mut contents, 0, row.cols(), false);
        assert_eq!(contents, "foobar");
    }

    fn row_with_contents(len: u16, contents: &str) -> Row {
        assert!(
            usize::from(len) >= contents.chars().count(),
            "length must be larger than content"
        );
        let mut row = Row::new(len);
        for (i, c) in contents.chars().enumerate() {
            let i = u16::try_from(i).unwrap();
            row.get_mut(i)
                .unwrap()
                .set(c, crate::attrs::Attrs::default());
        }
        row
    }

    fn cell_vec(s: &str) -> Vec<crate::Cell> {
        s.chars()
            .map(|c| {
                let mut cell = crate::Cell::new();
                cell.set(c, crate::attrs::Attrs::default());
                cell
            })
            .collect()
    }
}
