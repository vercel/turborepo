//! Byte classification tables and keyword lookup.

/// Bytes that may continue an identifier: ASCII letters, digits, `$`, `_`, `\`
/// (unicode escapes) and every non-ASCII byte (non-ASCII identifier characters
/// are not validated; the few non-ASCII whitespace characters are split off by
/// [`unicode_ws_len`]).
pub(crate) static IDENT: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        t[i] = b.is_ascii_alphanumeric() || b == b'$' || b == b'_' || b == b'\\' || b >= 0x80;
        i += 1;
    }
    t
};

/// Identifier continuation class: 0 = no, 1 = ASCII identifier byte, 2 =
/// non-ASCII byte (an identifier byte unless it starts non-ASCII whitespace).
pub(crate) static IDENT_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = if i >= 0x80 {
            2
        } else if IDENT[i] {
            1
        } else {
            0
        };
        i += 1;
    }
    t
};

/// Bytes that may start an identifier.
pub(crate) static IDENT_START: [bool; 256] = {
    let mut t = IDENT;
    let mut d = b'0';
    while d <= b'9' {
        t[d as usize] = false;
        d += 1;
    }
    t
};

/// ASCII bytes that start an identifier (`a-z A-Z $ _ \`); non-ASCII starts are
/// handled separately because they may be whitespace.
pub(crate) static IDENT_START_ASCII: [bool; 256] = {
    let mut t = IDENT_START;
    let mut i = 0x80;
    while i < 256 {
        t[i] = false;
        i += 1;
    }
    t
};

/// Length of the non-ASCII whitespace or line terminator at the start of `s`
/// (NBSP, BOM, U+1680, U+2000..U+200A, U+2028/U+2029, U+202F, U+205F, U+3000),
/// or 0.
#[inline]
pub(crate) fn unicode_ws_len(s: &[u8]) -> usize {
    match s {
        [0xC2, 0xA0, ..] => 2,
        [0xEF, 0xBB, 0xBF, ..] => 3,
        [0xE1, 0x9A, 0x80, ..] => 3,
        [0xE2, 0x80, 0x80..=0x8A | 0xA8 | 0xA9 | 0xAF, ..] => 3,
        [0xE2, 0x81, 0x9F, ..] => 3,
        [0xE3, 0x80, 0x80, ..] => 3,
        _ => 0,
    }
}

/// Keywords and contextual words the lexer or the recognizer care about.
/// Everything else is [`Kw::None`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Kw {
    #[default]
    None,
    // Module syntax.
    Import,
    Export,
    Require,
    Module,
    From,
    As,
    Type,
    Defer,
    Source,
    // Declarations.
    Const,
    Let,
    Var,
    Function,
    Class,
    // Keywords after which an expression (so a regex) starts.
    Return,
    Typeof,
    Case,
    Do,
    Else,
    In,
    Instanceof,
    New,
    Delete,
    Void,
    Throw,
    Yield,
    Await,
    Of,
    Extends,
    Default,
    // Keywords whose parenthesized head is followed by a statement.
    If,
    While,
    For,
    With,
    // Blocks.
    Try,
    // Dynamic code.
    Eval,
}

impl Kw {
    /// Whether an expression (and so a regex literal) may follow this keyword.
    #[inline]
    pub(crate) fn expr_follows(self) -> bool {
        matches!(
            self,
            Kw::Return
                | Kw::Typeof
                | Kw::Case
                | Kw::Do
                | Kw::Else
                | Kw::In
                | Kw::Instanceof
                | Kw::New
                | Kw::Delete
                | Kw::Void
                | Kw::Throw
                | Kw::Yield
                | Kw::Await
                | Kw::Of
                | Kw::Extends
                | Kw::Default
        )
    }

    /// Whether a `{` after this keyword opens an object literal (an expression)
    /// rather than a block.
    #[inline]
    pub(crate) fn object_follows(self) -> bool {
        self.expr_follows() && !matches!(self, Kw::Do | Kw::Else)
    }
}

const KEYWORDS: &[&str] = &[
    "import",
    "export",
    "require",
    "module",
    "from",
    "as",
    "type",
    "defer",
    "source",
    "const",
    "let",
    "var",
    "function",
    "class",
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "delete",
    "void",
    "throw",
    "yield",
    "await",
    "of",
    "extends",
    "default",
    "if",
    "while",
    "for",
    "with",
    "try",
    "eval",
];

/// The source text of a keyword (empty for [`Kw::None`]).
pub(crate) fn keyword_text(kw: Kw) -> &'static [u8] {
    match kw {
        Kw::None => b"",
        k => KEYWORDS[k as usize - 1].as_bytes(),
    }
}

/// Skim byte classes (see `skim.rs`).
pub(crate) const SK_ID: u8 = 1;
/// ASCII whitespace the lexer skips (` `, `\t`, `\n`, `\r`, VT, FF).
pub(crate) const SK_WS: u8 = 2;
/// Line terminators (`\n`, `\r`).
pub(crate) const SK_NL: u8 = 4;
/// `0-9`.
pub(crate) const SK_DIGIT: u8 = 8;
/// Last bytes of `if`, `while`, `for`, `with`, `await` (the keywords that
/// change how `(` is classified).
pub(crate) const SK_COND_LAST: u8 = 16;

