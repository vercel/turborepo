//! A Go tokenizer that is exact about code versus comments, interpreted
//! strings, raw strings and rune literals, and produces spans only (no
//! allocation per token).
//!
//! Keywords are identifiers; the recognizer compares their text. Comments are
//! tokens because directives (`//go:embed`, `//go:build`) and the cgo preamble
//! live in them. Newlines are not tokens; callers that need them (comment
//! grouping) look at the source between spans.

use turborepo_trace_core::lexkit::{Cursor, LexError, Span, skip};

/// Token kinds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An identifier or keyword (Unicode letters are treated as identifier
    /// bytes).
    Ident,
    /// `"interpreted"` string.
    Str,
    /// `` `raw` `` string.
    RawStr,
    /// `'r'` rune literal.
    Rune,
    /// A number literal.
    Number,
    /// Any other single byte (`(`, `.`, `+`, ...).
    Punct,
    /// `// ...` up to (not including) the newline.
    LineComment,
    /// `/* ... */`.
    BlockComment,
}

/// One token.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    /// Kind.
    pub kind: Kind,
    /// Byte range in the source.
    pub span: Span,
}

impl Tok {
    /// The token's bytes.
    pub fn text<'s>(&self, src: &'s [u8]) -> &'s [u8] {
        &src[self.span.range()]
    }

    /// Whether this is the punctuation byte `b`.
    pub fn is_punct(&self, src: &[u8], b: u8) -> bool {
        self.kind == Kind::Punct && src.get(self.span.start as usize) == Some(&b)
    }

    /// Whether this is the identifier (or keyword) `word`.
    pub fn is_ident(&self, src: &[u8], word: &[u8]) -> bool {
        self.kind == Kind::Ident && self.text(src) == word
    }

    /// Whether this is a comment.
    pub fn is_comment(&self) -> bool {
        matches!(self.kind, Kind::LineComment | Kind::BlockComment)
    }

    /// Whether this is a string literal (interpreted or raw).
    pub fn is_string(&self) -> bool {
        matches!(self.kind, Kind::Str | Kind::RawStr)
    }
}

/// Whether `b` can continue an identifier. Bytes >= 0x80 are parts of UTF-8
/// encoded letters.
#[inline]
pub fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The tokenizer. Yields `Err` for unterminated literals and comments, then
/// continues after them, so it never stops early and never panics.
pub struct Lexer<'s> {
    c: Cursor<'s>,
}

impl<'s> Lexer<'s> {
    /// A lexer over `src`.
    pub fn new(src: &'s [u8]) -> Self {
        Lexer {
            c: Cursor::new(src),
        }
    }

    /// The current byte offset.
    pub fn pos(&self) -> usize {
        self.c.pos()
    }

