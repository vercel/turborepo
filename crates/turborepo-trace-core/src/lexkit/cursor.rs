use crate::lexkit::Span;

/// A forward-only byte cursor over a source buffer.
///
/// All methods are bounds-safe: reading past the end yields `None` and
/// advancing clamps to the end of input.
#[derive(Clone, Debug)]
pub struct Cursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Creates a cursor at offset 0.
    #[inline]
    pub fn new(src: &'a [u8]) -> Self {
        Cursor { src, pos: 0 }
    }

    /// The full source buffer.
    #[inline]
    pub fn src(&self) -> &'a [u8] {
        self.src
    }

    /// Current byte offset.
    #[inline]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves to `pos`, clamped to the end of input.
    #[inline]
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos.min(self.src.len());
    }

    /// Whether the cursor is at the end of input.
    #[inline]
    pub fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// The unread remainder of the input.
    #[inline]
    pub fn rest(&self) -> &'a [u8] {
        &self.src[self.pos.min(self.src.len())..]
    }

    /// The current byte, if any.
    #[inline]
    pub fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    /// The byte `n` positions ahead of the current one, if any.
    #[inline]
    pub fn peek_at(&self, n: usize) -> Option<u8> {
        self.src.get(self.pos.checked_add(n)?).copied()
    }

    /// Consumes and returns the current byte.
    #[inline]
    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Advances by `n` bytes, clamped to the end of input.
    #[inline]
    pub fn advance(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.src.len());
    }

    /// Whether the remaining input starts with `prefix`.
    #[inline]
    pub fn starts_with(&self, prefix: &[u8]) -> bool {
        self.rest().starts_with(prefix)
    }

    /// Consumes `b` if it is the current byte.
    #[inline]
    pub fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consumes `prefix` if the remaining input starts with it.
    #[inline]
    pub fn eat_slice(&mut self, prefix: &[u8]) -> bool {
        if self.starts_with(prefix) {
            self.pos += prefix.len();
            true
        } else {
            false
        }
    }

    /// Advances to the next occurrence of `b`, leaving the cursor on it.
    ///
    /// Returns `false` and moves to the end of input when `b` does not occur.
    #[inline]
    pub fn skip_until(&mut self, b: u8) -> bool {
        match memchr::memchr(b, self.rest()) {
            Some(i) => {
                self.pos += i;
                true
            }
            None => {
                self.pos = self.src.len();
                false
            }
        }
    }

    /// Advances to the next byte contained in `set`, leaving the cursor on it.
    ///
    /// Uses `memchr`/`memchr2`/`memchr3` for sets of up to three bytes and a
    /// 256-entry lookup table otherwise. Returns `false` and moves to the
    /// end of input when no byte matches. An empty set moves to the end of
    /// input.
    pub fn skip_until_any(&mut self, set: &[u8]) -> bool {
        let rest = self.rest();
        let found = match *set {
            [] => None,
            [a] => memchr::memchr(a, rest),
            [a, b] => memchr::memchr2(a, b, rest),
            [a, b, c] => memchr::memchr3(a, b, c, rest),
            _ => {
                let mut table = [false; 256];
                for &b in set {
                    table[b as usize] = true;
                }
                rest.iter().position(|&b| table[b as usize])
            }
        };
        match found {
            Some(i) => {
                self.pos += i;
                true
            }
            None => {
                self.pos = self.src.len();
                false
            }
        }
    }

    /// Advances to the next occurrence of `needle` (via `memchr::memmem`),
    /// leaving the cursor at its start. Returns `false` and moves to the
    /// end of input when it does not occur.
    pub fn skip_until_slice(&mut self, needle: &[u8]) -> bool {
        match memchr::memmem::find(self.rest(), needle) {
            Some(i) => {
                self.pos += i;
                true
            }
            None => {
                self.pos = self.src.len();
                false
            }
        }
    }

    /// Advances while `pred` holds for the current byte. Returns the number of
    /// bytes skipped.
    #[inline]
    pub fn skip_while(&mut self, mut pred: impl FnMut(u8) -> bool) -> usize {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if !pred(b) {
                break;
            }
            self.pos += 1;
        }
        self.pos - start
    }

    /// Skips spaces, tabs, carriage returns, form feeds and newlines.
    #[inline]
    pub fn skip_ascii_whitespace(&mut self) -> usize {
        self.skip_while(|b| b.is_ascii_whitespace())
    }

    /// Skips spaces and tabs only (not newlines).
    #[inline]
    pub fn skip_inline_whitespace(&mut self) -> usize {
        self.skip_while(|b| b == b' ' || b == b'\t')
    }

    /// Skips to just past the next `\n`, or to the end of input.
    #[inline]
    pub fn skip_line(&mut self) {
        if self.skip_until(b'\n') {
            self.pos += 1;
        }
    }

    /// Span from `start` to the current position.
    #[inline]
    pub fn span_from(&self, start: usize) -> Span {
        Span::from_usize(start, self.pos)
    }

    /// The bytes covered by `span`, or an empty slice if out of bounds.
    #[inline]
    pub fn slice(&self, span: Span) -> &'a [u8] {
        span.slice(self.src).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_navigation() {
        let mut c = Cursor::new(b"abc");
        assert_eq!(c.peek(), Some(b'a'));
        assert_eq!(c.peek_at(2), Some(b'c'));
        assert_eq!(c.peek_at(3), None);
        assert_eq!(c.peek_at(usize::MAX), None);
        assert_eq!(c.bump(), Some(b'a'));
        assert!(c.eat(b'b'));
        assert!(!c.eat(b'x'));
        assert_eq!(c.rest(), b"c");
        c.advance(100);
        assert!(c.is_eof());
        assert_eq!(c.bump(), None);
        assert_eq!(c.rest(), b"");
        c.set_pos(1000);
        assert_eq!(c.pos(), 3);
    }

    #[test]
    fn eat_slice_and_starts_with() {
        let mut c = Cursor::new(b"import x");
        assert!(c.starts_with(b"imp"));
        assert!(!c.eat_slice(b"export"));
        assert!(c.eat_slice(b"import"));
        assert_eq!(c.skip_inline_whitespace(), 1);
        assert_eq!(c.peek(), Some(b'x'));
    }

    #[test]
    fn skip_until_variants() {
        let src = b"hello \"world\" // done\n";
        for set in [&b"\""[..], b"\"/", b"\"/\n", b"\"/\n'`"] {
            let mut c = Cursor::new(src);
            assert!(c.skip_until_any(set), "set {set:?}");
            assert_eq!(c.pos(), 6);
        }
        let mut c = Cursor::new(src);
        assert!(!c.skip_until_any(b"#"));
        assert!(c.is_eof());
        let mut c = Cursor::new(src);
        assert!(!c.skip_until_any(b""));
        assert!(c.is_eof());
        let mut c = Cursor::new(src);
        assert!(!c.skip_until_any(b"#$%^&"));
        assert!(c.is_eof());
    }

    #[test]
    fn skip_until_single_and_slice() {
        let mut c = Cursor::new(b"a\nb\nc");
        assert!(c.skip_until(b'\n'));
        assert_eq!(c.pos(), 1);
        c.skip_line();
        assert_eq!(c.pos(), 2);
        c.skip_line();
        c.skip_line();
        assert!(c.is_eof());

        let mut c = Cursor::new(b"xx require( yy");
        assert!(c.skip_until_slice(b"require"));
        assert_eq!(c.pos(), 3);
        assert!(!c.skip_until_slice(b"import"));
        assert!(c.is_eof());
    }

    #[test]
    fn skip_while_and_spans() {
        let mut c = Cursor::new(b"  \t\nfoo123 bar");
        c.skip_ascii_whitespace();
        let start = c.pos();
        c.skip_while(|b| b.is_ascii_alphanumeric());
        let span = c.span_from(start);
        assert_eq!(c.slice(span), b"foo123");
        assert_eq!(c.slice(Span::new(100, 200)), b"");
    }
}
