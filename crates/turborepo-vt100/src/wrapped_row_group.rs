use std::{collections::VecDeque, ops::Range};

use itertools::Itertools;

use crate::Cell;

pub struct WrappedRowGroup {
    // exclusive
    end: usize,
    length: usize,
}

impl WrappedRowGroup {
    pub fn new(number_of_rows: usize) -> Self {
        Self {
            end: number_of_rows,
            length: 0,
        }
    }

    // Extends the group to include the previous row
    pub fn extend(&mut self) {
        self.length += 1;
        debug_assert!(self.end >= self.length, "cannot have negative start");
    }

    pub fn start(&self) -> usize {
        self.end - self.length
    }

    // Returns the index before the current start, none if there is no valid index
    pub fn previous_start(&self) -> Option<usize> {
        let start = self.start();
        (start != 0).then(|| start - 1)
    }

    // TODO rename this
    /// Rewraps the current group of rows
    pub fn flush(&mut self, len: u16, rows: &mut Vec<crate::row::Row>) {
        // starting point is the new endpoint
        let start = self.start();

        let mut current_index = start;
        let mut buffer = Vec::new();
        while current_index < self.end {
            let current_cols = rows[current_index].cols();
            debug_assert!(len > current_cols, "flush should only be called with a greater len than the current cols");
            let available_cells_current = usize::from(len - current_cols);

            let mut next_row_index = current_index + 1;
            // need to loop forward and keep taking
            while buffer.len() < available_cells_current
                && next_row_index < self.end
            {
                let next_row = &mut rows[next_row_index];
                next_row.reclaim_wrapped_cells(
                    u16::try_from(available_cells_current - buffer.len())
                        .expect("cols size is limited to u16"),
                    &mut buffer,
                );

                next_row_index += 1;
            }

            let current_row = &mut rows[current_index];
            current_row.append_cells(buffer.len(), &mut buffer);
            debug_assert!(
                current_row.cols() <= len,
                "current row should not have more cells than desired length"
            );

            current_index += 1;
        }

        for row in &mut rows[start..self.end] {
            if row.cols() < len {
                let removed_cells = row.resize(len, Cell::new());
                debug_assert!(
                    removed_cells.is_none(),
                    "no cells should be removed"
                );
            }
        }

        // Go backwards and:
        // If there is contents in the row, then pad to correct size
        // If no content, then cut the row if the previous row is wrapped (and mark that row as unwrapped)
        for (curr_index, prev_index) in
            (start..self.end).rev().tuple_windows()
        {
            if rows[curr_index].cols_with_content() == 0 {
                let prev_row = &mut rows[prev_index];
                debug_assert!(
                    prev_row.wrapped(),
                    "previous row should be wrapped"
                );
                prev_row.wrap(false);
                // we remove the current row as it has no contents and is no longer needed for wrapping
                rows.remove(curr_index);
            }
        }

        // reset
        self.end = start;
        self.length = 0;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_no_wrapping() {
        let mut rows = vec![
            row_with_contents(3, "foo"),
            row_with_contents(3, "bar"),
            row_with_contents(3, "bar"),
        ];
        let mut group = WrappedRowGroup::new(rows.len());
        group.extend();
        assert_eq!(group.start(), 2);
        group.flush(6, &mut rows);
        assert_eq!(group.start(), 2);
        group.extend();
        assert_eq!(group.start(), 1);
        group.flush(6, &mut rows);
        assert_eq!(group.start(), 1);
        group.extend();
        assert_eq!(group.start(), 0);
        group.flush(6, &mut rows);
    }

    fn row_with_contents(len: u16, contents: &str) -> crate::row::Row {
        assert!(
            usize::from(len) >= contents.chars().count(),
            "length must be larger than content"
        );
        let mut row = crate::row::Row::new(len);
        for (i, c) in contents.chars().enumerate() {
            let i = u16::try_from(i).unwrap();
            row.get_mut(i)
                .unwrap()
                .set(c, crate::attrs::Attrs::default());
        }
        row
    }
}
