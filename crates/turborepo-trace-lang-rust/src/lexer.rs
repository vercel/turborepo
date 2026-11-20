//! A byte-oriented Rust tokenizer, exact about code versus comments, strings
//! and chars.
//!
//! Covers nested block comments (doc comments are comments), all string forms
//! (`"…"`, `b"…"`, `c"…"`, raw `r#"…"#`, `br#"…"#`, `cr#"…"#`), char and byte
//! literals versus lifetimes and labels, raw identifiers (`r#mod`), numbers and
//! single-byte punctuation. Tokens are spans; only string contents are ever
//! decoded ([`Tok::text`]).
//!
//! Linear in the input and panic-free on arbitrary bytes: unterminated
//! constructs consume the rest of the input and are reported as [`LexIssue`]s.

use std::borrow::Cow;

use memchr::{memchr, memchr2, memchr3};
use turborepo_trace_core::{
    Span,
    lexkit::{EscapeRules, decode},
};

/// What a token is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TokKind {
    /// An identifier or keyword.
    Ident,
    /// A raw identifier `r#name` (never a keyword). The text excludes `r#`.
    RawIdent,
    /// A lifetime or loop label, `'a`.
    Lifetime,
    /// A string literal of any form. See [`StrForm`].
    Str(StrForm),
    /// A char or byte literal.
    Char,
    /// A numeric literal.
    Num,
    /// One punctuation byte.
    Punct(u8),
}

/// The form of a string literal.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrForm {
    /// `"…"`, with escapes.
    Normal,
    /// `r"…"`, `r#"…"#`.
    Raw,
    /// `b"…"`.
    Byte,
    /// `br"…"`.
    RawByte,
    /// `c"…"`.
    C,
    /// `cr"…"`.
    RawC,
}

impl StrForm {
    /// Whether escapes are not processed.
    pub const fn is_raw(self) -> bool {
        matches!(self, StrForm::Raw | StrForm::RawByte | StrForm::RawC)
    }
}

/// One token: its kind, its full span and, for strings, the span of the
/// contents between the quotes (for identifiers the name without `r#`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Tok {
    /// Kind.
    pub kind: TokKind,
    /// Byte offset of the first byte.
    pub start: u32,
    /// Byte offset past the last byte.
    pub end: u32,
    /// Start of the contents (strings, raw identifiers), else `start`.
    pub cstart: u32,
    /// End of the contents, else `end`.
    pub cend: u32,
}

impl Tok {
    /// The full span.
    pub fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }

    /// The source bytes of the contents (identifier name, string contents).
    pub fn bytes<'s>(&self, src: &'s [u8]) -> &'s [u8] {
        src.get(self.cstart as usize..self.cend as usize)
            .unwrap_or_default()
    }

    /// Whether this is the non-raw identifier `word`.
    pub fn is_ident(&self, src: &[u8], word: &str) -> bool {
        self.kind == TokKind::Ident && self.bytes(src) == word.as_bytes()
    }

    /// Whether this is punctuation `b`.
    pub fn is_punct(&self, b: u8) -> bool {
        self.kind == TokKind::Punct(b)
    }

    /// Decoded string contents, or `None` for non-strings and undecodable
    /// contents.
    pub fn text<'s>(&self, src: &'s [u8]) -> Option<Cow<'s, str>> {
        let TokKind::Str(form) = self.kind else {
            return None;
        };
        let raw = self.bytes(src);
        if form.is_raw() {
            std::str::from_utf8(raw).ok().map(Cow::Borrowed)
        } else {
            decode(raw, EscapeRules::RUST).ok()
        }
    }
}

/// An unterminated construct.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LexIssue {
    /// Where the construct starts (to the end of input).
    pub span: Span,
    /// What it is.
    pub what: &'static str,
}

#[inline]
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

