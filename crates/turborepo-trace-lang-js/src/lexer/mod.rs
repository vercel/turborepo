//! A hand-written, byte-oriented JS / TS / JSX lexer.
//!
//! The lexer is exact about what is code and what is a comment, string,
//! template, regex or JSX text, and produces span-only [`Token`]s on demand
//! ([`Lexer::next_token`]). It never backtracks and never allocates per token:
//! the only heap state is the context stack.
//!
//! # Regex vs division
//!
//! A `/` starts a regex literal when the previous significant token cannot end
//! an expression ([`Lexer::regex_allowed`]): punctuators and operators,
//! expression keywords (`return`, `typeof`, `case`, `yield`, `await`, `void`,
//! `delete`, `instanceof`, `in`, `of`, `new`, `throw`, `else`, `do`, `extends`,
//! `default`), the `)` of `if/while/for/with (...)` and the `}` that closes a
//! block (as opposed to an object literal or a template substitution). Keywords
//! used as property names (`a.return / 2`) are plain identifiers.
//! In TypeScript, a `!` glued to the end of an expression is the non-null
//! assertion.
//!
//! Whether a `{` opens a block or an object literal is decided from the
//! previous token and the enclosing context (see `brace_is_block`). Mistakes
//! there only matter when a `}` is directly followed by a `/`, which real code
//! essentially never does.
//!
//! # Context stack
//!
//! Every `(`, `[`, `{`, template substitution `${` and JSX element pushes a
//! [`Ctx`]. Each [`Token`] records the stack depth (before an opener is pushed
//! and after a closer is popped, so matching delimiters have equal depth) and
//! whether it is inside a `try` block. Template literals with nested `${}`
//! resume when the `}` closing a substitution is reached. Unbalanced closers
//! are recovered from in O(1) amortized time.
//!
//! # JSX
//!
//! JSX is lexed when a `<` appears where an expression starts and is followed
//! by an identifier or `>` (fragment). Tag names, attributes and text are
//! skipped; only the `{...}` expression containers are lexed as code. In
//! `.tsx`, `<T,>`, `<T = D>`, `<T extends U>` and `<const T>` are TypeScript
//! type parameters, not JSX, and so is `<T>` before a parameter list and `=>`
//! (a generic function type: `f: <T>(x: T) => T`). Type arguments of an element
//! (`<Select<Option, true> ...>`) are skipped with the tag name.
//! Modes per dialect ([`Jsx`]):
//!
//! - `.jsx` / `.tsx`: JSX on.
//! - `.ts` / `.mts` / `.cts`: off (`<T>expr` is a type assertion there).
//! - `.js` / `.mjs` / `.cjs` and unknown: *heuristic*. A `<` in expression
//!   position cannot be a valid operator in plain JS, so JSX is the only
//!   correct reading. It can still be wrong for Flow-typed code (`<T>(x: T) =>
//!   x`): if JSX lexing reaches the end of input inside an element, or hits a
//!   byte that cannot appear in a tag, the lexer reports [`Lexer::jsx_failed`]
//!   and the caller re-runs the file with JSX off (at most once, so the whole
//!   extraction stays linear).
//!
//! # Skimming
//!
//! [`Lexer::next_matching_top`] (the recognizer's hot path) returns only
//! trigger identifiers: keywords from a mask, keywords at depth 0, and dynamic
//! words registered with [`Lexer::add_trigger_word`] (names bound to tracked
//! modules). It runs a block skim (`skim.rs`): SIMD masks of the bytes that can
//! change the lexer state ("events") and of identifier bytes, 64 bytes at a
//! time, so identifiers, whitespace and most operators are never tokenized. The
//! previous significant token is reconstructed from the bytes before an
//! event only when the event depends on it; ambiguous runs are re-lexed
//! exactly. The result is identical to filtering [`Lexer::next_token`] (tests
//! and a fuzz target compare it with a per-token reference implementation,
//! [`Lexer::set_reference_skim`]). TypeScript `import` tokens carry
//! [`TYPE_POS`] when they follow a type-annotation colon or a `<`.
//!
//! # Errors
//!
//! Malformed input (unterminated strings, comments, templates, regexes,
//! unbalanced delimiters) never stops the lexer. The first problem is recorded
//! as a [`UncertaintyKind::ParseIncomplete`] uncertainty
//! ([`Lexer::take_error`]); later ones are ignored.

mod skim;
mod tables;
#[cfg(test)]
mod tests;

use std::path::Path;

pub use tables::Kw;
use tables::{IDENT, IDENT_CLASS, IDENT_START, IDENT_START_ASCII, keyword, unicode_ws_len};
use turborepo_trace_core::{Span, Uncertainty, UncertaintyKind, lexkit::blocks::find_any3};

/// Whether and how JSX is recognized.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Jsx {
    /// Never (`.ts`, `.mts`, `.cts`).
    Off,
    /// Always (`.jsx`, `.tsx`).
    On,
    /// Plain JS (`.js`, `.mjs`, `.cjs`): JSX where a `<` cannot be anything
    /// else, with a fallback to no JSX if that reading fails (see the
    /// module docs).
    #[default]
    Heuristic,
}

/// The source dialect, derived from the file name.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Dialect {
    /// TypeScript syntax (non-null `!`, type parameters in `.tsx`).
    pub ts: bool,
    /// JSX handling.
    pub jsx: Jsx,
    /// A declaration file (`.d.ts`, `.d.mts`, `.d.cts`): every reference is
    /// type-only.
    pub dts: bool,
}

impl Dialect {
    /// Plain JavaScript with the JSX heuristic.
    pub const JS: Dialect = Dialect {
        ts: false,
        jsx: Jsx::Heuristic,
        dts: false,
    };
    /// `.jsx`.
    pub const JSX: Dialect = Dialect {
        ts: false,
        jsx: Jsx::On,
        dts: false,
    };
    /// `.ts`, `.mts`, `.cts`.
    pub const TS: Dialect = Dialect {
        ts: true,
        jsx: Jsx::Off,
        dts: false,
    };
    /// `.tsx`.
    pub const TSX: Dialect = Dialect {
        ts: true,
        jsx: Jsx::On,
        dts: false,
    };
    /// `.d.ts`, `.d.mts`, `.d.cts`.
    pub const DTS: Dialect = Dialect {
        ts: true,
        jsx: Jsx::Off,
        dts: true,
    };

    /// Picks the dialect from a file name. Unknown extensions are plain JS.
    pub fn from_path(path: &Path) -> Dialect {
        use turborepo_trace_core::os::OsStrExt;
        let name = path.file_name().map(|n| n.as_bytes()).unwrap_or_default();
        let ends = |s: &[u8]| name.len() > s.len() && name.ends_with(s);
        if ends(b".tsx") {
            Dialect::TSX
        } else if ends(b".d.ts") || ends(b".d.mts") || ends(b".d.cts") {
            Dialect::DTS
        } else if ends(b".ts") || ends(b".mts") || ends(b".cts") {
            Dialect::TS
        } else if ends(b".jsx") {
            Dialect::JSX
        } else {
            Dialect::JS
        }
    }
}

