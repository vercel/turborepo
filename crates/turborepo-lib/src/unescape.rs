use std::{fmt, fmt::Display, ops::Deref};

use biome_deserialize::{Deserializable, DeserializableValue, DeserializationDiagnostic};
use thiserror::Error;

// We're using a newtype here because biome currently doesn't
// handle escapes and we can't override the String deserializer
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(transparent)]
pub struct UnescapedString(String);

impl Display for UnescapedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Deref for UnescapedString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Copy, Clone)]
enum EscapeState {
    Normal,
    // We just saw a `\`
    Escape,
    // We just saw a `\u` and now expect 4 hex digits
    Unicode,
}

#[derive(Debug, Error)]
enum Error {
    #[error("Invalid escape sequence: `{seq}`")]
    InvalidEscapeSequence { seq: String },
    #[error("Invalid unicode escape sequence ({reason}): `{seq}`")]
    InvalidUnicodeEscapeSequence { seq: String, reason: &'static str },
    #[error("Unexpected end of input, expected {expected}")]
    UnexpectedEndOfInput { expected: &'static str },
}

fn unescape_str(s: String) -> Result<String, Error> {
    let mut state = EscapeState::Normal;
    // Invariant: unicode_digits.len() < 4 at the end of the loop
    let mut unicode_digits = String::new();
    let mut out = String::new();
    for c in s.chars() {
        match (c, state) {
            ('\\' | '"' | '/', EscapeState::Escape) => {
                out.push(c);
                state = EscapeState::Normal;
            }
            ('b', EscapeState::Escape) => out.push('\x08'),
            ('f', EscapeState::Escape) => out.push('\x0c'),
            ('n', EscapeState::Escape) => out.push('\n'),
            ('r', EscapeState::Escape) => out.push('\r'),
            ('t', EscapeState::Escape) => out.push('\t'),
            ('u', EscapeState::Escape) => {
                unicode_digits = String::new();
                state = EscapeState::Unicode;
            }
            (c, EscapeState::Escape) => {
                return Err(Error::InvalidEscapeSequence {
                    seq: format!("\\{}", c),
                })
            }
            (c, EscapeState::Unicode) if c.is_ascii_hexdigit() => {
                unicode_digits.push(c);
            }
            (c, EscapeState::Unicode) => {
                return Err(Error::InvalidUnicodeEscapeSequence {
                    seq: format!("\\u{}{}", unicode_digits, c),
                    reason: "not a hex digit",
                })
            }
            ('\\', EscapeState::Normal) => state = EscapeState::Escape,
            (c, EscapeState::Normal) => out.push(c),
        }

        if unicode_digits.len() == 4 {
            let codepoint = u32::from_str_radix(&unicode_digits, 16).map_err(|_| {
                Error::InvalidUnicodeEscapeSequence {
                    seq: format!("\\u{}", unicode_digits),
                    reason: "not a valid hex number",
                }
            })?;
            out.push(char::from_u32(codepoint).ok_or_else(|| {
                Error::InvalidUnicodeEscapeSequence {
                    seq: format!("\\u{}", unicode_digits),
                    reason: "not a valid unicode codepoint",
                }
            })?);
            unicode_digits = String::new();
            state = EscapeState::Normal;
        }
    }

    match state {
        EscapeState::Normal => {}
        EscapeState::Escape => {
            return Err(Error::UnexpectedEndOfInput {
                expected: "escape sequence",
            })
        }
        EscapeState::Unicode => {
            return Err(Error::UnexpectedEndOfInput {
                expected: "unicode escape sequence",
            })
        }
    }

    Ok(out)
}

impl Deserializable for UnescapedString {
    fn deserialize(
        value: &impl DeserializableValue,
        name: &str,
        diagnostics: &mut Vec<DeserializationDiagnostic>,
    ) -> Option<Self> {
        let Some(str) = String::deserialize(value, &name, diagnostics) else {
            return None;
        };

        match unescape_str(str) {
            Ok(s) => Some(Self(s)),
            Err(e) => {
                diagnostics.push(DeserializationDiagnostic::new(format!("{}", e)));
                None
            }
        }
    }
}

impl From<UnescapedString> for String {
    fn from(value: UnescapedString) -> Self {
        value.0
    }
}

// For testing purposes
impl From<&'static str> for UnescapedString {
    fn from(value: &'static str) -> Self {
        Self(value.to_owned())
    }
}