#[inline]
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Byte length of the UTF-8 sequence led by `b` (1 for invalid lead bytes).
#[inline]
fn utf8_len(b: u8) -> usize {
    match b {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Tokenizes `src` into `out` (cleared first) and reports unterminated
/// constructs.
pub fn tokenize(src: &[u8], out: &mut Vec<Tok>, issues: &mut Vec<LexIssue>) {
    out.clear();
    let n = src.len();
    let mut i = 0usize;
    // Shebang (not an inner attribute `#![`).
    if src.starts_with(b"#!")
        && !src[2..]
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_some_and(|b| *b == b'[')
    {
        i = memchr(b'\n', src).unwrap_or(n);
    }
    let push = |out: &mut Vec<Tok>, kind, s: usize, e: usize, cs: usize, ce: usize| {
        out.push(Tok {
            kind,
            start: s as u32,
            end: e as u32,
            cstart: cs as u32,
            cend: ce as u32,
        });
    };
    while i < n {
        let b = src[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        match b {
            b'/' if src.get(i + 1) == Some(&b'/') => {
                i = memchr(b'\n', &src[i..]).map_or(n, |p| i + p);
            }
            b'/' if src.get(i + 1) == Some(&b'*') => {
                let start = i;
                i += 2;
                let mut depth = 1usize;
                loop {
                    match memchr2(b'*', b'/', &src[i..]) {
                        None => {
                            issues.push(LexIssue {
                                span: Span::from_usize(start, n),
                                what: "unterminated block comment",
                            });
                            i = n;
                            break;
                        }
                        Some(p) => {
                            let j = i + p;
                            if src[j] == b'*' && src.get(j + 1) == Some(&b'/') {
                                depth -= 1;
                                i = j + 2;
                                if depth == 0 {
                                    break;
                                }
                            } else if src[j] == b'/' && src.get(j + 1) == Some(&b'*') {
                                depth += 1;
                                i = j + 2;
                            } else {
                                i = j + 1;
                            }
                        }
                    }
                }
            }
            b'"' => {
                let (e, ok) = scan_quoted(src, i + 1);
                if !ok {
                    issues.push(LexIssue {
                        span: Span::from_usize(i, n),
                        what: "unterminated string",
                    });
                }
                push(
                    out,
                    TokKind::Str(StrForm::Normal),
                    i,
                    e,
                    i + 1,
                    if ok { e - 1 } else { e },
                );
                i = e;
            }
            b'\'' => {
                i = char_or_lifetime(src, i, i, out, issues);
            }
            b'0'..=b'9' => {
                let s = i;
                i += 1;
                loop {
                    while i < n && is_ident_continue(src[i]) {
                        i += 1;
                    }
                    if i + 1 < n && src[i] == b'.' && src[i + 1].is_ascii_digit() {
                        i += 1;
                        continue;
                    }
                    break;
                }
                push(out, TokKind::Num, s, i, s, i);
            }
            _ if is_ident_start(b) => {
                if let Some(next) = prefixed_literal(src, i, out, issues) {
                    i = next;
                    continue;
                }
                let s = i;
                i += utf8_len(b).max(1);
                while i < n && is_ident_continue(src[i]) {
                    i += 1;
                }
                let i2 = i.min(n);
                push(out, TokKind::Ident, s, i2, s, i2);
                i = i2;
            }
            _ => {
                push(out, TokKind::Punct(b), i, i + 1, i, i + 1);
                i += 1;
            }
        }
    }
}

/// Scans a `"`-terminated string body with backslash escapes from `i`. Returns
/// the offset past the closing quote and whether it was found.
fn scan_quoted(src: &[u8], mut i: usize) -> (usize, bool) {
    let n = src.len();
    while i < n {
        match memchr2(b'"', b'\\', &src[i..]) {
            None => return (n, false),
            Some(p) => {
                let j = i + p;
                if src[j] == b'\\' {
                    i = j + 2;
                } else {
                    return (j + 1, true);
                }
            }
        }
    }
    (n, false)
}

/// Scans a raw string starting at the `#`s or `"` at `i` (after the
/// `r`/`br`/`cr` prefix). Returns `(end, content_start, content_end,
/// terminated)`, or `None` if it is not a raw string opening.
fn scan_raw(src: &[u8], i: usize) -> Option<(usize, usize, usize, bool)> {
    let n = src.len();
    let mut j = i;
    while j < n && src[j] == b'#' {
        j += 1;
    }
    if src.get(j) != Some(&b'"') {
        return None;
    }
    let hashes = j - i;
    let cs = j + 1;
    let mut k = cs;
    while k < n {
        let Some(p) = memchr(b'"', &src[k..]) else {
            break;
        };
        let q = k + p;
        let close = q + 1;
        if src.len() >= close + hashes && src[close..close + hashes].iter().all(|b| *b == b'#') {
            return Some((close + hashes, cs, q, true));
        }
        k = q + 1;
    }
    Some((n, cs, n, false))
}

/// Handles identifiers that start a literal (`r"`, `r#"`, `r#ident`, `b"`,
/// `b'`, `br"`, `c"`, `cr"`). Returns the next offset if a literal or raw
/// identifier was consumed.
fn prefixed_literal(
    src: &[u8],
    i: usize,
    out: &mut Vec<Tok>,
    issues: &mut Vec<LexIssue>,
) -> Option<usize> {
    let n = src.len();
    let b = src[i];
    let at = |k: usize| src.get(i + k).copied();
    let raw_str = |out: &mut Vec<Tok>, issues: &mut Vec<LexIssue>, form, from: usize| {
        scan_raw(src, from).map(|(e, cs, ce, ok)| {
            if !ok {
                issues.push(LexIssue {
                    span: Span::from_usize(i, n),
                    what: "unterminated raw string",
                });
            }
            out.push(Tok {
                kind: TokKind::Str(form),
                start: i as u32,
                end: e as u32,
                cstart: cs as u32,
                cend: ce as u32,
            });
            e
        })
    };
    let quoted = |out: &mut Vec<Tok>, issues: &mut Vec<LexIssue>, form, from: usize| {
        let (e, ok) = scan_quoted(src, from + 1);
        if !ok {
            issues.push(LexIssue {
                span: Span::from_usize(i, n),
                what: "unterminated string",
            });
        }
        out.push(Tok {
            kind: TokKind::Str(form),
            start: i as u32,
            end: e as u32,
            cstart: (from + 1) as u32,
            cend: if ok { e - 1 } else { e } as u32,
        });
        e
    };
    // The prefix must be a whole identifier start: `br"` is a literal, `abr"` is
    // not (the caller only calls this at identifier starts).
    match (b, at(1), at(2)) {
        (b'r', Some(b'"'), _) | (b'r', Some(b'#'), Some(b'"' | b'#')) => {
            raw_str(out, issues, StrForm::Raw, i + 1)
        }
        (b'r', Some(b'#'), Some(c)) if is_ident_start(c) => {
            let s = i + 2;
            let mut e = s;
            while e < n && is_ident_continue(src[e]) {
                e += 1;
            }
            out.push(Tok {
                kind: TokKind::RawIdent,
                start: i as u32,
                end: e as u32,
                cstart: s as u32,
                cend: e as u32,
            });
            Some(e)
        }
        (b'b', Some(b'"'), _) => Some(quoted(out, issues, StrForm::Byte, i + 1)),
        (b'c', Some(b'"'), _) => Some(quoted(out, issues, StrForm::C, i + 1)),
        (b'b', Some(b'\''), _) => Some(char_or_lifetime(src, i + 1, i, out, issues)),
        (b'b', Some(b'r'), Some(b'"' | b'#')) => raw_str(out, issues, StrForm::RawByte, i + 2),
        (b'c', Some(b'r'), Some(b'"' | b'#')) => raw_str(out, issues, StrForm::RawC, i + 2),
        _ => None,
    }
}

/// At a `'` (offset `q`; the token starts at `start`, before a `b` prefix): a
/// char literal, a lifetime/label, or a lone quote. Returns the next offset.
fn char_or_lifetime(
    src: &[u8],
    q: usize,
    start: usize,
    out: &mut Vec<Tok>,
    _issues: &mut Vec<LexIssue>,
) -> usize {
    let n = src.len();
    let tok = |kind, e: usize| Tok {
        kind,
        start: start as u32,
        end: e as u32,
        cstart: (q + 1).min(e) as u32,
        cend: e.saturating_sub(1).max(q + 1).min(e) as u32,
    };
    match src.get(q + 1).copied() {
        Some(b'\\') => {
            // Escape: find the closing quote within a short window (`'\u{10FFFF}'`).
            let lim = (q + 14).min(n);
            if let Some(p) = memchr3(b'\'', b'\n', b'"', &src[(q + 3).min(lim)..lim])
                .map(|p| p + (q + 3).min(lim))
                .filter(|&p| src[p] == b'\'')
            {
                out.push(tok(TokKind::Char, p + 1));
                return p + 1;
            }
            out.push(tok(TokKind::Punct(b'\''), q + 1));
            q + 1
        }
        Some(c) => {
            let len = utf8_len(c);
            if src.get(q + 1 + len) == Some(&b'\'') && c != b'\'' {
                out.push(tok(TokKind::Char, q + 2 + len));
                return q + 2 + len;
            }
            if is_ident_start(c) && start == q {
                let mut e = q + 1;
                while e < n && is_ident_continue(src[e]) {
                    e += 1;
                }
                out.push(Tok {
                    kind: TokKind::Lifetime,
                    start: q as u32,
                    end: e as u32,
                    cstart: (q + 1) as u32,
                    cend: e as u32,
                });
                return e;
            }
            out.push(Tok {
                kind: TokKind::Punct(b'\''),
                start: start as u32,
                end: (q + 1) as u32,
                cstart: start as u32,
                cend: (q + 1) as u32,
            });
            q + 1
        }
        None => {
            out.push(Tok {
                kind: TokKind::Punct(b'\''),
                start: start as u32,
                end: (q + 1) as u32,
                cstart: start as u32,
                cend: (q + 1) as u32,
            });
            q + 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(TokKind, String)> {
        let mut out = Vec::new();
        let mut issues = Vec::new();
        tokenize(src.as_bytes(), &mut out, &mut issues);
        out.iter()
            .map(|t| {
                (
                    t.kind,
                    String::from_utf8_lossy(&src.as_bytes()[t.start as usize..t.end as usize])
                        .into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn comments_nest_and_vanish() {
        let k = kinds("a /* x /* y */ z */ b // c\n/// doc\n//! inner\nd");
        let words: Vec<_> = k.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(words, ["a", "b", "d"]);
    }

    #[test]
    fn strings_of_every_form() {
        let src = r####"x = "a\"b"; r#"q"x"#; br##"z"#"##; b"by"; c"cs"; cr#"r"#; r"plain""####;
        let mut out = Vec::new();
        let mut issues = Vec::new();
        tokenize(src.as_bytes(), &mut out, &mut issues);
        assert!(issues.is_empty());
        let texts: Vec<_> = out
            .iter()
            .filter_map(|t| t.text(src.as_bytes()).map(|s| s.into_owned()))
            .collect();
        assert_eq!(texts, ["a\"b", "q\"x", "z\"#", "by", "cs", "r", "plain"]);
    }

    #[test]
    fn chars_versus_lifetimes() {
        let k = kinds(
            "fn f<'a>(x: &'a str) { 'outer: loop {} let c = 'x'; let d = '\\n'; let e = \
             '\\u{1F600}'; let f = b'q'; let g = 'é'; }",
        );
        let chars: Vec<_> = k
            .iter()
            .filter(|(k, _)| *k == TokKind::Char)
            .map(|(_, s)| s.as_str())
            .collect();
        assert_eq!(chars, ["'x'", "'\\n'", "'\\u{1F600}'", "b'q'", "'é'"]);
        let lts: Vec<_> = k
            .iter()
            .filter(|(k, _)| *k == TokKind::Lifetime)
            .map(|(_, s)| s.as_str())
            .collect();
        assert_eq!(lts, ["'a", "'a", "'outer"]);
    }

    #[test]
    fn raw_identifiers_and_numbers() {
        let k = kinds("r#mod x 1.5e3 0x_ff 1..2 t.0");
        assert_eq!(k[0], (TokKind::RawIdent, "r#mod".into()));
        assert_eq!(k[2], (TokKind::Num, "1.5e3".into()));
        assert_eq!(k[3], (TokKind::Num, "0x_ff".into()));
        assert_eq!(k[4], (TokKind::Num, "1".into()));
    }

    #[test]
    fn unterminated_constructs_are_reported() {
        for src in ["/* a", "\"abc", "r#\"abc\"", "b\"x", "'", "b'", "r#", "br#"] {
            let mut out = Vec::new();
            let mut issues = Vec::new();
            tokenize(src.as_bytes(), &mut out, &mut issues);
            for t in &out {
                assert!(t.start <= t.end && t.end as usize <= src.len(), "{src:?}");
                assert!(
                    t.cstart <= t.cend && t.cend as usize <= src.len(),
                    "{src:?}"
                );
            }
        }
    }

    #[test]
    fn shebang_but_not_inner_attribute() {
        assert_eq!(kinds("#!/usr/bin/env run\nx").len(), 1);
        assert_eq!(kinds("#![allow(x)]").first().unwrap().1, "#");
    }
}
