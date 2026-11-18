//! Scanning primitives shared by the language plugins.
//!
//! Everything works on raw bytes (`&[u8]`) and reports positions as [`Span`]s,
//! which are byte offsets. Only specifiers are decoded to text, through
//! [`decode()`], which borrows when the source contains no escapes.
//!
//! Building blocks:
//! - [`Cursor`]: a byte cursor with `memchr`-backed skipping
//!   ([`Cursor::skip_until_any`]).
//! - [`skip`]: comment skippers (`//`, `/* */`, nested block comments, `#`) and
//!   quoted string scanning with escape handling.
//! - [`decode()`]: escape decoding to `Cow<str>`.
//! - [`LineIndex`]: byte offset to line/column conversion, computed lazily on
//!   demand.
//! - [`blocks`]: SIMD classification of 64-byte blocks against byte sets, for
//!   lexers that skim between interesting bytes.
//!
//! All functions are linear in the input and never panic on arbitrary bytes.

pub mod blocks;
mod cursor;
pub mod decode;
mod line;
pub mod skip;
mod span;

pub use cursor::Cursor;
pub use decode::{DecodeError, EscapeRules, decode};
pub use line::{LineCol, LineIndex};
pub use skip::{LexError, LexErrorKind, QuoteOpts, StrLit};
pub use span::Span;
