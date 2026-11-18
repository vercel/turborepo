//! Escape decoding for string literal contents.

use std::borrow::Cow;

/// Which escape sequences a language supports.
///
/// Common escapes (`\n \r \t \\ \' \" \0`) and line continuations (`\` followed
/// by a newline, which is removed) are always recognized.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EscapeRules {
    /// `\xHH`: a byte value. Values >= 0x80 become the corresponding Unicode
    /// scalar (JS and Python `str` semantics).
    pub hex: bool,
    /// `\uHHHH`, with UTF-16 surrogate pair joining. Lone surrogates become
    /// U+FFFD.
    pub unicode4: bool,
    /// `\UHHHHHHHH` (Python, C).
    pub unicode8: bool,
    /// `\u{H...}` (JS, Rust).
    pub unicode_braced: bool,
    /// `\b \f \v` (JS, Python, C).
    pub control: bool,
    /// Octal escapes `\ooo` with 1-3 digits (Python, C). Without this, `\0` is
    /// NUL.
    pub octal: bool,
    /// What to do with an unknown escape `\q`: keep the backslash (`\q`,
    /// Python) or drop it (`q`, JS). Rust treats unknown escapes as errors;
    /// lenient decoding keeps the backslash.
    pub unknown_keeps_backslash: bool,
}

impl EscapeRules {
    /// JavaScript / TypeScript string literals.
    pub const JS: EscapeRules = EscapeRules {
        hex: true,
        unicode4: true,
        unicode8: false,
        unicode_braced: true,
        control: true,
        octal: false,
        unknown_keeps_backslash: false,
    };

    /// Python non-raw string literals.
    pub const PYTHON: EscapeRules = EscapeRules {
        hex: true,
        unicode4: true,
        unicode8: true,
        unicode_braced: false,
        control: true,
        octal: true,
        unknown_keeps_backslash: true,
    };

    /// Rust string literals.
    pub const RUST: EscapeRules = EscapeRules {
        hex: true,
        unicode4: false,
        unicode8: false,
        unicode_braced: true,
        control: false,
        octal: false,
        unknown_keeps_backslash: true,
    };
}