    fn tok(&self, kind: Kind, start: usize) -> Tok {
        Tok {
            kind,
            span: self.c.span_from(start),
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Result<Tok, LexError>;

    fn next(&mut self) -> Option<Self::Item> {
        let c = &mut self.c;
        c.skip_ascii_whitespace();
        let start = c.pos();
        let b = c.peek()?;
        Some(match b {
            b'/' if c.peek_at(1) == Some(b'/') => {
                c.skip_until(b'\n');
                Ok(self.tok(Kind::LineComment, start))
            }
            b'/' if c.peek_at(1) == Some(b'*') => match skip::block_comment(c) {
                Some(Ok(_)) => Ok(self.tok(Kind::BlockComment, start)),
                Some(Err(e)) => Err(e),
                None => {
                    c.advance(1);
                    Ok(self.tok(Kind::Punct, start))
                }
            },
            b'"' => match skip::quoted(c, skip::QuoteOpts::single_line(b'"')) {
                Some(Ok(_)) => Ok(self.tok(Kind::Str, start)),
                Some(Err(e)) => Err(e),
                None => unreachable_punct(c, start),
            },
            b'`' => match skip::quoted(c, skip::QuoteOpts::raw(b'`')) {
                Some(Ok(_)) => Ok(self.tok(Kind::RawStr, start)),
                Some(Err(e)) => Err(e),
                None => unreachable_punct(c, start),
            },
            b'\'' => match skip::quoted(c, skip::QuoteOpts::single_line(b'\'')) {
                Some(Ok(_)) => Ok(self.tok(Kind::Rune, start)),
                Some(Err(e)) => Err(e),
                None => unreachable_punct(c, start),
            },
            b'0'..=b'9' => {
                skip_number(c);
                Ok(self.tok(Kind::Number, start))
            }
            b'.' if c.peek_at(1).is_some_and(|d| d.is_ascii_digit()) => {
                c.advance(1);
                skip_number(c);
                Ok(self.tok(Kind::Number, start))
            }
            b if is_ident_byte(b) => {
                c.skip_while(is_ident_byte);
                Ok(self.tok(Kind::Ident, start))
            }
            _ => {
                c.advance(1);
                Ok(self.tok(Kind::Punct, start))
            }
        })
    }
}

/// Never reached in practice (the byte was checked); keeps the lexer total.
fn unreachable_punct(c: &mut Cursor<'_>, start: usize) -> Result<Tok, LexError> {
    if c.pos() == start {
        c.advance(1);
    }
    Ok(Tok {
        kind: Kind::Punct,
        span: c.span_from(start),
    })
}

/// Skips the rest of a number literal: digits, letters, `_`, `.`, and a sign
/// right after an exponent marker (`1e+9`, `0x1p-2`).
fn skip_number(c: &mut Cursor<'_>) {
    loop {
        let Some(b) = c.peek() else { return };
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
            c.advance(1);
            if matches!(b, b'e' | b'E' | b'p' | b'P') && matches!(c.peek(), Some(b'+' | b'-')) {
                c.advance(1);
            }
        } else {
            return;
        }
    }
}

/// Tokenizes `src` completely. Lexer errors are returned separately; tokens
/// around them are kept.
pub fn tokenize(src: &[u8]) -> (Vec<Tok>, Vec<LexError>) {
    let mut toks = Vec::with_capacity(src.len() / 5);
    let mut errs = Vec::new();
    for t in Lexer::new(src) {
        match t {
            Ok(t) => toks.push(t),
            Err(e) => errs.push(e),
        }
    }
    (toks, errs)
}

/// Decodes a Go string literal token (with its delimiters) into its value.
/// `None` for malformed escapes or invalid UTF-8.
pub fn string_value<'s>(src: &'s [u8], t: &Tok) -> Option<std::borrow::Cow<'s, str>> {
    let text = t.text(src);
    if text.len() < 2 {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    match t.kind {
        // Go drops carriage returns from raw strings.
        Kind::RawStr if inner.contains(&b'\r') => {
            let v: Vec<u8> = inner.iter().copied().filter(|&b| b != b'\r').collect();
            String::from_utf8(v).ok().map(std::borrow::Cow::Owned)
        }
        Kind::RawStr => std::str::from_utf8(inner)
            .ok()
            .map(std::borrow::Cow::Borrowed),
        Kind::Str => turborepo_trace_core::lexkit::decode(inner, GO_ESCAPES).ok(),
        _ => None,
    }
}

/// Go's escapes in interpreted strings: `\a \b \f \n \r \t \v \\ \' \"`, octal
/// `\ooo`, `\xHH`, `\uHHHH` and `\UHHHHHHHH`.
pub const GO_ESCAPES: turborepo_trace_core::lexkit::EscapeRules =
    turborepo_trace_core::lexkit::EscapeRules {
        hex: true,
        unicode4: true,
        unicode8: true,
        unicode_braced: false,
        control: true,
        octal: true,
        unknown_keeps_backslash: true,
    };

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(Kind, String)> {
        tokenize(src.as_bytes())
            .0
            .iter()
            .map(|t| {
                (
                    t.kind,
                    String::from_utf8_lossy(t.text(src.as_bytes())).into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn literals_and_comments() {
        let k = kinds("a := \"x\\\"y\" + `r\n\"` + 'q' // c\n/* b */ 1.5e+3 .5 '\\''");
        let got: Vec<Kind> = k.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            got,
            [
                Kind::Ident,
                Kind::Punct,
                Kind::Punct,
                Kind::Str,
                Kind::Punct,
                Kind::RawStr,
                Kind::Punct,
                Kind::Rune,
                Kind::LineComment,
                Kind::BlockComment,
                Kind::Number,
                Kind::Number,
                Kind::Rune,
            ]
        );
        assert_eq!(k[10].1, "1.5e+3");
    }

    #[test]
    fn errors_resync() {
        let (toks, errs) = tokenize(b"a \"unterminated\nb `raw");
        assert_eq!(errs.len(), 2);
        assert!(
            toks.iter()
                .any(|t| t.text(b"a \"unterminated\nb `raw") == b"b")
        );
        let (_, errs) = tokenize(b"/* open");
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn decodes_strings() {
        let src = b"\"a\\tb\\u00e9\\101\" `x\r\ny`";
        let (toks, _) = tokenize(src);
        assert_eq!(string_value(src, &toks[0]).unwrap(), "a\tb\u{e9}A");
        assert_eq!(string_value(src, &toks[1]).unwrap(), "x\ny");
    }
}