/// Token kinds. Punctuators the recognizer or the regex rule care about have
/// their own kind; all other operators are [`TokKind::Op`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum TokKind {
    /// End of input (also the "previous token" at the start of input).
    #[default]
    Eof,
    /// Identifier or keyword (see [`Token::kw`]).
    Ident,
    /// `#name`.
    PrivateName,
    /// `'...'` or `"..."`.
    Str,
    /// A template literal without substitutions.
    Template,
    /// `` `...${ ``.
    TemplateHead,
    /// `}...${`.
    TemplateMiddle,
    /// `` }...` ``.
    TemplateTail,
    /// `/.../flags`.
    Regex,
    /// Numeric literal, including separators, bigint suffix, hex/octal/binary.
    Number,
    /// End of a complete JSX element (span: its closing tag); a value.
    Jsx,
    /// A `///` comment before any code (a TypeScript triple-slash directive
    /// candidate). Never the "previous token".
    TripleSlash,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    /// `?.`
    OptChain,
    /// `...`
    Ellipsis,
    Semi,
    Comma,
    Colon,
    Question,
    /// `=`
    Assign,
    /// `=>`
    Arrow,
    Lt,
    Gt,
    Bang,
    Star,
    /// `++` or `--`
    IncDec,
    At,
    Hash,
    /// Any other operator.
    Op,
}

/// Token flag: inside a `try { ... }` block (at any depth).
pub const IN_TRY: u8 = 1 << 0;
/// Token flag: a string or template contains a backslash escape.
pub const ESCAPES: u8 = 1 << 1;
/// Token flag: a string, template or regex is unterminated.
pub const UNTERMINATED: u8 = 1 << 2;
/// Token flag: a line terminator precedes the token.
pub const NEWLINE_BEFORE: u8 = 1 << 3;
/// Token flag, set only on `import` in TypeScript: the previous token puts it
/// in a type position — a `:` that is not a ternary, object-literal or `case`
/// colon (a type annotation), or a `<` (type arguments).
pub const TYPE_POS: u8 = 1 << 4;

/// A token: a byte span plus classification. 16 bytes, `Copy`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Token {
    /// Start offset.
    pub start: u32,
    /// End offset (exclusive).
    pub end: u32,
    /// Context stack depth: before an opener is pushed, after a closer is
    /// popped.
    pub depth: u32,
    /// Kind.
    pub kind: TokKind,
    /// Keyword classification for [`TokKind::Ident`]; [`Kw::None`] after `.` /
    /// `?.`.
    pub kw: Kw,
    /// `IN_TRY | ESCAPES | UNTERMINATED | NEWLINE_BEFORE`.
    pub flags: u8,
    /// [`Token::kw`] of the previous significant token ([`Kw::None`] if it was
    /// not a keyword-classified identifier).
    pub prev_kw: Kw,
}

impl Token {
    /// The token's span.
    #[inline]
    pub fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }

    /// The token's bytes in `src` (empty if out of range).
    #[inline]
    pub fn text<'s>(&self, src: &'s [u8]) -> &'s [u8] {
        src.get(self.start as usize..self.end as usize)
            .unwrap_or_default()
    }

    /// Whether the token is in a `try` block.
    #[inline]
    pub fn in_try(&self) -> bool {
        self.flags & IN_TRY != 0
    }

    /// For string literals and substitution-free templates: the raw contents
    /// between the delimiters (escapes not decoded), or `None` for other
    /// tokens and unterminated ones.
    #[inline]
    pub fn literal_content<'s>(&self, src: &'s [u8]) -> Option<&'s [u8]> {
        match self.kind {
            TokKind::Str | TokKind::Template if self.flags & UNTERMINATED == 0 => {
                src.get(self.start as usize + 1..(self.end as usize).checked_sub(1)?)
            }
            _ => None,
        }
    }
}

/// The offset just past the `*/` that closes a block comment whose body starts
/// at `from`. A `memchr` loop: cheaper than building a `memmem` searcher for
/// the short comments (`/*#__PURE__*/`) that fill bundles.
#[inline]
fn block_comment_end(src: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(off) = memchr::memchr(b'*', src.get(i..)?) {
        let star = i + off;
        if src.get(star + 1) == Some(&b'/') {
            return Some(star + 2);
        }
        i = star + 1;
    }
    None
}

/// The offset just past the `>` that closes the TypeScript type argument list
/// whose `<` is at `lt` (`<A, B<C>, (x: D) => E, { f: 'g>' }>`), or `None` if
/// the brackets do not balance before the end of input. One forward pass: angle
/// brackets count only outside `( [ {` (inside, they are balanced or part of
/// `=>`), and strings, template literal types and comments are skipped.
fn jsx_type_args_end(src: &[u8], lt: usize) -> Option<usize> {
    let mut angle = 0u32;
    let mut nest = 0u32;
    let mut i = lt;
    loop {
        let b = *src.get(i)?;
        i += 1;
        match b {
            b'<' if nest == 0 => angle += 1,
            b'>' if nest == 0 => {
                angle -= 1;
                if angle == 0 {
                    return Some(i);
                }
            }
            // `=>` of a function type.
            b'=' if src.get(i) == Some(&b'>') => i += 1,
            b'(' | b'[' | b'{' => nest += 1,
            b')' | b']' | b'}' => nest = nest.checked_sub(1)?,
            b'\'' | b'"' | b'`' => loop {
                match *src.get(i)? {
                    b'\\' => i += 2,
                    b'\n' if b != b'`' => return None,
                    c => {
                        i += 1;
                        if c == b {
                            break;
                        }
                    }
                }
            },
            b'/' => match src.get(i) {
                Some(b'/') => i = i + memchr::memchr(b'\n', &src[i..])?,
                Some(b'*') => i = block_comment_end(src, i + 1)?,
                _ => {}
            },
            // A statement end: these were not type arguments.
            b';' if nest == 0 => return None,
            _ => {}
        }
    }
}

/// How far [`generic_function_type`] looks for the `)` of a parameter list. A
/// bound keeps the lookahead linear in total: every `<` in expression position
/// pays at most this much.
const PARAMS_LOOKAHEAD: usize = 2048;

/// Whether the bytes at `i` (just past `<T>`) are a parenthesized list followed
/// by `=>`: `(x: T, f: (y: U) => V) => R`. Only parentheses are counted, so
/// apostrophes and braces in JSX text (`<b>(don't {x})</b>`) cannot mislead it,
/// and a closing tag ends the search.
fn generic_function_type(src: &[u8], mut i: usize) -> bool {
    while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
        i += 1;
    }
    if src.get(i) != Some(&b'(') {
        return false;
    }
    let end = src.len().min(i + PARAMS_LOOKAHEAD);
    let mut depth = 0u32;
    while i < end {
        match src[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                        i += 1;
                    }
                    return src[i..].starts_with(b"=>");
                }
            }
            // A closing tag: text of an element (`x = <b>(</b>) => 1` is not a type).
            b'<' if src.get(i + 1) == Some(&b'/') => return false,
            _ => {}
        }
        i += 1;
    }
    false
}

/// What an entry on the context stack is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum CtxKind {
    /// `(`.
    Paren,
    /// `(` after `if`, `while`, `for`, `for await`, `with`: a statement follows
    /// its `)`.
    ParenCond,
    /// `[`.
    Bracket,
    /// `{` opening a block.
    Block,
    /// `{` opening a `try` block.
    TryBlock,
    /// `{` opening an object literal (or other expression-level braces).
    Object,
    /// `${` in a template literal.
    Template,
    /// `{` in a JSX tag (attribute value or spread).
    JsxAttr,
    /// `{` in JSX children.
    JsxChild,
    /// Inside a JSX opening tag (name and attributes).
    JsxTag,
    /// Inside a JSX element's children.
    JsxChildren,
}

#[derive(Copy, Clone, Debug)]
struct Ctx {
    kind: CtxKind,
    /// Open `?` count of the enclosing context, restored on pop.
    saved_ternary: u32,
}