/// Why decoding failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DecodeError {
    /// The contents (after decoding) are not valid UTF-8. The payload is the
    /// byte offset of the first invalid byte in the raw contents.
    InvalidUtf8(usize),
    /// A malformed escape such as `\x4` or `\u{110000}`. The payload is the
    /// byte offset of the backslash.
    BadEscape(usize),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::InvalidUtf8(at) => write!(f, "invalid UTF-8 at byte {at}"),
            DecodeError::BadEscape(at) => write!(f, "malformed escape at byte {at}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes the contents of a string literal (delimiters excluded).
///
/// Returns `Cow::Borrowed` when `raw` contains no backslash, which is the
/// common case for specifiers, so no allocation happens. Pass [`EscapeRules`]
/// matching the source language. For raw strings, call [`decode_raw`] instead.
pub fn decode<'a>(raw: &'a [u8], rules: EscapeRules) -> Result<Cow<'a, str>, DecodeError> {
    let Some(first) = memchr::memchr(b'\\', raw) else {
        return decode_raw(raw);
    };
    let mut out = String::with_capacity(raw.len());
    push_utf8(&mut out, &raw[..first], 0)?;
    let mut i = first;
    while i < raw.len() {
        if raw[i] != b'\\' {
            let next = memchr::memchr(b'\\', &raw[i..]).map_or(raw.len(), |n| i + n);
            push_utf8(&mut out, &raw[i..next], i)?;
            i = next;
            continue;
        }
        let esc_at = i;
        let Some(&e) = raw.get(i + 1) else {
            // A trailing lone backslash; keep it.
            out.push('\\');
            break;
        };
        i += 2;
        match e {
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'\\' => out.push('\\'),
            b'\'' => out.push('\''),
            b'"' => out.push('"'),
            b'\n' => {}
            b'\r' => {
                if raw.get(i) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'b' if rules.control => out.push('\u{8}'),
            b'f' if rules.control => out.push('\u{c}'),
            b'v' if rules.control => out.push('\u{b}'),
            b'0'..=b'7' if rules.octal => {
                let mut v = u32::from(e - b'0');
                let mut n = 1;
                while n < 3 {
                    match raw.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            i += 1;
                            n += 1;
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(v).ok_or(DecodeError::BadEscape(esc_at))?);
            }
            b'0' => out.push('\0'),
            b'x' if rules.hex => {
                let v = hex_digits(raw, i, 2).ok_or(DecodeError::BadEscape(esc_at))?;
                i += 2;
                out.push(char::from_u32(v).ok_or(DecodeError::BadEscape(esc_at))?);
            }
            b'u' if rules.unicode_braced && raw.get(i) == Some(&b'{') => {
                let close = memchr::memchr(b'}', &raw[i..])
                    .map(|n| i + n)
                    .ok_or(DecodeError::BadEscape(esc_at))?;
                let digits = &raw[i + 1..close];
                if digits.is_empty() || digits.len() > 6 {
                    return Err(DecodeError::BadEscape(esc_at));
                }
                let v =
                    hex_digits(raw, i + 1, digits.len()).ok_or(DecodeError::BadEscape(esc_at))?;
                i = close + 1;
                out.push(char::from_u32(v).ok_or(DecodeError::BadEscape(esc_at))?);
            }
            b'u' if rules.unicode4 => {
                let hi = hex_digits(raw, i, 4).ok_or(DecodeError::BadEscape(esc_at))?;
                i += 4;
                if (0xD800..0xDC00).contains(&hi) {
                    // Try to join with a following `\uDC00..\uDFFF`.
                    if raw.get(i) == Some(&b'\\')
                        && raw.get(i + 1) == Some(&b'u')
                        && let Some(lo) = hex_digits(raw, i + 2, 4)
                        && (0xDC00..0xE000).contains(&lo)
                    {
                        i += 6;
                        let c = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                        out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                        continue;
                    }
                    out.push('\u{FFFD}');
                } else {
                    out.push(char::from_u32(hi).unwrap_or('\u{FFFD}'));
                }
            }
            b'U' if rules.unicode8 => {
                let v = hex_digits(raw, i, 8).ok_or(DecodeError::BadEscape(esc_at))?;
                i += 8;
                out.push(char::from_u32(v).ok_or(DecodeError::BadEscape(esc_at))?);
            }
            _ => {
                // Unknown escape. Re-decode the escaped character as UTF-8 so multi-byte
                // characters survive.
                if rules.unknown_keeps_backslash {
                    out.push('\\');
                }
                let end = utf8_char_end(raw, i - 1);
                push_utf8(&mut out, &raw[i - 1..end], i - 1)?;
                i = end;
            }
        }
    }
    Ok(Cow::Owned(out))
}

/// Validates raw string contents as UTF-8 without interpreting escapes. Always
/// borrows.
pub fn decode_raw(raw: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
    std::str::from_utf8(raw)
        .map(Cow::Borrowed)
        .map_err(|e| DecodeError::InvalidUtf8(e.valid_up_to()))
}

fn push_utf8(out: &mut String, bytes: &[u8], offset: usize) -> Result<(), DecodeError> {
    let s = std::str::from_utf8(bytes)
        .map_err(|e| DecodeError::InvalidUtf8(offset + e.valid_up_to()))?;
    out.push_str(s);
    Ok(())
}

/// Parses exactly `n` hex digits at `raw[at..]`.
fn hex_digits(raw: &[u8], at: usize, n: usize) -> Option<u32> {
    let digits = raw.get(at..at.checked_add(n)?)?;
    let mut v: u32 = 0;
    for &d in digits {
        v = v.checked_mul(16)? + char::from(d).to_digit(16)?;
    }
    Some(v)
}

