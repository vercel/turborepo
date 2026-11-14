//! Comment skippers and quoted-string scanning.
//!
//! Every function here expects the cursor to be positioned at the start of the
//! construct. Functions returning `Option` return `None` without moving the
//! cursor when the construct is not present, so they can be chained in a lexer
//! loop. Errors carry a span and leave the cursor at a sensible recovery point
//! (end of line or end of input).

use crate::lexkit::{Cursor, Span};

/// Kind of a lexing error.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LexErrorKind {
    /// A string literal hit a newline (single-line strings) or the end of
    /// input.
    UnterminatedString,
    /// A block comment hit the end of input.
    UnterminatedComment,
}

/// A recoverable lexing error.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LexError {
    /// What went wrong.
    pub kind: LexErrorKind,
    /// From the start of the construct to the recovery point.
    pub span: Span,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            LexErrorKind::UnterminatedString => "unterminated string literal",
            LexErrorKind::UnterminatedComment => "unterminated block comment",
        };
        write!(f, "{what} at bytes {}..{}", self.span.start, self.span.end)
    }
}

impl std::error::Error for LexError {}

/// Skips a line comment introduced by `prefix`, up to but not including the
/// next `\n`.
///
/// Returns the comment span (prefix included), or `None` if the input does not
/// start with `prefix`.
pub fn line_comment(c: &mut Cursor<'_>, prefix: &[u8]) -> Option<Span> {
    if !c.starts_with(prefix) {
        return None;
    }
    let start = c.pos();
    c.advance(prefix.len());
    c.skip_until(b'\n');
    Some(c.span_from(start))
}

/// Skips a `//` comment. See [`line_comment`].
#[inline]
pub fn slash_comment(c: &mut Cursor<'_>) -> Option<Span> {
    line_comment(c, b"//")
}

/// Skips a `#` comment. See [`line_comment`].
#[inline]
pub fn hash_comment(c: &mut Cursor<'_>) -> Option<Span> {
    line_comment(c, b"#")
}

/// Skips a non-nesting `/* ... */` comment.
///
/// Returns `None` if the input does not start with `/*`. An unterminated
/// comment consumes the rest of the input and returns an error.
pub fn block_comment(c: &mut Cursor<'_>) -> Option<Result<Span, LexError>> {
    if !c.starts_with(b"/*") {
        return None;
    }
    let start = c.pos();
    c.advance(2);
    if c.skip_until_slice(b"*/") {
        c.advance(2);
        Some(Ok(c.span_from(start)))
    } else {
        Some(Err(LexError {
            kind: LexErrorKind::UnterminatedComment,
            span: c.span_from(start),
        }))
    }
}

/// Skips a nesting `/* ... /* ... */ ... */` comment (Rust, Swift, ...).
///
/// Returns `None` if the input does not start with `/*`.
pub fn nested_block_comment(c: &mut Cursor<'_>) -> Option<Result<Span, LexError>> {
    if !c.starts_with(b"/*") {
        return None;
    }
    let start = c.pos();
    c.advance(2);
    let mut depth = 1usize;
    while c.skip_until_any(b"/*") {
        if c.eat_slice(b"/*") {
            depth += 1;
        } else if c.eat_slice(b"*/") {
            depth -= 1;
            if depth == 0 {
                return Some(Ok(c.span_from(start)));
            }
        } else {
            c.advance(1);
        }
    }
    Some(Err(LexError {
        kind: LexErrorKind::UnterminatedComment,
        span: c.span_from(start),
    }))
}

/// Skips either a `//` or a `/* */` comment (C, JS, Go, ...).
pub fn c_comment(c: &mut Cursor<'_>) -> Option<Result<Span, LexError>> {
    slash_comment(c).map(Ok).or_else(|| block_comment(c))
}

/// Options for [`quoted`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct QuoteOpts {
    /// The delimiter byte, used for both opening and closing.
    pub quote: u8,
    /// Whether raw newlines may appear inside the literal.
    pub multiline: bool,
    /// Whether `\` escapes the next byte. When `false` the literal is raw.
    pub escapes: bool,
}

impl QuoteOpts {
    /// A single-line string with backslash escapes, like JS `'...'` or `"..."`.
    pub const fn single_line(quote: u8) -> Self {
        QuoteOpts {
            quote,
            multiline: false,
            escapes: true,
        }
    }

    /// A string that may span lines, with backslash escapes, like a JS template
    /// literal (without `${}` handling).
    pub const fn multiline(quote: u8) -> Self {
        QuoteOpts {
            quote,
            multiline: true,
            escapes: true,
        }
    }

    /// A raw string without escapes that may span lines, like a Go backtick
    /// string.
    pub const fn raw(quote: u8) -> Self {
        QuoteOpts {
            quote,
            multiline: true,
            escapes: false,
        }
    }
}

