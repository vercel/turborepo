//! Lexer and recursive-descent parser for the query language.
//!
//! ```text
//! query    := expr EOF
//! expr     := 'let' NAME '=' expr 'in' expr
//!           | primary (setop primary)*            -- left-associative, equal precedence
//! setop    := '+' | '-' | '^' | 'union' | 'except' | 'intersect'
//! primary  := '(' expr ')'
//!           | NAME '(' [expr (',' expr)*] ')'     -- function call
//!           | WORD | STRING                       -- literal: path, glob, pkg:name, number
//!           | '$' NAME                            -- variable
//! ```
//!
//! Words are runs of `[A-Za-z0-9_./*?@:~%!#|-]` (plus non-ASCII), `[...]`
//! classes and `{a,b}` groups (a comma inside braces belongs to the word). A
//! word may not start with `-`, so `a - b` is a difference and `my-file.ts` is
//! one word. Strings use `"` or `'` with `\` escapes and may contain anything.
//! `let`, `in`, `union`, `except` and `intersect` are keywords only where
//! the grammar expects them, so a file named `in` still parses as a literal.

use std::fmt;

/// A byte range in the query source.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct SrcSpan {
    /// Start byte.
    pub start: usize,
    /// End byte (exclusive).
    pub end: usize,
}

impl SrcSpan {
    fn to(self, other: SrcSpan) -> SrcSpan {
        SrcSpan {
            start: self.start,
            end: other.end,
        }
    }
}

/// A parse or evaluation error with a position in the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    /// Where.
    pub span: SrcSpan,
    /// What.
    pub message: String,
    /// Optional suggestion.
    pub hint: Option<String>,
}

impl QueryError {
    pub(crate) fn new(span: SrcSpan, message: impl Into<String>) -> Self {
        QueryError {
            span,
            message: message.into(),
            hint: None,
        }
    }

    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Renders the error against its source with a caret line:
    ///
    /// ```text
    /// error: expected `,` or `)`, found end of input
    ///   --> 1:17
    ///    |
    ///  1 | deps(src/a.ts, 2
    ///    |                 ^
    /// ```
    pub fn render(&self, src: &str) -> String {
        let start = self.span.start.min(src.len());
        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = src[start..].find('\n').map_or(src.len(), |i| start + i);
        let line_no = src[..line_start].matches('\n').count() + 1;
        let col = src[line_start..start].chars().count();
        let end = self.span.end.clamp(start, line_end);
        let width = src[start..end].chars().count().max(1);
        let gutter = line_no.to_string().len();
        let pad = " ".repeat(gutter);
        let mut out = format!(
            "error: {}\n{pad}--> {}:{}\n{pad} |\n{line_no} | {}\n{pad} | {}{}",
            self.message,
            line_no,
            col + 1,
            &src[line_start..line_end],
            " ".repeat(col),
            "^".repeat(width)
        );
        if let Some(h) = &self.hint {
            out.push_str(&format!("\n{pad} = hint: {h}"));
        }
        out
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.span.start)?;
        if let Some(h) = &self.hint {
            write!(f, " (hint: {h})")?;
        }
        Ok(())
    }
}

impl std::error::Error for QueryError {}

/// Binary set operators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SetOp {
    /// `+` / `union`.
    Union,
    /// `-` / `except`.
    Difference,
    /// `^` / `intersect`.
    Intersection,
}

/// A parsed expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// Source range.
    pub span: SrcSpan,
    /// The node.
    pub kind: ExprKind,
}

/// Expression variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// An unquoted word or quoted string.
    Literal {
        /// Text (unescaped).
        text: String,
        /// Whether it was quoted.
        quoted: bool,
    },
    /// `$name`.
    Var(String),
    /// `name(args)`.
    Call {
        /// Function name.
        name: String,
        /// Span of the name.
        name_span: SrcSpan,
        /// Arguments.
        args: Vec<Expr>,
    },
    /// `lhs op rhs`.
    Binary {
        /// Operator.
        op: SetOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// `let name = value in body`.
    Let {
        /// Bound name.
        name: String,
        /// Bound value.
        value: Box<Expr>,
        /// Body.
        body: Box<Expr>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Str(String),
    Var(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Caret,
    Eq,
    Eof,
}

impl Tok {
    fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("`{w}`"),
            Tok::Str(s) => format!("string {s:?}"),
            Tok::Var(v) => format!("`${v}`"),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::Comma => "`,`".into(),
            Tok::Plus => "`+`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Caret => "`^`".into(),
            Tok::Eq => "`=`".into(),
            Tok::Eof => "end of input".into(),
        }
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'_' | b'.'
                | b'/'
                | b'*'
                | b'?'
                | b'@'
                | b':'
                | b'~'
                | b'%'
                | b'!'
                | b'#'
                | b'|'
                | b'-'
        )
        || b >= 0x80
}