/// End offset of the UTF-8 sequence starting at `at` (best effort; clamps to
/// `raw.len()`).
fn utf8_char_end(raw: &[u8], at: usize) -> usize {
    let len = match raw[at] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    };
    (at + len).min(raw.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js(s: &[u8]) -> Result<Cow<'_, str>, DecodeError> {
        decode(s, EscapeRules::JS)
    }

    #[test]
    fn borrows_without_escapes() {
        let c = js(b"./foo/bar.js").unwrap();
        assert!(matches!(c, Cow::Borrowed("./foo/bar.js")));
        assert!(matches!(js("héllo".as_bytes()).unwrap(), Cow::Borrowed(_)));
        assert!(matches!(
            decode_raw(b"a\\b").unwrap(),
            Cow::Borrowed("a\\b")
        ));
    }

    #[test]
    fn common_escapes() {
        let c = js(br#"a\nb\tc\\d\'e\"f\0"#).unwrap();
        assert!(matches!(c, Cow::Owned(_)));
        assert_eq!(c, "a\nb\tc\\d'e\"f\0");
        assert_eq!(js(br"\b\f\v").unwrap(), "\u{8}\u{c}\u{b}");
    }

    #[test]
    fn line_continuations_removed() {
        assert_eq!(js(b"a\\\nb").unwrap(), "ab");
        assert_eq!(js(b"a\\\r\nb").unwrap(), "ab");
        assert_eq!(js(b"a\\\rb").unwrap(), "ab");
    }

    #[test]
    fn hex_and_unicode() {
        assert_eq!(js(br"\x41\x7a").unwrap(), "Az");
        assert_eq!(js(br"\xe9").unwrap(), "é");
        assert_eq!(js(br"\u0041").unwrap(), "A");
        assert_eq!(js(br"\u{1F600}").unwrap(), "😀");
        assert_eq!(js(br"\uD83D\uDE00").unwrap(), "😀");
        assert_eq!(js(br"\uD83Dx").unwrap(), "\u{FFFD}x");
        assert_eq!(js(br"\uDE00").unwrap(), "\u{FFFD}");
        assert_eq!(decode(br"\U0001F600", EscapeRules::PYTHON).unwrap(), "😀");
    }

    #[test]
    fn malformed_escapes() {
        assert_eq!(js(br"\x4"), Err(DecodeError::BadEscape(0)));
        assert_eq!(js(br"ab\xZZ"), Err(DecodeError::BadEscape(2)));
        assert_eq!(js(br"\u12"), Err(DecodeError::BadEscape(0)));
        assert_eq!(js(br"\u{110000}"), Err(DecodeError::BadEscape(0)));
        assert_eq!(js(br"\u{}"), Err(DecodeError::BadEscape(0)));
        assert_eq!(js(br"\u{12"), Err(DecodeError::BadEscape(0)));
        assert_eq!(js(br"\u{1234567}"), Err(DecodeError::BadEscape(0)));
    }

    #[test]
    fn unknown_escapes() {
        assert_eq!(js(r"\q\é".as_bytes()).unwrap(), "qé");
        assert_eq!(decode(br"\q\d", EscapeRules::PYTHON).unwrap(), r"\q\d");
        assert_eq!(decode(br"\b", EscapeRules::RUST).unwrap(), r"\b");
        // Unicode escapes are not JS-style in Python or Rust.
        assert_eq!(
            decode(br"\u{41}", EscapeRules::PYTHON),
            Err(DecodeError::BadEscape(0))
        );
        assert_eq!(decode(br"\u{41}", EscapeRules::RUST).unwrap(), "A");
    }

    #[test]
    fn octal() {
        let py = EscapeRules::PYTHON;
        assert_eq!(decode(br"\101\0\12", py).unwrap(), "A\0\n");
        assert_eq!(decode(br"\1018", py).unwrap(), "A8");
        // JS: `\0` is NUL, other digits are unknown escapes (dropped backslash).
        assert_eq!(js(br"\0\7").unwrap(), "\u{0}7");
    }

    #[test]
    fn trailing_backslash_kept() {
        assert_eq!(js(b"abc\\").unwrap(), "abc\\");
    }

    #[test]
    fn invalid_utf8() {
        assert_eq!(js(b"ab\xff"), Err(DecodeError::InvalidUtf8(2)));
        assert_eq!(js(b"\\n\xff"), Err(DecodeError::InvalidUtf8(2)));
        assert_eq!(js(b"\\\xff"), Err(DecodeError::InvalidUtf8(1)));
    }

    #[test]
    fn never_panics() {
        let samples: [&[u8]; 9] = [
            b"\\",
            b"\\u",
            b"\\u{",
            b"\\x",
            b"\\U",
            b"\\uD800\\u",
            b"\\uD800\\uZZZZ",
            b"\\\xf0",
            b"\\\xf0\x9f",
        ];
        for s in samples {
            for rules in [EscapeRules::JS, EscapeRules::PYTHON, EscapeRules::RUST] {
                let _ = decode(s, rules);
            }
        }
    }
}