/// The lexer. See the module documentation.
#[derive(Debug)]
pub struct Lexer<'s> {
    src: &'s [u8],
    pos: usize,
    dialect: Dialect,
    stack: Vec<Ctx>,
    /// Whether the top of `stack` is a JSX tag or children context (cached
    /// mode).
    jsx_top: bool,
    /// Open `?` (ternary or optional marker) count in the current context.
    ternary: u32,
    try_depth: u32,
    last: TokKind,
    last_kw: Kw,
    prev_kw: Kw,
    last_paren_cond: bool,
    last_brace_block: bool,
    last_bang_postfix: bool,
    last_colon_ternary: bool,
    /// A `case` / `default` label is open (its `:` is not a type annotation).
    case_pending: bool,
    /// The last `:` closed a `case` / `default` label.
    last_colon_case: bool,
    newline: bool,
    done: bool,
    jsx_failed: bool,
    /// A `<` after a tag name was not a type argument list (see
    /// [`Lexer::open_jsx_tag`]).
    jsx_type_args_failed: bool,
    error: Option<Uncertainty>,
    /// The lowest context depth reached since [`Lexer::reset_low_water`].
    low_water: usize,
    /// Dynamic trigger words: bit `min(len, 15)` of entry `first byte` (see
    /// [`Lexer::add_trigger_word`]).
    dyn_filter: Option<Box<[u16; 256]>>,
    /// Use the block skim (`skim.rs`) in [`Lexer::next_matching_top`]; off
    /// selects the per-token reference path.
    fast_skim: bool,
    /// The `(mask, top_mask)` that `skim_filter` was built for.
    skim_key: Option<(u64, u64)>,
    /// Words the block skim must look at: bit `min(len, 15)` of entry `first
    /// byte`.
    skim_filter: [u16; 256],
    /// The first bytes with a non-empty `skim_filter` entry.
    skim_first: turborepo_trace_core::lexkit::blocks::ByteSet,
    /// The second bytes of the words in `skim_filter`.
    skim_second: turborepo_trace_core::lexkit::blocks::ByteSet,
    /// The first bytes of one-byte dynamic trigger words (they have no second
    /// byte).
    skim_short_first: turborepo_trace_core::lexkit::blocks::ByteSet,
    /// Second bytes of dynamic trigger words (a 256-bit set).
    dyn_second: [u64; 4],
}