/// A scanned string literal.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StrLit {
    /// The whole literal, delimiters included.
    pub span: Span,
    /// The contents between the delimiters, escapes not decoded.
    pub content: Span,
    /// Whether the contents contain at least one backslash escape. When
    /// `false`, the contents can be borrowed as-is (see
    /// [`crate::lexkit::decode()`]).
    pub has_escapes: bool,
}

/// Scans a quoted string literal starting at the opening delimiter.
///
/// Returns `None` if the current byte is not `opts.quote`. On success the
/// cursor is just past the closing delimiter. An escaped newline (`\` followed
/// by `\n` or `\r\n`) is a line continuation and is allowed in single-line
/// strings. On error the cursor stops at the offending newline (single-line
/// strings) or at the end of input.
pub fn quoted(c: &mut Cursor<'_>, opts: QuoteOpts) -> Option<Result<StrLit, LexError>> {
    if c.peek() != Some(opts.quote) {
        return None;
    }
    let start = c.pos();
    c.advance(1);
    let content_start = c.pos();
    let mut has_escapes = false;
    let set_buf;
    let set: &[u8] = match (opts.escapes, opts.multiline) {
        (true, false) => {
            set_buf = [opts.quote, b'\\', b'\n'];
            &set_buf
        }
        (true, true) => {
            set_buf = [opts.quote, b'\\', opts.quote];
            &set_buf[..2]
        }
        (false, false) => {
            set_buf = [opts.quote, b'\n', opts.quote];
            &set_buf[..2]
        }
        (false, true) => {
            set_buf = [opts.quote, opts.quote, opts.quote];
            &set_buf[..1]
        }
    };
    loop {
        if !c.skip_until_any(set) {
            return Some(Err(LexError {
                kind: LexErrorKind::UnterminatedString,
                span: c.span_from(start),
            }));
        }
        match c.peek() {
            Some(b) if b == opts.quote => {
                let content = c.span_from(content_start);
                c.advance(1);
                return Some(Ok(StrLit {
                    span: c.span_from(start),
                    content,
                    has_escapes,
                }));
            }
            Some(b'\\') => {
                has_escapes = true;
                c.advance(1);
                if c.peek() == Some(b'\r') && c.peek_at(1) == Some(b'\n') {
                    c.advance(2);
                } else {
                    c.advance(1);
                }
            }
            _ => {
                // A raw newline in a single-line string.
                return Some(Err(LexError {
                    kind: LexErrorKind::UnterminatedString,
                    span: c.span_from(start),
                }));
            }
        }
    }
}