// Invariant: `i` is a char boundary inside the string.
#[allow(clippy::expect_used)]
fn lex(src: &str) -> Result<Vec<(Tok, SrcSpan)>, QueryError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let single = match c {
            b'(' => Some(Tok::LParen),
            b')' => Some(Tok::RParen),
            b',' => Some(Tok::Comma),
            b'+' => Some(Tok::Plus),
            b'-' => Some(Tok::Minus),
            b'^' => Some(Tok::Caret),
            b'=' => Some(Tok::Eq),
            _ => None,
        };
        if let Some(t) = single {
            i += 1;
            out.push((t, SrcSpan { start, end: i }));
            continue;
        }
        match c {
            b'"' | b'\'' => {
                let quote = c;
                i += 1;
                let mut s = String::new();
                loop {
                    let Some(&ch) = b.get(i) else {
                        return Err(QueryError::new(
                            SrcSpan { start, end: i },
                            "unterminated string",
                        )
                        .with_hint(format!("close it with {}", quote as char)));
                    };
                    if ch == quote {
                        i += 1;
                        break;
                    }
                    if ch == b'\\' && i + 1 < b.len() {
                        let next = src[i + 1..].chars().next().expect("char");
                        s.push(next);
                        i += 1 + next.len_utf8();
                        continue;
                    }
                    let ch = src[i..].chars().next().expect("char");
                    s.push(ch);
                    i += ch.len_utf8();
                }
                out.push((Tok::Str(s), SrcSpan { start, end: i }));
            }
            b'$' => {
                i += 1;
                let name_start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if i == name_start {
                    return Err(QueryError::new(
                        SrcSpan { start, end: i },
                        "expected a variable name after `$`",
                    ));
                }
                out.push((
                    Tok::Var(src[name_start..i].to_owned()),
                    SrcSpan { start, end: i },
                ));
            }
            _ if is_word_byte(c) || c == b'[' || c == b'{' => {
                let mut braces = 0usize;
                let mut class = false;
                while i < b.len() {
                    let ch = b[i];
                    let ok = if class {
                        if ch == b']' {
                            class = false;
                        }
                        true
                    } else {
                        match ch {
                            b'[' => {
                                class = true;
                                true
                            }
                            b'{' => {
                                braces += 1;
                                true
                            }
                            b'}' if braces > 0 => {
                                braces -= 1;
                                true
                            }
                            b',' => braces > 0,
                            _ => is_word_byte(ch),
                        }
                    };
                    if !ok {
                        break;
                    }
                    i += 1;
                }
                if braces > 0 || class {
                    return Err(QueryError::new(
                        SrcSpan { start, end: i },
                        if class {
                            "unclosed `[` in word"
                        } else {
                            "unclosed `{` in word"
                        },
                    )
                    .with_hint("quote the pattern if it contains spaces or parentheses"));
                }
                out.push((
                    Tok::Word(src[start..i].to_owned()),
                    SrcSpan { start, end: i },
                ));
            }
            _ => {
                let ch = src[i..].chars().next().expect("char");
                return Err(QueryError::new(
                    SrcSpan {
                        start,
                        end: i + ch.len_utf8(),
                    },
                    format!("unexpected character `{ch}`"),
                )
                .with_hint("quote literals that contain special characters"));
            }
        }
    }
    out.push((
        Tok::Eof,
        SrcSpan {
            start: b.len(),
            end: b.len(),
        },
    ));
    Ok(out)
}

struct Parser {
    toks: Vec<(Tok, SrcSpan)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn span(&self) -> SrcSpan {
        self.toks[self.pos].1
    }