#[cfg(test)]
thread_local! {
    /// Test hook: new lexers use the per-token reference skim.
    pub(crate) static REFERENCE_SKIM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl<'s> Lexer<'s> {
    /// Creates a lexer over `src`. A leading BOM and hashbang line are skipped.
    pub fn new(src: &'s [u8], dialect: Dialect) -> Self {
        let mut pos = 0;
        if src.starts_with(b"\xEF\xBB\xBF") {
            pos = 3;
        }
        if src[pos..].starts_with(b"#!") {
            pos = memchr::memchr2(b'\n', b'\r', &src[pos..]).map_or(src.len(), |i| pos + i);
        }
        Lexer {
            src,
            pos,
            dialect,
            stack: Vec::new(),
            jsx_top: false,
            ternary: 0,
            try_depth: 0,
            last: TokKind::Eof,
            last_kw: Kw::None,
            prev_kw: Kw::None,
            last_paren_cond: false,
            last_brace_block: false,
            last_bang_postfix: false,
            last_colon_ternary: false,
            case_pending: false,
            last_colon_case: false,
            newline: false,
            done: false,
            jsx_failed: false,
            jsx_type_args_failed: false,
            error: None,
            low_water: 0,
            dyn_filter: None,
            #[cfg(not(test))]
            fast_skim: true,
            #[cfg(test)]
            fast_skim: !REFERENCE_SKIM.with(|c| c.get()),
            skim_key: None,
            skim_filter: [0; 256],
            skim_first: turborepo_trace_core::lexkit::blocks::ByteSet::EMPTY,
            skim_second: turborepo_trace_core::lexkit::blocks::ByteSet::EMPTY,
            skim_short_first: turborepo_trace_core::lexkit::blocks::ByteSet::EMPTY,
            dyn_second: [0; 4],
        }
    }

    /// Selects the per-token reference implementation of
    /// [`Lexer::next_matching_top`] instead of the block skim. Both return
    /// identical tokens; this exists for differential tests and fuzzing.
    #[doc(hidden)]
    pub fn set_reference_skim(&mut self, on: bool) {
        self.fast_skim = !on;
    }

    /// The lowest context depth reached since the last
    /// [`Lexer::reset_low_water`] (the recognizer uses it to notice that a
    /// scope closed among skipped tokens).
    #[inline]
    pub fn low_water(&self) -> usize {
        self.low_water
    }

    /// Restarts low-water tracking at the current depth.
    #[inline]
    pub fn reset_low_water(&mut self) {
        self.low_water = self.stack.len();
    }

    /// Makes identifiers equal to `word` stop [`Lexer::next_matching`] (in
    /// addition to the keyword mask). The filter is approximate (first
    /// byte, length, and the second byte of some trigger word): callers
    /// re-check the text.
    pub fn add_trigger_word(&mut self, word: &[u8]) {
        if let Some(&f) = word.first() {
            let t = self.dyn_filter.get_or_insert_with(|| Box::new([0; 256]));
            t[f as usize] |= 1 << word.len().min(15);
            if let Some(&s) = word.get(1) {
                self.dyn_second[s as usize / 64] |= 1 << (s % 64);
            }
            self.skim_key = None;
        }
    }

    /// Whether `t` may be a dynamic trigger word (see
    /// [`Lexer::add_trigger_word`]).
    #[inline(always)]
    pub fn is_dyn_trigger(&self, t: &Token) -> bool {
        self.dyn_filter.is_some()
            && t.kind == TokKind::Ident
            && t.kw == Kw::None
            && t.start < t.end
            && (t.end as usize) <= self.src.len()
            && self.dyn_hit(t.start as usize, t.end as usize)
    }

    /// The source buffer.
    #[inline]
    pub fn src(&self) -> &'s [u8] {
        self.src
    }

    /// Whether heuristic JSX lexing failed (see the module docs). The lexer
    /// stops (returns [`TokKind::Eof`]) as soon as this becomes true.
    pub fn jsx_failed(&self) -> bool {
        self.jsx_failed
    }

    /// The first lexing problem, if any.
    pub fn take_error(&mut self) -> Option<Uncertainty> {
        self.error.take()
    }

    fn err(&mut self, start: usize, end: usize, what: &str) {
        if self.error.is_none() {
            self.error = Some(Uncertainty::new(
                UncertaintyKind::ParseIncomplete,
                Span::from_usize(start, end.max(start)),
                format!("{what} at bytes {start}..{end}; lexer recovered"),
            ));
        }
    }

    /// Whether a regex literal (or, with JSX on, an element) may start here:
    /// the previous significant token cannot end an expression.
    pub fn regex_allowed(&self) -> bool {
        match self.last {
            TokKind::Eof => true,
            TokKind::Ident => self.last_kw.expr_follows(),
            TokKind::PrivateName
            | TokKind::Number
            | TokKind::Str
            | TokKind::Template
            | TokKind::TemplateTail
            | TokKind::Regex
            | TokKind::Jsx
            | TokKind::RBracket
            | TokKind::IncDec => false,
            TokKind::RParen => self.last_paren_cond,
            TokKind::RBrace => self.last_brace_block,
            TokKind::Bang => !self.last_bang_postfix,
            _ => true,
        }
    }

    /// Whether a `{` here opens a block (rather than an object literal).
    #[inline(always)]
    fn brace_is_block(&self) -> bool {
        let top = self.stack.last().map(|c| c.kind);
        match self.last {
            TokKind::Eof | TokKind::Semi | TokKind::RBrace | TokKind::Arrow | TokKind::RParen => {
                true
            }
            TokKind::LBrace => matches!(top, None | Some(CtxKind::Block | CtxKind::TryBlock)),
            TokKind::Colon => !(self.last_colon_ternary || top == Some(CtxKind::Object)),
            TokKind::Ident => {
                // `(): void {` is a TypeScript return type, not `void {}`.
                !self.last_kw.object_follows() || (self.dialect.ts && self.last_kw == Kw::Void)
            }
            // Values followed by `{`: a new statement after ASI, or a TypeScript type
            // (`(): Promise<T> {`, `(): string[] {`, `(): 'a' {`).
            TokKind::Gt => self.dialect.ts,
            TokKind::PrivateName
            | TokKind::Number
            | TokKind::Str
            | TokKind::Template
            | TokKind::TemplateTail
            | TokKind::Regex
            | TokKind::Jsx
            | TokKind::RBracket
            | TokKind::IncDec => true,
            TokKind::Bang => self.last_bang_postfix,
            _ => false,
        }
    }

    #[inline(always)]
    fn push(&mut self, kind: CtxKind) {
        if kind == CtxKind::TryBlock {
            self.try_depth += 1;
        }
        self.stack.push(Ctx {
            kind,
            saved_ternary: self.ternary,
        });
        self.ternary = 0;
        self.jsx_top = matches!(kind, CtxKind::JsxTag | CtxKind::JsxChildren);
    }

    #[inline(always)]
    fn pop(&mut self) -> Option<CtxKind> {
        let c = self.stack.pop()?;
        if c.kind == CtxKind::TryBlock {
            self.try_depth -= 1;
        }
        self.ternary = c.saved_ternary;
        // Code contexts (brackets, braces, `${`) are only pushed in code mode, so
        // popping one returns to code mode.
        self.jsx_top = matches!(
            c.kind,
            CtxKind::JsxAttr | CtxKind::JsxChild | CtxKind::JsxTag | CtxKind::JsxChildren
        ) && matches!(self.top(), Some(CtxKind::JsxTag | CtxKind::JsxChildren));
        self.low_water = self.low_water.min(self.stack.len());
        Some(c.kind)
    }

    #[inline]
    fn top(&self) -> Option<CtxKind> {
        self.stack.last().map(|c| c.kind)
    }

    /// Builds a token ending at `self.pos` and makes it the previous
    /// significant token.
    #[inline(always)]
    fn tok(&mut self, kind: TokKind, start: usize, depth: usize, kw: Kw, flags: u8) -> Token {
        let flags = flags
            | (u8::from(self.try_depth > 0) * IN_TRY)
            | (u8::from(self.newline) * NEWLINE_BEFORE);
        self.commit(kind, kw);
        Token {
            start: start as u32,
            end: self.pos as u32,
            depth: depth as u32,
            kind,
            kw,
            flags,
            prev_kw: self.prev_kw,
        }
    }

    /// Makes a token of `kind` the previous significant token without building
    /// it (the state part of [`Lexer::tok`]).
    #[inline(always)]
    fn commit(&mut self, kind: TokKind, kw: Kw) {
        self.newline = false;
        self.prev_kw = self.last_kw;
        self.last = kind;
        self.last_kw = kw;
    }

    /// Context bookkeeping for a `(` (already consumed).
    #[inline(always)]
    fn open_paren(&mut self) {
        let kind = self.paren_kind();
        self.push(kind);
    }

    /// The context a `(` here opens.
    #[inline(always)]
    fn paren_kind(&self) -> CtxKind {
        let cond = matches!(self.last_kw, Kw::If | Kw::While | Kw::For | Kw::With)
            && self.last == TokKind::Ident
            || (self.last_kw == Kw::Await && self.prev_kw == Kw::For);
        if cond {
            CtxKind::ParenCond
        } else {
            CtxKind::Paren
        }
    }

    /// Context bookkeeping for a `{` (already consumed).
    #[inline(always)]
    fn open_brace(&mut self) {
        let kind = if self.last == TokKind::Ident && self.last_kw == Kw::Try {
            CtxKind::TryBlock
        } else if self.brace_is_block() {
            CtxKind::Block
        } else {
            CtxKind::Object
        };
        self.push(kind);
    }

    /// Ternary / `case` bookkeeping for a `:`.
    #[inline(always)]
    fn colon(&mut self) {
        self.last_colon_ternary = self.ternary > 0;
        if self.ternary > 0 {
            self.ternary -= 1;
            self.last_colon_case = false;
        } else {
            self.last_colon_case = self.case_pending;
            self.case_pending = false;
        }
    }

    /// Context bookkeeping for the `)` at `start` (already consumed).
    #[inline(always)]
    fn close_paren(&mut self, start: usize) {
        let cond = self.close(
            start,
            &[CtxKind::Paren, CtxKind::ParenCond],
            CtxKind::Bracket,
        ) == Some(CtxKind::ParenCond);
        self.last_paren_cond = cond;
    }

    fn eof(&mut self) -> Token {
        if !self.done {
            self.done = true;
            if !self.stack.is_empty() {
                if self.dialect.jsx == Jsx::Heuristic
                    && self
                        .stack
                        .iter()
                        .any(|c| matches!(c.kind, CtxKind::JsxTag | CtxKind::JsxChildren))
                {
                    self.jsx_failed = true;
                }
                let len = self.src.len();
                self.err(len, len, "unclosed delimiter at end of input");
            }
        }
        let len = self.src.len();
        Token {
            start: len as u32,
            end: len as u32,
            depth: self.stack.len() as u32,
            kind: TokKind::Eof,
            kw: Kw::None,
            flags: 0,
            prev_kw: self.last_kw,
        }
    }

    /// Returns the next token. After the end of input, keeps returning
    /// [`TokKind::Eof`].
    pub fn next_token(&mut self) -> Token {
        self.next_token_inline()
    }

    /// [`Lexer::next_token`], inlined into the hot loop of
    /// [`Lexer::next_matching`] so the per-token work runs without a call
    /// boundary.
    #[inline(always)]
    fn next_token_inline(&mut self) -> Token {
        loop {
            if self.done {
                return self.eof();
            }
            let t = if !self.jsx_top {
                self.normal()
            } else if self.top() == Some(CtxKind::JsxTag) {
                self.jsx_tag()
            } else {
                self.jsx_children()
            };
            if let Some(t) = t {
                return t;
            }
        }
    }

    /// Lexes one token in code mode. Returns `None` when the mode changed
    /// without a token (entering JSX).
    #[inline(always)]
    fn normal(&mut self) -> Option<Token> {
        let src = self.src;
        // Whitespace and comments.
        loop {
            let Some(&b) = src.get(self.pos) else {
                return Some(self.eof());
            };
            // Fast path: identifiers are the most common token.
            if IDENT_START_ASCII[b as usize] {
                return Some(self.ident(self.pos));
            }
            match b {
                b' ' | b'\t' | 0x0b | 0x0c => self.pos += 1,
                b'\n' | b'\r' => {
                    self.pos += 1;
                    self.newline = true;
                }
                b'/' => match src.get(self.pos + 1) {
                    Some(b'/') => {
                        let start = self.pos;
                        self.pos = memchr::memchr2(b'\n', b'\r', &src[start..])
                            .map_or(src.len(), |i| start + i);
                        // Only before any code: `last` is still the start-of-input marker.
                        if self.last == TokKind::Eof && src.get(start + 2) == Some(&b'/') {
                            return Some(Token {
                                start: start as u32,
                                end: self.pos as u32,
                                depth: 0,
                                kind: TokKind::TripleSlash,
                                kw: Kw::None,
                                flags: 0,
                                prev_kw: Kw::None,
                            });
                        }
                    }
                    Some(b'*') => {
                        let start = self.pos;
                        match block_comment_end(src, start + 2) {
                            Some(end) => {
                                let body = &src[start + 2..end - 2];
                                if memchr::memchr2(b'\n', b'\r', body).is_some() {
                                    self.newline = true;
                                }
                                self.pos = end;
                            }
                            None => {
                                self.pos = src.len();
                                self.err(start, src.len(), "unterminated block comment");
                            }
                        }
                    }
                    _ => break,
                },
                0x80.. => match unicode_ws_len(&src[self.pos..]) {
                    0 => break,
                    n => self.pos += n,
                },
                _ => break,
            }
        }

        let start = self.pos;
        let b = src[start];
        let depth = self.stack.len();
        if IDENT_START[b as usize] {
            return Some(self.ident(start));
        }
        let t = match b {
            b'0'..=b'9' => self.number(start),
            b'"' | b'\'' => self.string(start, b),
            b'`' => {
                self.pos += 1;
                self.template(start, depth, TokKind::Template, TokKind::TemplateHead)
            }
            b'(' => {
                self.pos += 1;
                self.open_paren();
                self.tok(TokKind::LParen, start, depth, Kw::None, 0)
            }
            b')' => {
                self.pos += 1;
                self.close_paren(start);
                let depth = self.stack.len();
                self.tok(TokKind::RParen, start, depth, Kw::None, 0)
            }
            b'[' => {
                self.pos += 1;
                self.push(CtxKind::Bracket);
                self.tok(TokKind::LBracket, start, depth, Kw::None, 0)
            }
            b']' => {
                self.pos += 1;
                self.close(start, &[CtxKind::Bracket], CtxKind::Paren);
                let depth = self.stack.len();
                self.tok(TokKind::RBracket, start, depth, Kw::None, 0)
            }
            b'{' => {
                self.pos += 1;
                self.open_brace();
                self.tok(TokKind::LBrace, start, depth, Kw::None, 0)
            }
            b'}' => return self.close_brace(start),
            b';' => self.punct(start, 1, TokKind::Semi),
            b',' => self.punct(start, 1, TokKind::Comma),
            b':' => {
                self.colon();
                self.punct(start, 1, TokKind::Colon)
            }
            b'.' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(b'0'..=b'9'), _) => self.number(start),
                (Some(b'.'), Some(b'.')) => self.punct(start, 3, TokKind::Ellipsis),
                _ => self.punct(start, 1, TokKind::Dot),
            },
            b'?' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(b'.'), c) if !matches!(c, Some(b'0'..=b'9')) => {
                    self.punct(start, 2, TokKind::OptChain)
                }
                (Some(b'?'), Some(b'=')) => self.punct(start, 3, TokKind::Op),
                (Some(b'?'), _) => self.punct(start, 2, TokKind::Op),
                _ => {
                    self.ternary += 1;
                    self.punct(start, 1, TokKind::Question)
                }
            },
            b'=' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(b'>'), _) => self.punct(start, 2, TokKind::Arrow),
                (Some(b'='), Some(b'=')) => self.punct(start, 3, TokKind::Op),
                (Some(b'='), _) => self.punct(start, 2, TokKind::Op),
                _ => self.punct(start, 1, TokKind::Assign),
            },
            b'!' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(b'='), Some(b'=')) => self.punct(start, 3, TokKind::Op),
                (Some(b'='), _) => self.punct(start, 2, TokKind::Op),
                _ => {
                    // Glued to the previous token: no whitespace or comment in between.
                    let glued = start > 0
                        && !matches!(
                            src[start - 1],
                            b' ' | b'\t' | b'\n' | b'\r' | b'/' | 0x0b | 0x0c
                        );
                    let postfix = self.dialect.ts
                        && glued
                        && matches!(
                            self.last,
                            TokKind::Ident
                                | TokKind::PrivateName
                                | TokKind::RParen
                                | TokKind::RBracket
                                | TokKind::Str
                                | TokKind::Template
                                | TokKind::TemplateTail
                                | TokKind::Number
                                | TokKind::Bang
                        )
                        && (self.last != TokKind::Ident || !self.last_kw.expr_follows())
                        && (self.last != TokKind::Bang || self.last_bang_postfix);
                    let t = self.punct(start, 1, TokKind::Bang);
                    self.last_bang_postfix = postfix;
                    t
                }
            },
            b'<' => {
                if self.dialect.jsx != Jsx::Off && self.regex_allowed() && self.jsx_start(start) {
                    self.open_jsx_tag(start);
                    return None;
                }
                match (src.get(start + 1), src.get(start + 2)) {
                    (Some(b'<'), Some(b'=')) => self.punct(start, 3, TokKind::Op),
                    (Some(b'<' | b'='), _) => self.punct(start, 2, TokKind::Op),
                    _ => self.punct(start, 1, TokKind::Lt),
                }
            }
            b'>' => self.punct(start, 1, TokKind::Gt),
            b'+' | b'-' => match src.get(start + 1) {
                Some(&c) if c == b => self.punct(start, 2, TokKind::IncDec),
                Some(b'=') => self.punct(start, 2, TokKind::Op),
                _ => self.punct(start, 1, TokKind::Op),
            },
            b'*' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(b'*'), Some(b'=')) => self.punct(start, 3, TokKind::Op),
                (Some(b'*' | b'='), _) => self.punct(start, 2, TokKind::Op),
                _ => self.punct(start, 1, TokKind::Star),
            },
            b'/' => {
                if self.regex_allowed() {
                    self.regex(start)
                } else if src.get(start + 1) == Some(&b'=') {
                    self.punct(start, 2, TokKind::Op)
                } else {
                    self.punct(start, 1, TokKind::Op)
                }
            }
            b'&' | b'|' => match (src.get(start + 1), src.get(start + 2)) {
                (Some(&c), Some(b'=')) if c == b => self.punct(start, 3, TokKind::Op),
                (Some(&c), _) if c == b || c == b'=' => self.punct(start, 2, TokKind::Op),
                _ => self.punct(start, 1, TokKind::Op),
            },
            b'%' | b'^' => match src.get(start + 1) {
                Some(b'=') => self.punct(start, 2, TokKind::Op),
                _ => self.punct(start, 1, TokKind::Op),
            },
            b'@' => self.punct(start, 1, TokKind::At),
            b'#' => {
                if src.get(start + 1).is_some_and(|&c| IDENT_START[c as usize]) {
                    self.pos = start + 1;
                    self.skip_ident_tail();
                    self.tok(TokKind::PrivateName, start, depth, Kw::None, 0)
                } else {
                    self.punct(start, 1, TokKind::Hash)
                }
            }
            // `~`, control bytes, stray bytes: an operator-like token.
            _ => self.punct(start, 1, TokKind::Op),
        };
        Some(t)
    }

    #[inline(always)]
    fn punct(&mut self, start: usize, len: usize, kind: TokKind) -> Token {
        self.pos = start + len;
        let depth = self.stack.len();
        self.tok(kind, start, depth, Kw::None, 0)
    }

    #[inline(always)]
    fn skip_ident_tail(&mut self) {
        let src = self.src;
        let mut i = self.pos;
        loop {
            while let Some(&c) = src.get(i)
                && IDENT_CLASS[c as usize] == 1
            {
                i += 1;
            }
            // A non-ASCII byte continues the identifier unless it starts whitespace.
            match src.get(i) {
                Some(&c) if c >= 0x80 && unicode_ws_len(&src[i..]) == 0 => i += 1,
                _ => break,
            }
        }
        self.pos = i;
    }

    /// Skips tokens until an identifier whose keyword bit (`1 << kw as u8`) is
    /// set in `mask`, a [`TokKind::TripleSlash`] or the end of input. The
    /// recognizer's fast path: most tokens never leave the lexer.
    pub fn next_matching(&mut self, mask: u64) -> Token {
        self.next_matching_top(mask, 0)
    }

    /// [`Lexer::next_matching`], where keywords in `top_mask` also stop the
    /// scan when they occur at depth 0 (outside any bracket), and dynamic
    /// trigger words ([`Lexer::add_trigger_word`]) stop it anywhere.
    ///
    /// # Skim
    ///
    /// This is the extraction hot path. By default it runs the block skim
    /// (`skim.rs`): SIMD masks of "event" bytes (quotes, backtick, `/`,
    /// brackets, `?`, `:`, non-ASCII, and `!` / `<` where the dialect gives
    /// them meaning) and identifier bytes, 64 bytes at a time. Identifiers,
    /// whitespace and other operators between events are not tokenized; the
    /// previous significant token is reconstructed from the bytes before an
    /// event only when the event needs it (regex vs division, block vs
    /// object, `if (`, triggers). Ambiguous cases re-lex the run with the
    /// per-token path below, from the last point where the state is exact.
    ///
    /// The per-token reference path ([`Lexer::set_reference_skim`]) does not
    /// build a [`Token`] for tokens it skips. Whitespace, ASCII identifiers
    /// and the most frequent punctuators (`, ; ( ) [ ] .`) are handled
    /// inline: their state transitions (context stack, previous-token class
    /// and keyword, `case` labels) go through the same helpers as
    /// [`Lexer::next_token`] ([`Lexer::commit`], `open_paren`, `close_paren`,
    /// `close`), so regex/division and block/object decisions stay
    /// identical. Everything else is delegated to the full single-token
    /// lexer.
    ///
    /// Tests and the `lang_js_skim` fuzz target check that both paths return
    /// the same tokens as filtering the full token stream.
    #[inline(never)]
    pub fn next_matching_top(&mut self, mask: u64, top_mask: u64) -> Token {
        // Bit 0 (`Kw::None`) means "may be a dynamic trigger word"; it is never a
        // keyword.
        let mask = if self.dyn_filter.is_some() {
            mask | 1
        } else {
            mask & !1
        };
        let top_mask = top_mask & !1;
        if self.fast_skim {
            self.skim(mask, top_mask)
        } else {
            self.next_matching_reference(mask, top_mask)
        }
    }

    /// The per-token implementation of [`Lexer::next_matching_top`] (masks
    /// normalized).
    fn next_matching_reference(&mut self, mask: u64, top_mask: u64) -> Token {
        let src = self.src;
        // `done` implies JSX mode or the end of input (see `jsx_fail`), so it needs no
        // separate check. JSX mode can only be entered or re-entered by the full lexer
        // (`<` in expression position, a `}` closing a JSX expression container), so
        // `jsx_top` is checked on entry and after delegating.
        'outer: loop {
            while self.jsx_top {
                let t = self.next_token_inline();
                if self.is_match(&t, mask, top_mask) {
                    return t;
                }
            }
            loop {
                let start = self.pos;
                let Some(&b) = src.get(start) else {
                    return self.eof();
                };
                if IDENT_START_ASCII[b as usize] {
                    self.pos = start + 1;
                    self.skip_ident_tail();
                    let kw = if matches!(self.last, TokKind::Dot | TokKind::OptChain) {
                        Kw::None
                    } else {
                        keyword(&src[start..self.pos])
                    };
                    let bit = 1u64 << kw as u8;
                    let hit = (mask & bit != 0
                        && (kw != Kw::None || self.dyn_hit(start, self.pos)))
                        || (top_mask & bit != 0 && self.stack.is_empty());
                    let flags = if kw != Kw::None {
                        self.keyword_flags(kw)
                    } else {
                        0
                    };
                    if hit {
                        let depth = self.stack.len();
                        return self.tok(TokKind::Ident, start, depth, kw, flags);
                    }
                    self.commit(TokKind::Ident, kw);
                    continue;
                }
                match b {
                    b' ' | b'\t' => self.pos += 1,
                    b'\n' | b'\r' => {
                        self.pos += 1;
                        self.newline = true;
                    }
                    b',' => {
                        self.pos += 1;
                        self.commit(TokKind::Comma, Kw::None);
                    }
                    b';' => {
                        self.pos += 1;
                        self.commit(TokKind::Semi, Kw::None);
                    }
                    b'(' => {
                        self.pos += 1;
                        self.open_paren();
                        self.commit(TokKind::LParen, Kw::None);
                    }
                    b')' => {
                        self.pos += 1;
                        self.close_paren(start);
                        self.commit(TokKind::RParen, Kw::None);
                    }
                    b'[' => {
                        self.pos += 1;
                        self.push(CtxKind::Bracket);
                        self.commit(TokKind::LBracket, Kw::None);
                    }
                    b']' => {
                        self.pos += 1;
                        self.close(start, &[CtxKind::Bracket], CtxKind::Paren);
                        self.commit(TokKind::RBracket, Kw::None);
                    }
                    b'.' if !matches!(src.get(start + 1), Some(b'0'..=b'9' | b'.')) => {
                        self.pos += 1;
                        self.commit(TokKind::Dot, Kw::None);
                    }
                    b'{' => {
                        self.pos += 1;
                        self.open_brace();
                        self.commit(TokKind::LBrace, Kw::None);
                    }
                    b'}' if matches!(
                        self.top(),
                        Some(CtxKind::Block | CtxKind::TryBlock | CtxKind::Object)
                    ) =>
                    {
                        // Same as the fast path of `close_brace`.
                        let k = self.top();
                        self.pos += 1;
                        self.pop();
                        self.last_brace_block = k != Some(CtxKind::Object);
                        self.commit(TokKind::RBrace, Kw::None);
                    }
                    b':' => {
                        self.pos += 1;
                        self.colon();
                        self.commit(TokKind::Colon, Kw::None);
                    }
                    b'=' if !matches!(src.get(start + 1), Some(b'=' | b'>')) => {
                        self.pos += 1;
                        self.commit(TokKind::Assign, Kw::None);
                    }
                    b'?' if !matches!(src.get(start + 1), Some(b'.' | b'?')) => {
                        self.pos += 1;
                        self.ternary += 1;
                        self.commit(TokKind::Question, Kw::None);
                    }
                    _ => {
                        if let Some(t) = self.normal()
                            && self.is_match(&t, mask, top_mask)
                        {
                            return t;
                        }
                        if self.jsx_top {
                            continue 'outer;
                        }
                    }
                }
            }
        }
    }

    /// Whether a token from the full lexer stops [`Lexer::next_matching_top`].
    #[inline(always)]
    fn is_match(&self, t: &Token, mask: u64, top_mask: u64) -> bool {
        match t.kind {
            TokKind::Ident => {
                let bit = 1u64 << t.kw as u8;
                (mask & bit != 0 && (t.kw != Kw::None || self.is_dyn_trigger(t)))
                    || (top_mask & bit != 0 && t.depth == 0)
            }
            TokKind::Eof | TokKind::TripleSlash => true,
            _ => false,
        }
    }

    /// The dynamic-trigger filter for the identifier at `start..end`
    /// (non-empty, in range): first byte and length, plus the second byte
    /// for longer words.
    #[inline(always)]
    fn dyn_hit(&self, start: usize, end: usize) -> bool {
        match &self.dyn_filter {
            None => false,
            Some(f) => {
                let len = end - start;
                f[self.src[start] as usize] & (1 << len.min(15)) != 0
                    && (len == 1 || {
                        let s = self.src[start + 1];
                        self.dyn_second[s as usize / 64] & (1 << (s % 64)) != 0
                    })
            }
        }
    }

    #[inline(always)]
    fn ident(&mut self, start: usize) -> Token {
        self.pos = start + 1;
        self.skip_ident_tail();
        let kw = if matches!(self.last, TokKind::Dot | TokKind::OptChain) {
            Kw::None
        } else {
            keyword(&self.src[start..self.pos])
        };
        let mut flags = 0;
        if kw != Kw::None {
            flags = self.keyword_flags(kw);
        }
        let depth = self.stack.len();
        self.tok(TokKind::Ident, start, depth, kw, flags)
    }

    /// Bookkeeping for keywords: `case` labels, and [`TYPE_POS`] on TypeScript
    /// `import`.
    #[inline]
    fn keyword_flags(&mut self, kw: Kw) -> u8 {
        match kw {
            Kw::Case | Kw::Default => {
                self.case_pending = true;
                0
            }
            Kw::Import if self.dialect.ts => {
                let type_pos = match self.last {
                    TokKind::Colon => {
                        !self.last_colon_ternary
                            && !self.last_colon_case
                            && self.top() != Some(CtxKind::Object)
                    }
                    TokKind::Lt => true,
                    _ => false,
                };
                if type_pos { TYPE_POS } else { 0 }
            }
            _ => 0,
        }
    }

    #[inline(always)]
    fn number(&mut self, start: usize) -> Token {
        let src = self.src;
        let radix_prefix = src[start] == b'0'
            && matches!(
                src.get(start + 1),
                Some(b'x' | b'X' | b'o' | b'O' | b'b' | b'B')
            );
        let mut seen_dot = false;
        let mut i = start;
        while let Some(&c) = src.get(i) {
            match c {
                b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => i += 1,
                b'.' if !seen_dot && !radix_prefix => {
                    seen_dot = true;
                    i += 1;
                }
                b'+' | b'-'
                    if !radix_prefix && matches!(src.get(i.wrapping_sub(1)), Some(b'e' | b'E')) =>
                {
                    i += 1
                }
                _ => break,
            }
        }
        self.pos = i;
        let depth = self.stack.len();
        self.tok(TokKind::Number, start, depth, Kw::None, 0)
    }

    fn string(&mut self, start: usize, q: u8) -> Token {
        let src = self.src;
        let mut i = start + 1;
        let mut flags = 0;
        loop {
            // Most strings are short: `find_any3` has no setup cost, unlike `memchr3`.
            match find_any3(&src[i..], q, b'\\', b'\n') {
                None => {
                    self.err(start, src.len(), "unterminated string literal");
                    i = src.len();
                    flags |= UNTERMINATED;
                    break;
                }
                Some(off) => {
                    let j = i + off;
                    match src[j] {
                        b'\\' => {
                            flags |= ESCAPES;
                            i = if src.get(j + 1) == Some(&b'\r') && src.get(j + 2) == Some(&b'\n')
                            {
                                j + 3
                            } else {
                                j + 2
                            }
                            .min(src.len());
                        }
                        b'\n' => {
                            self.err(start, j, "unterminated string literal");
                            i = j;
                            flags |= UNTERMINATED;
                            break;
                        }
                        _ => {
                            i = j + 1;
                            break;
                        }
                    }
                }
            }
        }
        self.pos = i;
        let depth = self.stack.len();
        self.tok(TokKind::Str, start, depth, Kw::None, flags)
    }

    /// Scans template characters from `self.pos` (just past `` ` `` or `}`) and
    /// returns `end_kind` at the closing backtick or `subst_kind` at `${`
    /// (pushing a template context).
    fn template(
        &mut self,
        start: usize,
        depth: usize,
        end_kind: TokKind,
        subst_kind: TokKind,
    ) -> Token {
        let src = self.src;
        let mut i = self.pos;
        let mut flags = 0;
        loop {
            match memchr::memchr3(b'`', b'\\', b'$', &src[i..]) {
                None => {
                    self.err(start, src.len(), "unterminated template literal");
                    self.pos = src.len();
                    return self.tok(end_kind, start, depth, Kw::None, flags | UNTERMINATED);
                }
                Some(off) => {
                    let j = i + off;
                    match src[j] {
                        b'`' => {
                            self.pos = j + 1;
                            return self.tok(end_kind, start, depth, Kw::None, flags);
                        }
                        b'\\' => {
                            flags |= ESCAPES;
                            i = (j + 2).min(src.len());
                        }
                        _ => {
                            if src.get(j + 1) == Some(&b'{') {
                                self.pos = j + 2;
                                let t = self.tok(subst_kind, start, depth, Kw::None, flags);
                                self.push(CtxKind::Template);
                                return t;
                            }
                            i = j + 1;
                        }
                    }
                }
            }
        }
    }

    fn regex(&mut self, start: usize) -> Token {
        let src = self.src;
        let mut i = start + 1;
        let mut class = false;
        let mut flags = 0;
        loop {
            match src.get(i) {
                None | Some(b'\n' | b'\r') => {
                    self.err(start, i, "unterminated regular expression");
                    flags |= UNTERMINATED;
                    break;
                }
                Some(b'\\') => {
                    i += if matches!(src.get(i + 1), None | Some(b'\n' | b'\r')) {
                        1
                    } else {
                        2
                    }
                }
                Some(b'[') => {
                    class = true;
                    i += 1;
                }
                Some(b']') => {
                    class = false;
                    i += 1;
                }
                Some(b'/') if !class => {
                    i += 1;
                    while src.get(i).is_some_and(|&c| IDENT[c as usize] && c < 0x80) {
                        i += 1;
                    }
                    break;
                }
                Some(_) => i += 1,
            }
        }
        self.pos = i;
        let depth = self.stack.len();
        self.tok(TokKind::Regex, start, depth, Kw::None, flags)
    }

    /// Pops the context closed by a `)` or `]`. `want` are the matching kinds;
    /// when the top is `other` (the other bracket type) and the entry below
    /// matches, both are popped. Otherwise a stray closer is ignored.
    /// Returns the popped matching kind.
    #[inline(always)]
    fn close(&mut self, start: usize, want: &[CtxKind], other: CtxKind) -> Option<CtxKind> {
        match self.top() {
            Some(k) if want.contains(&k) => self.pop(),
            _ => self.close_mismatched(start, want, other),
        }
    }

    #[cold]
    #[inline(never)]
    fn close_mismatched(
        &mut self,
        start: usize,
        want: &[CtxKind],
        other: CtxKind,
    ) -> Option<CtxKind> {
        let n = self.stack.len();
        match self.top() {
            Some(k) if k == other && n >= 2 && want.contains(&self.stack[n - 2].kind) => {
                self.err(start, start + 1, "mismatched closing delimiter");
                self.pop();
                self.pop()
            }
            _ => {
                self.err(start, start + 1, "unbalanced closing delimiter");
                None
            }
        }
    }

    #[inline(always)]
    fn close_brace(&mut self, start: usize) -> Option<Token> {
        if let Some(k @ (CtxKind::Block | CtxKind::TryBlock | CtxKind::Object)) = self.top() {
            self.pos = start + 1;
            self.pop();
            self.last_brace_block = k != CtxKind::Object;
            let depth = self.stack.len();
            return Some(self.tok(TokKind::RBrace, start, depth, Kw::None, 0));
        }
        self.close_brace_slow(start)
    }

    #[inline(never)]
    fn close_brace_slow(&mut self, start: usize) -> Option<Token> {
        // Recover from unclosed `(` / `[` inside the braces.
        while matches!(
            self.top(),
            Some(CtxKind::Paren | CtxKind::ParenCond | CtxKind::Bracket)
        ) {
            self.err(start, start + 1, "mismatched closing delimiter");
            self.pop();
        }
        self.pos = start + 1;
        match self.top() {
            Some(CtxKind::Template) => {
                self.pop();
                let depth = self.stack.len();
                Some(self.template(start, depth, TokKind::TemplateTail, TokKind::TemplateMiddle))
            }
            Some(k) => {
                self.pop();
                self.last_brace_block = matches!(k, CtxKind::Block | CtxKind::TryBlock);
                let depth = self.stack.len();
                Some(self.tok(TokKind::RBrace, start, depth, Kw::None, 0))
            }
            None => {
                self.err(start, start + 1, "unbalanced closing delimiter");
                self.last_brace_block = true;
                Some(self.tok(TokKind::RBrace, start, 0, Kw::None, 0))
            }
        }
    }

    // ---- JSX ----

    /// Decides whether the `<` at `start` (in expression position) starts a JSX
    /// element.
    fn jsx_start(&self, start: usize) -> bool {
        let src = self.src;
        let mut i = start + 1;
        while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
            i += 1;
        }
        let Some(&b) = src.get(i) else { return false };
        if b == b'>' {
            return true;
        }
        if !IDENT_START[b as usize] {
            return false;
        }
        if self.dialect.ts {
            // `<T,>`, `<T extends U>`, `<const T>`: type parameters of a generic arrow.
            let name_start = i;
            while src.get(i).is_some_and(|&c| IDENT[c as usize]) {
                i += 1;
            }
            let name = &src[name_start..i];
            if name == b"const" {
                return false;
            }
            while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            match src.get(i) {
                // `<T,>` and `<T = D>`: TypeScript reads both as type parameters (a tag
                // name is never followed by `,` or `=`).
                Some(b',' | b'=') => return false,
                // `<T>(x: T) => R`: a generic function type (`f: <T>(x: T) => T` in an
                // interface, a type literal or an annotation). JSX text cannot contain a
                // raw `>`, so `<T>(...) =>` is never an element.
                Some(b'>') => return !generic_function_type(src, i + 1),
                Some(b'e') if src[i..].starts_with(b"extends") => {
                    let mut j = i + 7;
                    if src.get(j).is_some_and(|&c| IDENT[c as usize]) {
                        return true;
                    }
                    while src.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                        j += 1;
                    }
                    // `<T extends>` / `<T extends={..}>` / `<T extends/>`: an attribute.
                    return matches!(src.get(j), Some(b'=' | b'>' | b'/') | None);
                }
                _ => {}
            }
        }
        true
    }

    /// Enters the opening tag whose `<` is at `lt`. In TypeScript, the type
    /// arguments of the element (`<Select<Option, true> ...>`, directly
    /// after the tag name) are skipped here: they produce no tokens, like
    /// the rest of the tag outside its `{...}` containers.
    fn open_jsx_tag(&mut self, lt: usize) {
        self.pos = lt + 1;
        self.push(CtxKind::JsxTag);
        if !self.dialect.ts || self.jsx_type_args_failed {
            return;
        }
        let src = self.src;
        let mut i = lt + 1;
        while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
            i += 1;
        }
        let name = i;
        while src
            .get(i)
            .is_some_and(|&c| IDENT[c as usize] || matches!(c, b'.' | b':' | b'-'))
        {
            i += 1;
        }
        if i == name {
            return;
        }
        while src.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
            i += 1;
        }
        if src.get(i) != Some(&b'<') {
            return;
        }
        match jsx_type_args_end(src, i) {
            Some(end) => self.pos = end,
            // Not type arguments. The scan may have read to the end of input, so it is not
            // tried again: the total work stays linear.
            None => self.jsx_type_args_failed = true,
        }
    }

    fn jsx_fail(&mut self, at: usize, what: &str) {
        if self.dialect.jsx == Jsx::Heuristic {
            self.jsx_failed = true;
            self.done = true;
            // The pass is discarded; ending here keeps `done` cheap to check in the skim.
            self.pos = self.src.len();
        } else {
            self.err(at, at, what);
        }
    }

    /// After an element closed: continue in the enclosing JSX mode, or return
    /// to code with a value token.
    fn after_jsx_element(&mut self, start: usize) -> Option<Token> {
        match self.top() {
            Some(CtxKind::JsxChildren | CtxKind::JsxTag) => None,
            _ => {
                let depth = self.stack.len();
                Some(self.tok(TokKind::Jsx, start, depth, Kw::None, 0))
            }
        }
    }

    /// Scans an opening tag (name and attributes).
    fn jsx_tag(&mut self) -> Option<Token> {
        let src = self.src;
        loop {
            let Some(&b) = src.get(self.pos) else {
                self.jsx_fail(self.pos, "unterminated JSX tag");
                if !self.done {
                    self.done = true;
                }
                return None;
            };
            let start = self.pos;
            match b {
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => self.pos += 1,
                b'>' => {
                    self.pos += 1;
                    if let Some(top) = self.stack.last_mut() {
                        top.kind = CtxKind::JsxChildren;
                    }
                    return None;
                }
                b'/' => match src.get(start + 1) {
                    Some(b'>') => {
                        self.pos += 2;
                        self.pop();
                        return self.after_jsx_element(start);
                    }
                    Some(b'/') => {
                        self.pos =
                            memchr::memchr(b'\n', &src[start..]).map_or(src.len(), |i| start + i);
                    }
                    Some(b'*') => self.pos = block_comment_end(src, start + 2).unwrap_or(src.len()),
                    _ => {
                        self.pos += 1;
                        self.jsx_fail(start, "unexpected `/` in JSX tag");
                        if self.done {
                            return None;
                        }
                    }
                },
                b'{' => {
                    self.pos += 1;
                    let depth = self.stack.len();
                    let t = self.tok(TokKind::LBrace, start, depth, Kw::None, 0);
                    self.push(CtxKind::JsxAttr);
                    return Some(t);
                }
                b'"' | b'\'' => match memchr::memchr(b, &src[start + 1..]) {
                    Some(i) => self.pos = start + 1 + i + 1,
                    None => {
                        self.pos = src.len();
                        self.jsx_fail(start, "unterminated JSX attribute string");
                    }
                },
                // An element as attribute value.
                b'<' => self.open_jsx_tag(start),
                b'=' | b'.' | b':' | b'-' => self.pos += 1,
                _ if IDENT[b as usize] => {
                    self.pos += 1;
                    while src.get(self.pos).is_some_and(|&c| IDENT[c as usize]) {
                        self.pos += 1;
                    }
                }
                _ => {
                    self.pos += 1;
                    self.jsx_fail(start, "unexpected byte in JSX tag");
                    if self.done {
                        return None;
                    }
                }
            }
        }
    }

    /// Scans JSX children text up to a `{` container, a nested element or the
    /// closing tag.
    fn jsx_children(&mut self) -> Option<Token> {
        let src = self.src;
        let Some(off) = memchr::memchr2(b'<', b'{', &src[self.pos..]) else {
            self.pos = src.len();
            self.jsx_fail(self.pos, "unterminated JSX element");
            self.done = true;
            return None;
        };
        let j = self.pos + off;
        if src[j] == b'{' {
            self.pos = j + 1;
            let depth = self.stack.len();
            let t = self.tok(TokKind::LBrace, j, depth, Kw::None, 0);
            self.push(CtxKind::JsxChild);
            return Some(t);
        }
        let mut k = j + 1;
        while src.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
            k += 1;
        }
        if src.get(k) == Some(&b'/') {
            match memchr::memchr(b'>', &src[k..]) {
                Some(i) => {
                    self.pos = k + i + 1;
                    self.pop();
                    self.after_jsx_element(j)
                }
                None => {
                    self.pos = src.len();
                    self.jsx_fail(j, "unterminated JSX closing tag");
                    self.done = true;
                    None
                }
            }
        } else {
            self.open_jsx_tag(j);
            None
        }
    }
}