/// Scans a single-line `'...'` or `"..."` string with backslash escapes.
///
/// Returns `None` if the current byte is neither quote.
pub fn string(c: &mut Cursor<'_>) -> Option<Result<StrLit, LexError>> {
    match c.peek()? {
        q @ (b'\'' | b'"') => quoted(c, QuoteOpts::single_line(q)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(src: &[u8], span: Span) -> &str {
        std::str::from_utf8(span.slice(src).unwrap()).unwrap()
    }

    #[test]
    fn line_comments() {
        let src = b"// hello\nx";
        let mut c = Cursor::new(src);
        let span = slash_comment(&mut c).unwrap();
        assert_eq!(text(src, span), "// hello");
        assert_eq!(c.peek(), Some(b'\n'));

        let mut c = Cursor::new(b"x // no");
        assert_eq!(slash_comment(&mut c), None);
        assert_eq!(c.pos(), 0);

        let src = b"# python comment";
        let mut c = Cursor::new(src);
        let span = hash_comment(&mut c).unwrap();
        assert_eq!(text(src, span), "# python comment");
        assert!(c.is_eof());

        let src = b"-- sql\n";
        let mut c = Cursor::new(src);
        assert_eq!(text(src, line_comment(&mut c, b"--").unwrap()), "-- sql");
    }

    #[test]
    fn block_comments() {
        let src = b"/* a * / b */rest";
        let mut c = Cursor::new(src);
        let span = block_comment(&mut c).unwrap().unwrap();
        assert_eq!(text(src, span), "/* a * / b */");
        assert_eq!(c.rest(), b"rest");

        // Not nesting: the first `*/` ends the comment.
        let mut c = Cursor::new(b"/* /* */ x */");
        block_comment(&mut c).unwrap().unwrap();
        assert_eq!(c.rest(), b" x */");

        let mut c = Cursor::new(b"/* never closed");
        let err = block_comment(&mut c).unwrap().unwrap_err();
        assert_eq!(err.kind, LexErrorKind::UnterminatedComment);
        assert_eq!(err.span, Span::new(0, 15));
        assert!(c.is_eof());

        let mut c = Cursor::new(b"/*/");
        assert!(block_comment(&mut c).unwrap().is_err());

        let mut c = Cursor::new(b"/**/x");
        block_comment(&mut c).unwrap().unwrap();
        assert_eq!(c.rest(), b"x");

        let mut c = Cursor::new(b"/ *");
        assert!(block_comment(&mut c).is_none());
    }

    #[test]
    fn nested_comments() {
        let src = b"/* a /* b */ c */rest";
        let mut c = Cursor::new(src);
        let span = nested_block_comment(&mut c).unwrap().unwrap();
        assert_eq!(text(src, span), "/* a /* b */ c */");
        assert_eq!(c.rest(), b"rest");

        let mut c = Cursor::new(b"/* /* */");
        assert!(nested_block_comment(&mut c).unwrap().is_err());
        assert!(c.is_eof());

        // `/*/` does not close and `*/*` handles overlap correctly.
        let mut c = Cursor::new(b"/*/ x */y");
        nested_block_comment(&mut c).unwrap().unwrap();
        assert_eq!(c.rest(), b"y");

        let mut c = Cursor::new(b"/* **/ok");
        nested_block_comment(&mut c).unwrap().unwrap();
        assert_eq!(c.rest(), b"ok");
    }

    #[test]
    fn c_comment_dispatch() {
        let mut c = Cursor::new(b"// x\n/* y */z");
        c_comment(&mut c).unwrap().unwrap();
        c.skip_ascii_whitespace();
        c_comment(&mut c).unwrap().unwrap();
        assert_eq!(c.rest(), b"z");
        assert!(c_comment(&mut c).is_none());
    }

    #[test]
    fn simple_strings() {
        let src = br#""hello" 'x'"#;
        let mut c = Cursor::new(src);
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(text(src, s.span), "\"hello\"");
        assert_eq!(text(src, s.content), "hello");
        assert!(!s.has_escapes);
        c.skip_ascii_whitespace();
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(text(src, s.content), "x");
        assert!(c.is_eof());
        assert!(string(&mut c).is_none());
    }

    #[test]
    fn escapes_in_strings() {
        let src = br#""a\"b\\" tail"#;
        let mut c = Cursor::new(src);
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(text(src, s.content), r#"a\"b\\"#);
        assert!(s.has_escapes);
        assert_eq!(c.rest(), b" tail");

        // The other quote kind needs no escaping.
        let src = br#"'say "hi"'"#;
        let mut c = Cursor::new(src);
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(text(src, s.content), r#"say "hi""#);
        assert!(!s.has_escapes);
    }

    #[test]
    fn line_continuation() {
        let src = b"'a\\\nb' x";
        let mut c = Cursor::new(src);
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(s.content, Span::new(1, 5));
        let src = b"'a\\\r\nb'";
        let mut c = Cursor::new(src);
        let s = string(&mut c).unwrap().unwrap();
        assert_eq!(s.content, Span::new(1, 6));
    }

    #[test]
    fn unterminated_strings() {
        let mut c = Cursor::new(b"'abc\nnext");
        let e = string(&mut c).unwrap().unwrap_err();
        assert_eq!(e.kind, LexErrorKind::UnterminatedString);
        assert_eq!(e.span, Span::new(0, 4));
        assert_eq!(c.peek(), Some(b'\n'));

        let mut c = Cursor::new(b"\"abc");
        assert!(string(&mut c).unwrap().is_err());
        assert!(c.is_eof());

        // Trailing backslash at EOF.
        let mut c = Cursor::new(b"\"abc\\");
        assert!(string(&mut c).unwrap().is_err());
        assert!(c.is_eof());
    }

    #[test]
    fn multiline_and_raw() {
        let src = b"`a\nb\\`c` x";
        let mut c = Cursor::new(src);
        let s = quoted(&mut c, QuoteOpts::multiline(b'`')).unwrap().unwrap();
        assert_eq!(text(src, s.content), "a\nb\\`c");
        assert!(s.has_escapes);

        let src = b"`raw\\` x";
        let mut c = Cursor::new(src);
        let s = quoted(&mut c, QuoteOpts::raw(b'`')).unwrap().unwrap();
        assert_eq!(text(src, s.content), "raw\\");
        assert!(!s.has_escapes);
        assert_eq!(c.rest(), b" x");

        let opts = QuoteOpts {
            quote: b'"',
            multiline: false,
            escapes: false,
        };
        let mut c = Cursor::new(b"\"a\\\"");
        let s = quoted(&mut c, opts).unwrap().unwrap();
        assert_eq!(s.content, Span::new(1, 3));
        let mut c = Cursor::new(b"\"a\nb\"");
        assert!(quoted(&mut c, opts).unwrap().is_err());
    }

    #[test]
    fn never_panics_on_garbage() {
        let inputs: [&[u8]; 8] = [
            b"",
            b"\"",
            b"'",
            b"/*",
            b"\\",
            b"\xff\xfe'\x00",
            b"'\\",
            b"/*/*/*",
        ];
        for input in inputs {
            for f in [string, |c: &mut Cursor<'_>| quoted(c, QuoteOpts::raw(b'`'))] {
                let mut c = Cursor::new(input);
                let _ = f(&mut c);
            }
            let mut c = Cursor::new(input);
            let _ = nested_block_comment(&mut c);
            let mut c = Cursor::new(input);
            let _ = c_comment(&mut c);
        }
    }
}
