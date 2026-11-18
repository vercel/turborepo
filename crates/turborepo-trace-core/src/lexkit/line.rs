/// A 1-based line and column. Columns count bytes, not characters.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineCol {
    /// 1-based line number.
    pub line: u32,
    /// 1-based byte column within the line.
    pub col: u32,
}

/// Maps byte offsets to line/column positions.
///
/// Building the index is a single `memchr` pass over the source. Build it
/// lazily, only when positions are needed for output (uncertainty reports,
/// `extract`), never on the hot path.
#[derive(Clone, Debug)]
pub struct LineIndex {
    /// Byte offset of the start of each line. Always contains at least `0`.
    starts: Vec<u32>,
    len: u32,
}

impl LineIndex {
    /// Builds the index for `src`. Lines are separated by `\n`; a `\r` before
    /// it is treated as part of the previous line.
    pub fn new(src: &[u8]) -> Self {
        let mut starts = Vec::with_capacity(src.len() / 32 + 1);
        starts.push(0);
        starts.extend(
            memchr::memchr_iter(b'\n', src).map(|i| u32::try_from(i + 1).unwrap_or(u32::MAX)),
        );
        LineIndex {
            starts,
            len: u32::try_from(src.len()).unwrap_or(u32::MAX),
        }
    }

    /// Number of lines (a trailing newline starts a final empty line).
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Line and column of `offset`. Offsets past the end clamp to the end of
    /// input.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = offset.min(self.len);
        // `starts[0] == 0 <= offset`, so the partition point is at least 1.
        let line_idx = self.starts.partition_point(|&s| s <= offset) - 1;
        LineCol {
            line: u32::try_from(line_idx + 1).unwrap_or(u32::MAX),
            col: offset - self.starts[line_idx] + 1,
        }
    }

    /// Byte offset where 1-based `line` starts, if it exists.
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.starts.get((line as usize).checked_sub(1)?).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source() {
        let idx = LineIndex::new(b"");
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(idx.line_col(10), LineCol { line: 1, col: 1 });
    }

    #[test]
    fn multi_line() {
        let src = b"ab\ncd\n\nefg";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 4);
        assert_eq!(idx.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(idx.line_col(2), LineCol { line: 1, col: 3 }); // the '\n'
        assert_eq!(idx.line_col(3), LineCol { line: 2, col: 1 });
        assert_eq!(idx.line_col(6), LineCol { line: 3, col: 1 });
        assert_eq!(idx.line_col(9), LineCol { line: 4, col: 3 });
        assert_eq!(idx.line_col(10), LineCol { line: 4, col: 4 }); // EOF
        assert_eq!(idx.line_col(u32::MAX), LineCol { line: 4, col: 4 });
        assert_eq!(idx.line_start(2), Some(3));
        assert_eq!(idx.line_start(0), None);
        assert_eq!(idx.line_start(5), None);
    }

    #[test]
    fn trailing_newline_and_crlf() {
        let idx = LineIndex::new(b"a\r\nb\n");
        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.line_col(3), LineCol { line: 2, col: 1 });
        assert_eq!(idx.line_col(5), LineCol { line: 3, col: 1 });
    }
}