pub(crate) static SKIM: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        let mut c = 0;
        if IDENT[i] && b < 0x80 {
            c |= SK_ID;
        }
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            c |= SK_WS;
        }
        if matches!(b, b'\n' | b'\r') {
            c |= SK_NL;
        }
        if b.is_ascii_digit() {
            c |= SK_DIGIT;
        }
        if matches!(b, b'f' | b'e' | b'r' | b'h' | b't') {
            c |= SK_COND_LAST;
        }
        t[i] = c;
        i += 1;
    }
    t
};

static KW_FIRST: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < KEYWORDS.len() {
        let k = KEYWORDS[i].as_bytes();
        t[k[0] as usize] |= 1 << k.len();
        i += 1;
    }
    t
};

static KW_LAST: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < KEYWORDS.len() {
        let k = KEYWORDS[i].as_bytes();
        t[k[k.len() - 1] as usize] |= 1 << k.len();
        i += 1;
    }
    t
};

/// Classifies an identifier. Two table loads reject almost every non-keyword;
/// the rest dispatches on length and compares at most a few candidates.
#[inline(always)]
pub(crate) fn keyword(s: &[u8]) -> Kw {
    // Bit `len` of `KW_FIRST[first byte]` (`KW_LAST[last byte]`) is set iff some
    // keyword of that length starts (ends) with that byte.
    let (Some(&first), Some(&last), len) = (s.first(), s.last(), s.len()) else {
        return Kw::None;
    };
    if len > 10 || KW_FIRST[first as usize] & KW_LAST[last as usize] & (1 << len) == 0 {
        return Kw::None;
    }
    keyword_slow(s)
}

#[inline(never)]
fn keyword_slow(s: &[u8]) -> Kw {
    match s.len() {
        2 => match s {
            b"as" => Kw::As,
            b"do" => Kw::Do,
            b"if" => Kw::If,
            b"in" => Kw::In,
            b"of" => Kw::Of,
            _ => Kw::None,
        },
        3 => match s {
            b"for" => Kw::For,
            b"let" => Kw::Let,
            b"new" => Kw::New,
            b"try" => Kw::Try,
            b"var" => Kw::Var,
            _ => Kw::None,
        },
        4 => match s {
            b"case" => Kw::Case,
            b"else" => Kw::Else,
            b"eval" => Kw::Eval,
            b"from" => Kw::From,
            b"type" => Kw::Type,
            b"void" => Kw::Void,
            b"with" => Kw::With,
            _ => Kw::None,
        },
        5 => match s {
            b"await" => Kw::Await,
            b"class" => Kw::Class,
            b"const" => Kw::Const,
            b"defer" => Kw::Defer,
            b"throw" => Kw::Throw,
            b"while" => Kw::While,
            b"yield" => Kw::Yield,
            _ => Kw::None,
        },
        6 => match s {
            b"delete" => Kw::Delete,
            b"export" => Kw::Export,
            b"import" => Kw::Import,
            b"module" => Kw::Module,
            b"return" => Kw::Return,
            b"source" => Kw::Source,
            b"typeof" => Kw::Typeof,
            _ => Kw::None,
        },
        7 => match s {
            b"default" => Kw::Default,
            b"extends" => Kw::Extends,
            b"require" => Kw::Require,
            _ => Kw::None,
        },
        8 => match s {
            b"function" => Kw::Function,
            _ => Kw::None,
        },
        10 => match s {
            b"instanceof" => Kw::Instanceof,
            _ => Kw::None,
        },
        _ => Kw::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_keyword_round_trips() {
        for (s, k) in [
            ("import", Kw::Import),
            ("export", Kw::Export),
            ("require", Kw::Require),
            ("module", Kw::Module),
            ("from", Kw::From),
            ("as", Kw::As),
            ("type", Kw::Type),
            ("defer", Kw::Defer),
            ("source", Kw::Source),
            ("const", Kw::Const),
            ("let", Kw::Let),
            ("var", Kw::Var),
            ("function", Kw::Function),
            ("class", Kw::Class),
            ("return", Kw::Return),
            ("typeof", Kw::Typeof),
            ("case", Kw::Case),
            ("do", Kw::Do),
            ("else", Kw::Else),
            ("in", Kw::In),
            ("instanceof", Kw::Instanceof),
            ("new", Kw::New),
            ("delete", Kw::Delete),
            ("void", Kw::Void),
            ("throw", Kw::Throw),
            ("yield", Kw::Yield),
            ("await", Kw::Await),
            ("of", Kw::Of),
            ("extends", Kw::Extends),
            ("default", Kw::Default),
            ("if", Kw::If),
            ("while", Kw::While),
            ("for", Kw::For),
            ("with", Kw::With),
            ("try", Kw::Try),
            ("eval", Kw::Eval),
        ] {
            assert_eq!(keyword(s.as_bytes()), k, "{s}");
            assert_eq!(keyword_text(k), s.as_bytes());
        }
        for s in [
            "",
            "a",
            "x",
            "Import",
            "imports",
            "requir",
            "typeofx",
            "instanceofs",
        ] {
            assert_eq!(keyword(s.as_bytes()), Kw::None, "{s}");
        }
    }
}