    fn bump(&mut self) -> (Tok, SrcSpan) {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn peek_at(&self, k: usize) -> &Tok {
        &self.toks[(self.pos + k).min(self.toks.len() - 1)].0
    }

    fn unexpected(&self, expected: &str) -> QueryError {
        QueryError::new(
            self.span(),
            format!("expected {expected}, found {}", self.peek().describe()),
        )
    }

    fn expr(&mut self) -> Result<Expr, QueryError> {
        if matches!(self.peek(), Tok::Word(w) if w == "let")
            && matches!(self.peek_at(1), Tok::Word(_))
            && matches!(self.peek_at(2), Tok::Eq)
        {
            let (_, start) = self.bump();
            let (Tok::Word(name), name_span) = self.bump() else {
                unreachable!()
            };
            if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(
                    QueryError::new(name_span, format!("invalid variable name `{name}`"))
                        .with_hint("use letters, digits and `_`"),
                );
            }
            self.bump(); // `=`
            let value = self.expr()?;
            match self.peek() {
                Tok::Word(w) if w == "in" => {
                    self.bump();
                }
                _ => return Err(self.unexpected("`in` after the `let` value")),
            }
            let body = self.expr()?;
            return Ok(Expr {
                span: start.to(body.span),
                kind: ExprKind::Let {
                    name,
                    value: Box::new(value),
                    body: Box::new(body),
                },
            });
        }
        let mut lhs = self.primary()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => SetOp::Union,
                Tok::Minus => SetOp::Difference,
                Tok::Caret => SetOp::Intersection,
                Tok::Word(w) if w == "union" => SetOp::Union,
                Tok::Word(w) if w == "except" => SetOp::Difference,
                Tok::Word(w) if w == "intersect" => SetOp::Intersection,
                _ => break,
            };
            self.bump();
            let rhs = self.primary_or_let()?;
            lhs = Expr {
                span: lhs.span.to(rhs.span),
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        Ok(lhs)
    }

    /// The right operand of a set operator: a primary, or a `let` that extends
    /// to the end.
    fn primary_or_let(&mut self) -> Result<Expr, QueryError> {
        if matches!(self.peek(), Tok::Word(w) if w == "let") && matches!(self.peek_at(2), Tok::Eq) {
            self.expr()
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Result<Expr, QueryError> {
        let span = self.span();
        match self.peek().clone() {
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                if *self.peek() != Tok::RParen {
                    return Err(self
                        .unexpected("`)`")
                        .with_hint(format!("the `(` at byte {} is not closed", span.start)));
                }
                let (_, end) = self.bump();
                Ok(Expr {
                    span: span.to(end),
                    kind: inner.kind,
                })
            }
            Tok::Word(w) => {
                self.bump();
                if *self.peek() == Tok::LParen {
                    if !w.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        return Err(
                            QueryError::new(span, format!("`{w}` is not a function name"))
                                .with_hint("quote literals that are followed by `(`"),
                        );
                    }
                    let open = self.span();
                    self.bump();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            args.push(self.expr()?);
                            match self.peek() {
                                Tok::Comma => {
                                    self.bump();
                                }
                                Tok::RParen => break,
                                Tok::Eof => {
                                    return Err(self.unexpected("`,` or `)`").with_hint(format!(
                                        "the `(` of `{w}` at byte {} is not closed",
                                        open.start
                                    )));
                                }
                                _ => return Err(self.unexpected("`,` or `)` after an argument")),
                            }
                        }
                    }
                    let (_, end) = self.bump();
                    Ok(Expr {
                        span: span.to(end),
                        kind: ExprKind::Call {
                            name: w,
                            name_span: span,
                            args,
                        },
                    })
                } else {
                    Ok(Expr {
                        span,
                        kind: ExprKind::Literal {
                            text: w,
                            quoted: false,
                        },
                    })
                }
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr {
                    span,
                    kind: ExprKind::Literal {
                        text: s,
                        quoted: true,
                    },
                })
            }
            Tok::Var(v) => {
                self.bump();
                Ok(Expr {
                    span,
                    kind: ExprKind::Var(v),
                })
            }
            Tok::Minus => Err(self
                .unexpected("an expression")
                .with_hint("a word cannot start with `-`; quote it, e.g. \"-x.ts\"")),
            _ => Err(self.unexpected("an expression")),
        }
    }
}

/// Parses a query.
pub fn parse(src: &str) -> Result<Expr, QueryError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    if *p.peek() == Tok::Eof {
        return Err(p.unexpected("an expression"));
    }
    let e = p.expr()?;
    if *p.peek() != Tok::Eof {
        let hint = match p.peek() {
            Tok::RParen => "this `)` has no matching `(`",
            Tok::Word(_) | Tok::Str(_) | Tok::Var(_) | Tok::LParen => {
                "combine sets with `+`, `-` or `^`"
            }
            _ => "remove the trailing input",
        };
        return Err(p.unexpected("end of input").with_hint(hint));
    }
    Ok(e)
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Literal { text, quoted } => {
                if *quoted {
                    write!(f, "{text:?}")
                } else {
                    f.write_str(text)
                }
            }
            ExprKind::Var(v) => write!(f, "${v}"),
            ExprKind::Call { name, args, .. } => {
                write!(f, "{name}(")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{a}")?;
                }
                f.write_str(")")
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let o = match op {
                    SetOp::Union => "+",
                    SetOp::Difference => "-",
                    SetOp::Intersection => "^",
                };
                write!(f, "({lhs} {o} {rhs})")
            }
            ExprKind::Let { name, value, body } => write!(f, "(let {name} = {value} in {body})"),
        }
    }
}
