//! The JS partial evaluator over call arguments.
//!
//! A Pratt-lite parser over the tokens of one expression (usually a call
//! argument) computes a [`JsVal`]: a string value from
//! [`turborepo_trace_core::eval::Value`] (known, pattern with wildcards, capped
//! alternatives, unknown), a path anchored at the file's directory or at the
//! process working directory, a file URL, or one of the few objects and
//! functions that build paths. Understood:
//!
//! - string and template literals (substitutions are evaluated; unknown ones
//!   become wildcards), `String.raw` tagged templates, `+` concatenation,
//!   numbers;
//! - `a ? b : c`, `a || b`, `a ?? b` (alternatives, capped at
//!   [`turborepo_trace_core::eval::MAX_ALTERNATIVES`]), `a && b` (`b`),
//!   parentheses and the comma operator (`(0, path.join)(...)`);
//! - top-level `const NAME = ...` values (collected by the recognizer);
//! - `__dirname`, `__filename`, `import.meta.url` / `dirname` / `filename`,
//!   `process.cwd()`, `process.env.X` (unknown, marked env-dependent);
//! - `path.join/resolve/dirname/basename/extname/relative/normalize` (also
//!   `path.posix` and destructured or renamed imports of `path` / `node:path`),
//!   `fileURLToPath`, `pathToFileURL`, `new URL(x, import.meta.url)` and its
//!   `.pathname` / `.href`, `require.resolve('./x')` (a path next to the file),
//!   `[a, b].join(sep)`;
//! - `require.resolve('pkg')` / `import.meta.resolve('pkg')` and the paths and
//!   URLs built on them ([`JsVal::Resolved`]: a path next to a module only the
//!   resolver can find). Where the search starts when that is not the file
//!   ([`Start`]: `{ paths }`, `createRequire`) is read by the value-flow engine
//!   ([`resolve_from`]).
//!
//! Everything else is unknown, with its source text kept as the wildcard
//! reason. Parsing is bounded: the recognizer caps the token count and the
//! nesting depth is capped here.

use std::{borrow::Cow, collections::HashMap};

use turborepo_trace_core::{
    Base, Origin,
    eval::{MAX_ALTERNATIVES, PatPart, Value, path},
    lexkit::{EscapeRules, decode},
};

use super::{
    api::{ApiBinding, BaseRule, Loader},
    origin,
};
use crate::lexer::{Kw, TokKind, Token, UNTERMINATED};

/// Maximum length of a known string; longer values become unknown.
pub const MAX_STR: usize = 4096;
/// Maximum expression nesting.
const MAX_NEST: u32 = 64;
/// Maximum length of a wildcard reason.
const MAX_REASON: usize = 120;
/// Maximum number of call arguments or array elements considered.
const MAX_LIST: usize = 16;

/// The [`Base::Language`] anchor of a lowered [`JsVal::Resolved`]: the
/// specifier is the text of a [`Located`] module followed by `/<relative path>`
/// (`require.resolve('m')/x`), and the JS resolver finds the module first
/// ([`Located::parse`]).
pub const RESOLVED_BASE: &str = "require.resolve";

/// Maximum nesting of [`Start::Next`] in a [`Located`] module.
const MAX_LOCATED_DEPTH: usize = 3;

/// A module found by a resolver call: `require.resolve(m)` or
/// `import.meta.resolve(m)`, from the file or, with a [`Start`], from another
/// place (`require.resolve(m, { paths })`, a `require` made by
/// `createRequire(elsewhere)`).
///
/// Its text form is what a lowered reference carries and what the JS resolver
/// parses:
///
/// ```text
/// located := ("require.resolve" | "import.meta.resolve") "('" module "'" [", from " start] ")"
/// start   := "'" directory "'"  |  located ["/'" path "'"]
/// ```
///
/// No part contains a `'`, so the text has one reading ([`Located::new`] and
/// [`Located::from_start`] refuse anything else).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Located {
    /// The request: a package specifier (`pkg`, `@scope/pkg/sub`) or a `#`
    /// import.
    module: Box<str>,
    /// Found by `import.meta.resolve` (the ESM resolver and its conditions),
    /// not by `require.resolve`.
    esm: bool,
    /// Where the search starts when that is not the file's own directory.
    from: Option<Box<Start>>,
}

/// Where a module search starts instead of the directory of the file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Start {
    /// A directory given relative to the file's directory (normalized, never
    /// empty: the file's own directory is no start).
    Dir(Box<str>),
    /// The path joined onto the file another module was found at (`..` is that
    /// file's directory; the empty path is the file). Node searches from a
    /// file as from its directory.
    Next(Located, Box<str>),
}

/// Whether `s` can be a part of a [`Located`] text.
fn plain_part(s: &str) -> bool {
    !s.contains('\'') && !s.contains('\n') && s.len() <= MAX_STR
}

impl Located {
    /// The module `request` as `require.resolve` (or, with `esm`,
    /// `import.meta.resolve`) finds it from the file. `None` unless
    /// `request` names a package or a `#` import: a path, a URL or a
    /// builtin is not found through `node_modules`.
    pub fn new(request: &str, esm: bool) -> Option<Located> {
        let ok = !request.is_empty()
            && !request.starts_with('.')
            && !request.starts_with('/')
            && !request.contains(':')
            && plain_part(request);
        ok.then(|| Located {
            module: request.into(),
            esm,
            from: None,
        })
    }

    /// The package `request` as `require.resolve` finds it searching from
    /// `start`. A `#` import is not accepted: Node resolves it through the
    /// package of the calling module, wherever the search starts.
    pub fn from_start(request: &str, start: Start) -> Option<Located> {
        let mut l = Located::new(request, false)?;
        if request.starts_with('#') || start.depth() >= MAX_LOCATED_DEPTH {
            return None;
        }
        l.from = match start {
            Start::Dir(d) if d.is_empty() => None,
            Start::Dir(d) if !plain_part(&d) => return None,
            Start::Next(_, rel) if !plain_part(&rel) => return None,
            s => Some(Box::new(s)),
        };
        Some(l)
    }

    /// The request.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Whether `import.meta.resolve` found it (ESM conditions).
    pub fn esm(&self) -> bool {
        self.esm
    }

    /// Where the search starts, when not at the file.
    pub fn start(&self) -> Option<&Start> {
        self.from.as_deref()
    }

    /// Parses the text form at the start of `s`: the module and the rest of
    /// `s`.
    pub fn parse(s: &str) -> Option<(Located, &str)> {
        Located::parse_at(s, 0)
    }

    fn parse_at(s: &str, depth: usize) -> Option<(Located, &str)> {
        if depth > MAX_LOCATED_DEPTH {
            return None;
        }
        let (esm, rest) = match s.strip_prefix("require.resolve('") {
            Some(r) => (false, r),
            None => (true, s.strip_prefix("import.meta.resolve('")?),
        };
        let (module, rest) = rest.split_once('\'')?;
        let mut l = Located::new(module, esm)?;
        if let Some(rest) = rest.strip_prefix(')') {
            return Some((l, rest));
        }
        let rest = rest.strip_prefix(", from ")?;
        let (start, rest) = match rest.strip_prefix('\'') {
            Some(r) => {
                let (dir, r) = r.split_once('\'')?;
                (Start::Dir(dir.into()), r)
            }
            None => {
                let (inner, r) = Located::parse_at(rest, depth + 1)?;
                match r.strip_prefix("/'") {
                    Some(r) => {
                        let (rel, r) = r.split_once('\'')?;
                        (Start::Next(inner, rel.into()), r)
                    }
                    None => (Start::Next(inner, "".into()), r),
                }
            }
        };
        if esm || module.starts_with('#') || matches!(&start, Start::Dir(d) if d.is_empty()) {
            return None;
        }
        l.from = Some(Box::new(start));
        Some((l, rest.strip_prefix(')')?))
    }
}

impl Start {
    fn depth(&self) -> usize {
        match self {
            Start::Dir(_) => 0,
            Start::Next(l, _) => 1 + l.from.as_ref().map_or(0, |s| s.depth()),
        }
    }
}

impl std::fmt::Display for Located {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let head = if self.esm {
            "import.meta.resolve"
        } else {
            "require.resolve"
        };
        write!(f, "{head}('{}'", self.module)?;
        match self.from.as_deref() {
            None => {}
            Some(Start::Dir(d)) => write!(f, ", from '{d}'")?,
            Some(Start::Next(l, rel)) if rel.is_empty() => write!(f, ", from {l}")?,
            Some(Start::Next(l, rel)) => write!(f, ", from {l}/'{rel}'")?,
        }
        f.write_str(")")
    }
}

/// What an anchored path is relative to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// The directory of the file (`__dirname`, `import.meta.url`).
    FileDir,
    /// The process working directory (`process.cwd()`, `path.resolve('x')`).
    Cwd,
    /// The system's temporary directory (`os.tmpdir()`, `mkdtemp`, temp-file
    /// helpers): outside the repository, so paths below it are no
    /// references. Only the value-flow engine ([`crate::flow`]) produces
    /// it.
    Tmp,
}

/// Objects with members the evaluator understands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Obj {
    /// The `path` module (or `path.posix`).
    Path,
    /// The `url` module.
    UrlModule,
    /// `process`.
    Process,
    /// `process.env`.
    ProcessEnv,
    /// `import.meta`.
    ImportMeta,
    /// `require` (or an alias / `createRequire` result).
    Require,
    /// `String`.
    StringCtor,
    /// `URL`.
    UrlCtor,
}

/// Functions the evaluator can apply.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Func {
    PathJoin,
    PathResolve,
    PathDirname,
    PathBasename,
    PathExtname,
    PathRelative,
    PathNormalize,
    FileUrlToPath,
    PathToFileUrl,
    ProcessCwd,
    /// `require.resolve`.
    Resolve,
    /// `import.meta.resolve`: the ESM resolver; the result is a `file:` URL
    /// string.
    MetaResolve,
    /// `String.raw`.
    StringRaw,
    /// The `resolve-from` package: `resolveFrom(dir, './x')`.
    ResolveFrom,
    /// `<unknown>.join` called with an anchored path first: assumed to be
    /// `path.join` (bundler namespace objects such as webpack's
    /// `path__WEBPACK_IMPORTED_MODULE_0___default()`).
    GuessJoin,
    /// `<unknown>.resolve`, likewise assumed to be `path.resolve`.
    GuessResolve,
}

/// The abstract value of an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsVal {
    /// A string that is not anchored (a module specifier, or a path relative to
    /// the cwd if used as one).
    Str(Value),
    /// An absolute path: the anchor directory joined with a relative path. The
    /// relative part may still contain `.`/`..` segments and a leading `/`;
    /// [`normalize_value`] canonicalizes it.
    Path(Anchor, Value),
    /// `__filename` when the file name is not known.
    File,
    /// `import.meta.url`.
    FileUrl,
    /// A `file:` URL object or href for an anchored path.
    Url(Anchor, Value),
    /// A path relative to the file a resolver call returns for a package
    /// specifier (`path.join(require.resolve('pkg'), '../x.wasm')`), as a
    /// path string, a `file:` URL string or a URL object: the module and
    /// the relative part. The relative part is joined onto the resolved
    /// file path, so `..` is its directory; it is a path, not
    /// text (no string is appended to it without a `/`), and a trailing `/`
    /// marks a directory URL, as in [`JsVal::Url`].
    Resolved(Located, Value),
    /// Alternatives that cannot be merged into one [`JsVal::Str`] or
    /// [`JsVal::Path`].
    Alts(Vec<JsVal>),
    /// A known object.
    Obj(Obj),
    /// A known function.
    Func(Func),
    /// An array literal.
    Arr(Vec<JsVal>),
    /// Nothing is known; the source text.
    Unknown(Box<str>),
}

impl JsVal {
    /// `__dirname`.
    pub fn dirname() -> JsVal {
        JsVal::Path(Anchor::FileDir, Value::str(""))
    }

    fn unknown(r: impl Into<Box<str>>) -> JsVal {
        JsVal::Unknown(r.into())
    }

    /// Whether the value is worth remembering as a constant.
    pub fn is_useful(&self) -> bool {
        !matches!(self, JsVal::Unknown(_))
    }
}

/// What the evaluator can see: the source, top-level constants and bindings.
pub struct Scope<'a, 's> {
    /// The source buffer.
    pub src: &'s [u8],
    /// Top-level `const` values, with whether they depend on the environment.
    pub consts: &'a HashMap<&'s str, (JsVal, bool)>,
    /// Bindings to tracked modules (innermost last).
    pub apis: &'a [ApiBinding<'s>],
    /// Names with `require` semantics.
    pub loaders: &'a [Loader<'s>],
    /// The file's own name, if known (for `__filename`).
    pub file_name: Option<&'a str>,
}

/// Top-level `let` / `var` values at their declaration, with whether they
/// depend on the environment ([`evaluate_with`]). Later assignments are not
/// tracked, so a value read from here is a guess ([`Outcome::guess`]).
pub type Vars<'s> = HashMap<&'s str, (JsVal, bool)>;

impl Scope<'_, '_> {
    /// `__filename`.
    pub fn this_file(&self) -> JsVal {
        match self.file_name {
            Some(n) => JsVal::Path(Anchor::FileDir, Value::str(n)),
            None => JsVal::File,
        }
    }

    /// The value of identifier `name`, and whether it depends on the
    /// environment.
    fn lookup(&self, name: &str) -> (JsVal, bool) {
        if let Some(b) = self.apis.iter().rev().find(|b| b.local == name) {
            return (binding_value(b, self), false);
        }
        if let Some(l) = self.loaders.iter().rev().find(|l| l.local == name) {
            // A `require` that may search from another place is not the file's.
            return match l.elsewhere {
                true => (JsVal::unknown(name), false),
                false => (JsVal::Obj(Obj::Require), false),
            };
        }
        if let Some((v, env)) = self.consts.get(name) {
            return (v.clone(), *env);
        }
        let v = match name {
            "__dirname" => JsVal::dirname(),
            "__filename" => self.this_file(),
            "process" => JsVal::Obj(Obj::Process),
            "String" => JsVal::Obj(Obj::StringCtor),
            "URL" => JsVal::Obj(Obj::UrlCtor),
            "require" => JsVal::Obj(Obj::Require),
            _ => JsVal::unknown(name),
        };
        (v, false)
    }
}

fn binding_value(b: &ApiBinding<'_>, sc: &Scope<'_, '_>) -> JsVal {
    let obj = match b.module.as_ref() {
        "resolve-from" if b.member.is_none() => return JsVal::Func(Func::ResolveFrom),
        "path" | "path/posix" => Obj::Path,
        "url" => Obj::UrlModule,
        "process" => Obj::Process,
        _ => return JsVal::unknown(b.local),
    };
    match &b.member {
        None => JsVal::Obj(obj),
        Some(m) => obj_member(obj, m, sc).unwrap_or_else(|| JsVal::unknown(b.local)),
    }
}

pub(crate) fn obj_member(o: Obj, name: &str, sc: &Scope<'_, '_>) -> Option<JsVal> {
    use Func::*;
    Some(match (o, name) {
        (Obj::Path, "join") => JsVal::Func(PathJoin),
        (Obj::Path, "resolve") => JsVal::Func(PathResolve),
        (Obj::Path, "dirname") => JsVal::Func(PathDirname),
        (Obj::Path, "basename") => JsVal::Func(PathBasename),
        (Obj::Path, "extname") => JsVal::Func(PathExtname),
        (Obj::Path, "relative") => JsVal::Func(PathRelative),
        (Obj::Path, "normalize") => JsVal::Func(PathNormalize),
        (Obj::Path, "posix" | "default") => JsVal::Obj(Obj::Path),
        (Obj::Path, "sep") => JsVal::Str(Value::str("/")),
        (Obj::Path, "delimiter") => JsVal::Str(Value::str(":")),
        (Obj::UrlModule, "fileURLToPath") => JsVal::Func(FileUrlToPath),
        (Obj::UrlModule, "pathToFileURL") => JsVal::Func(PathToFileUrl),
        (Obj::UrlModule, "URL") => JsVal::Obj(Obj::UrlCtor),
        (Obj::UrlModule, "default") => JsVal::Obj(Obj::UrlModule),
        (Obj::Process, "cwd") => JsVal::Func(ProcessCwd),
        (Obj::Process, "env") => JsVal::Obj(Obj::ProcessEnv),
        (Obj::ImportMeta, "url") => JsVal::FileUrl,
        (Obj::ImportMeta, "dirname") => JsVal::dirname(),
        (Obj::ImportMeta, "filename") => sc.this_file(),
        (Obj::ImportMeta, "resolve") => JsVal::Func(MetaResolve),
        (Obj::Require, "resolve") => JsVal::Func(Resolve),
        (Obj::StringCtor, "raw") => JsVal::Func(StringRaw),
        _ => return None,
    })
}

/// The result of [`evaluate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The value (unknown with the whole source text if parsing failed).
    pub val: JsVal,
    /// Some part depends on `process.env`.
    pub env: bool,
    /// The tokens formed exactly one expression.
    pub complete: bool,
    /// The value relies on an assumption (a mutable binding's initial value, an
    /// unknown object's `.join` taken as `path.join`): the detail for an
    /// uncertainty record.
    pub guess: Option<Box<str>>,
    /// The value read a top-level `const`/`let`/`var` by name. The token
    /// evaluator does not know scopes (a parameter or local of the same
    /// name shadows it), so such values are re-evaluated by the value-flow
    /// engine.
    pub named: bool,
    /// The value comes from the value-flow engine ([`crate::flow`]): its
    /// unknown parts carry origin marks ([`super::origin`]) and `env` is
    /// not used.
    pub flow: bool,
}

/// Evaluates `toks` as one expression.
pub fn evaluate(sc: &Scope<'_, '_>, toks: &[Token]) -> Outcome {
    evaluate_with(sc, None, toks)
}

/// [`evaluate`] that also sees top-level `let`/`var` initial values ([`Vars`]);
/// a value read from them is reported in [`Outcome::guess`].
pub fn evaluate_with<'s>(sc: &Scope<'_, 's>, vars: Option<&Vars<'s>>, toks: &[Token]) -> Outcome {
    let mut ev = Ev {
        sc,
        vars,
        toks,
        pos: 0,
        nest: 0,
        env: false,
        guess: None,
        named: false,
    };
    match ev.expr() {
        Some(val) if ev.pos == toks.len() => Outcome {
            val,
            env: ev.env,
            complete: true,
            guess: ev.guess,
            named: ev.named,
            flow: false,
        },
        _ => Outcome {
            val: JsVal::Unknown(reason_text(sc.src, toks)),
            env: ev.env,
            complete: false,
            guess: None,
            named: ev.named,
            flow: false,
        },
    }
}

/// The source text of `toks`, whitespace-collapsed and capped, as a wildcard
/// reason.
pub fn reason_text(src: &[u8], toks: &[Token]) -> Box<str> {
    let (Some(a), Some(b)) = (toks.first(), toks.last()) else {
        return "".into();
    };
    let bytes = src
        .get(a.start as usize..(b.end as usize).max(a.start as usize))
        .unwrap_or_default();
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.len() > MAX_REASON {
        let mut n = MAX_REASON;
        while !out.is_char_boundary(n) {
            n -= 1;
        }
        out.truncate(n);
        out.push('…');
    }
    out.into()
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Bin {
    Add,
    Or,
    And,
    Other,
}

struct Ev<'a, 's> {
    sc: &'a Scope<'a, 's>,
    vars: Option<&'a Vars<'s>>,
    toks: &'a [Token],
    pos: usize,
    nest: u32,
    env: bool,
    guess: Option<Box<str>>,
    named: bool,
}

impl<'s> Ev<'_, 's> {
    /// Records an assumption (the first one wins).
    fn assume(&mut self, why: impl FnOnce() -> String) {
        if self.guess.is_none() {
            self.guess = Some(why().into());
        }
    }

    /// The left operand of a concatenation: `import.meta.url` with a known file
    /// name is the file's URL (`${import.meta.url}/../x`).
    fn concat_left(&self, v: JsVal) -> JsVal {
        match (v, self.sc.file_name) {
            (JsVal::FileUrl, Some(n)) => JsVal::Url(Anchor::FileDir, Value::str(n)),
            (v, _) => v,
        }
    }

    fn peek(&self) -> Option<Token> {
        self.toks.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<Token> {
        self.toks.get(self.pos + n).copied()
    }

    fn kind(&self) -> TokKind {
        self.peek().map_or(TokKind::Eof, |t| t.kind)
    }

    fn text(&self, t: &Token) -> &'s [u8] {
        t.text(self.sc.src)
    }

    fn reason(&self, a: usize) -> Box<str> {
        let b = self.pos.min(self.toks.len());
        if a >= b {
            return "".into();
        }
        reason_text(self.sc.src, &self.toks[a..b])
    }

    fn unk(&self, a: usize) -> JsVal {
        JsVal::Unknown(self.reason(a))
    }

    fn enter(&mut self) -> Option<()> {
        self.nest += 1;
        (self.nest <= MAX_NEST).then_some(())
    }

    fn expr(&mut self) -> Option<JsVal> {
        self.enter()?;
        let r = self.ternary();
        self.nest -= 1;
        r
    }

    fn ternary(&mut self) -> Option<JsVal> {
        let v = self.binary(0)?;
        if self.kind() != TokKind::Question {
            return Some(v);
        }
        self.pos += 1;
        let a = self.expr()?;
        if self.kind() != TokKind::Colon {
            return None;
        }
        self.pos += 1;
        let b = self.expr()?;
        Some(alts([a, b]))
    }

    fn binop(&self) -> Option<(u8, Bin, usize)> {
        let t = self.peek()?;
        Some(match t.kind {
            TokKind::Star => (11, Bin::Other, 1),
            TokKind::Lt => (8, Bin::Other, 1),
            TokKind::Gt => {
                // `>>`, `>=`, `>>>` are lexed as glued single-character tokens.
                let mut n = 1;
                let mut end = t.end;
                while n < 4
                    && let Some(u) = self.peek_at(n)
                    && u.start == end
                    && matches!(u.kind, TokKind::Gt | TokKind::Assign)
                {
                    end = u.end;
                    n += 1;
                }
                (8, Bin::Other, n)
            }
            TokKind::Ident if matches!(t.kw, Kw::In | Kw::Instanceof) => (8, Bin::Other, 1),
            TokKind::Op => match self.text(&t) {
                b"+" => (10, Bin::Add, 1),
                b"-" => (10, Bin::Other, 1),
                b"/" | b"%" => (11, Bin::Other, 1),
                b"**" => (12, Bin::Other, 1),
                b"==" | b"!=" | b"===" | b"!==" => (7, Bin::Other, 1),
                b"<=" => (8, Bin::Other, 1),
                b"<<" => (9, Bin::Other, 1),
                b"&&" => (3, Bin::And, 1),
                b"||" | b"??" => (2, Bin::Or, 1),
                b"&" => (6, Bin::Other, 1),
                b"|" => (4, Bin::Other, 1),
                b"^" => (5, Bin::Other, 1),
                _ => return None,
            },
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> Option<JsVal> {
        let start = self.pos;
        let mut left = self.unary()?;
        while let Some((prec, op, n)) = self.binop() {
            if prec <= min {
                break;
            }
            self.pos += n;
            let rstart = self.pos;
            let right = self.binary(prec)?;
            left = match op {
                // Two operands that can both be numbers are added, not concatenated (a
                // numeric literal is kept as its digits: `n + 1` is not `n` followed by
                // `1`).
                Bin::Add if maybe_number(&left) && maybe_number(&right) => self.unk(start),
                Bin::Add => {
                    let left = self.concat_left(left);
                    let l = stringy(left, || self.reason(start));
                    let r = stringy_right(right, || self.reason(rstart));
                    concat(l, r, &self.reason(start))
                }
                Bin::Or => alts([left, right]),
                // `a && b` is `b`, or `a` when that is falsy: the empty string for an empty
                // `a` (`` `x${s && '-' + s}` `` is `x`), and nothing that is a path
                // otherwise.
                Bin::And => match left {
                    JsVal::Str(Value::Str(s)) if !s.is_empty() && s != "0" => right,
                    JsVal::Path(..) | JsVal::Url(..) | JsVal::FileUrl | JsVal::File => right,
                    _ => alts([right, JsVal::Str(Value::str(""))]),
                },
                Bin::Other => self.unk(start),
            };
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<JsVal> {
        let start = self.pos;
        let t = self.peek()?;
        let prefix = match t.kind {
            TokKind::Bang | TokKind::IncDec => true,
            TokKind::Op => matches!(self.text(&t), b"-" | b"+" | b"~"),
            TokKind::Ident => matches!(t.kw, Kw::Typeof | Kw::Void | Kw::Delete),
            _ => false,
        };
        if prefix {
            self.pos += 1;
            self.enter()?;
            let r = self.unary();
            self.nest -= 1;
            r?;
            return Some(self.unk(start));
        }
        if t.kind == TokKind::Ident && t.kw == Kw::Await {
            self.pos += 1;
            self.enter()?;
            let r = self.unary();
            self.nest -= 1;
            return r;
        }
        if t.kind == TokKind::Ident && t.kw == Kw::New {
            return self.new_expr();
        }
        let v = self.primary()?;
        self.postfix(v, start)
    }

    fn new_expr(&mut self) -> Option<JsVal> {
        let start = self.pos;
        self.pos += 1;
        let mut callee = self.primary()?;
        while self.kind() == TokKind::Dot {
            let n = self.peek_at(1)?;
            if n.kind != TokKind::Ident {
                return None;
            }
            self.pos += 2;
            let name = std::str::from_utf8(self.text(&n)).ok()?;
            callee = self.member(callee, name, start);
        }
        let args = if self.kind() == TokKind::LParen {
            self.args()?
        } else {
            Vec::new()
        };
        let v = match callee {
            JsVal::Obj(Obj::UrlCtor) => {
                let sc = self.sc;
                distribute(args, |a| url_ctor(a, sc)).unwrap_or_else(|| self.unk(start))
            }
            _ => self.unk(start),
        };
        self.postfix(v, start)
    }

    fn postfix(&mut self, mut v: JsVal, start: usize) -> Option<JsVal> {
        while let Some(t) = self.peek() {
            match t.kind {
                TokKind::Dot | TokKind::OptChain => {
                    let n = self.peek_at(1)?;
                    if n.kind != TokKind::Ident {
                        if t.kind == TokKind::OptChain
                            && matches!(n.kind, TokKind::LParen | TokKind::LBracket)
                        {
                            self.pos += 1;
                            continue;
                        }
                        return None;
                    }
                    self.pos += 2;
                    let name = std::str::from_utf8(self.text(&n)).ok()?;
                    if self.kind() == TokKind::LParen {
                        let args = self.args()?;
                        v = self.method(v, name, args, start);
                    } else {
                        v = self.member(v, name, start);
                    }
                }
                TokKind::LBracket => {
                    self.pos += 1;
                    let k = self.expr()?;
                    if self.kind() != TokKind::RBracket {
                        return None;
                    }
                    self.pos += 1;
                    v = match (&v, k) {
                        (_, JsVal::Str(Value::Str(s))) => self.member(v, &s, start),
                        (JsVal::Obj(Obj::ProcessEnv), _) => {
                            self.env = true;
                            self.unk(start)
                        }
                        _ => self.unk(start),
                    };
                }
                TokKind::LParen => {
                    let args = self.args()?;
                    v = self.call(v, args, start);
                }
                TokKind::Template | TokKind::TemplateHead => {
                    let raw = v == JsVal::Func(Func::StringRaw);
                    let tv = self.template(raw)?;
                    v = if raw { tv } else { self.unk(start) };
                }
                // TypeScript non-null assertion (`x!`): `!` cannot follow a value otherwise.
                TokKind::Bang => self.pos += 1,
                _ => break,
            }
        }
        Some(v)
    }

    fn args(&mut self) -> Option<Vec<JsVal>> {
        self.pos += 1; // `(`
        let mut out = Vec::new();
        loop {
            match self.kind() {
                TokKind::RParen => {
                    self.pos += 1;
                    return Some(out);
                }
                TokKind::Ellipsis => {
                    // `f(...xs)`: an unknown number of unknown arguments, kept as one
                    // wildcard (nft does the same).
                    let a = self.pos;
                    self.pos += 1;
                    self.expr()?;
                    if out.len() >= MAX_LIST {
                        return None;
                    }
                    out.push(self.unk(a));
                    match self.kind() {
                        TokKind::Comma => self.pos += 1,
                        TokKind::RParen => {}
                        _ => return None,
                    }
                    continue;
                }
                _ => {}
            }
            if out.len() >= MAX_LIST {
                return None;
            }
            out.push(self.expr()?);
            match self.kind() {
                TokKind::Comma => self.pos += 1,
                TokKind::RParen => {}
                _ => return None,
            }
        }
    }

    fn primary(&mut self) -> Option<JsVal> {
        let start = self.pos;
        let t = self.peek()?;
        match t.kind {
            TokKind::Str | TokKind::Template => {
                self.pos += 1;
                let raw = t.literal_content(self.sc.src)?;
                let s = decode(raw, EscapeRules::JS).ok()?;
                Some(JsVal::Str(Value::Str(s.into_owned())))
            }
            TokKind::TemplateHead => self.template(false),
            TokKind::Number => {
                self.pos += 1;
                let txt = self.text(&t);
                if txt.len() <= 15 && txt.iter().all(u8::is_ascii_digit) {
                    Some(JsVal::Str(Value::Str(
                        String::from_utf8_lossy(txt).into_owned(),
                    )))
                } else {
                    Some(self.unk(start))
                }
            }
            TokKind::LParen => {
                self.pos += 1;
                let mut v = self.expr()?;
                while self.kind() == TokKind::Comma {
                    self.pos += 1;
                    v = self.expr()?;
                }
                if self.kind() != TokKind::RParen {
                    return None;
                }
                self.pos += 1;
                if self.kind() == TokKind::Arrow {
                    return None;
                }
                Some(v)
            }
            TokKind::LBracket => {
                self.pos += 1;
                let mut els = Vec::new();
                loop {
                    match self.kind() {
                        TokKind::RBracket => {
                            self.pos += 1;
                            return Some(JsVal::Arr(els));
                        }
                        TokKind::Comma | TokKind::Ellipsis => return None,
                        _ => {}
                    }
                    if els.len() >= MAX_LIST {
                        return None;
                    }
                    els.push(self.expr()?);
                    match self.kind() {
                        TokKind::Comma => self.pos += 1,
                        TokKind::RBracket => {}
                        _ => return None,
                    }
                }
            }
            TokKind::LBrace => {
                // An object literal: skipped, unknown.
                let close = self.toks[self.pos + 1..]
                    .iter()
                    .position(|u| u.kind == TokKind::RBrace && u.depth == t.depth)?;
                self.pos += close + 2;
                Some(self.unk(start))
            }
            TokKind::Ident => match t.kw {
                Kw::Import => {
                    let n = self.peek_at(1)?;
                    if n.kind == TokKind::Dot
                        && self.peek_at(2).is_some_and(|m| self.text(&m) == b"meta")
                    {
                        self.pos += 3;
                        Some(JsVal::Obj(Obj::ImportMeta))
                    } else if n.kind == TokKind::LParen {
                        self.pos += 1;
                        self.args()?;
                        Some(self.unk(start))
                    } else {
                        None
                    }
                }
                Kw::Function | Kw::Class => None,
                _ => {
                    if self.peek_at(1).is_some_and(|n| n.kind == TokKind::Arrow) {
                        return None;
                    }
                    self.pos += 1;
                    let name = std::str::from_utf8(self.text(&t)).ok()?;
                    Some(match name {
                        "true" | "false" | "null" | "undefined" | "this" => JsVal::unknown(name),
                        _ => {
                            let (mut v, mut env) = self.sc.lookup(name);
                            self.named |= self.sc.consts.contains_key(name);
                            let mut mutable = false;
                            if matches!(v, JsVal::Unknown(_))
                                && let Some((x, e)) = self.vars.and_then(|m| m.get(name))
                            {
                                (v, env, mutable) = (x.clone(), *e, true);
                                self.named = true;
                            }
                            self.env |= env;
                            if mutable {
                                self.assume(|| {
                                    format!(
                                        "`{name}` is a `let`/`var` or function-local binding; its \
                                         initial value is used (later assignments are not tracked)"
                                    )
                                });
                            }
                            v
                        }
                    })
                }
            },
            TokKind::Regex | TokKind::Jsx | TokKind::PrivateName => {
                self.pos += 1;
                Some(self.unk(start))
            }
            _ => None,
        }
    }

    /// A template literal (at a `Template` or `TemplateHead` token). `raw`:
    /// `String.raw`.
    fn template(&mut self, raw: bool) -> Option<JsVal> {
        let src = self.sc.src;
        let part = |tok: &Token, trim: usize| -> Option<String> {
            if tok.flags & UNTERMINATED != 0 {
                return None;
            }
            let s = src.get(tok.start as usize + 1..(tok.end as usize).checked_sub(trim)?)?;
            if raw {
                std::str::from_utf8(s).ok().map(str::to_owned)
            } else {
                decode(s, EscapeRules::JS).ok().map(Cow::into_owned)
            }
        };
        let t = self.peek()?;
        match t.kind {
            TokKind::Template => {
                self.pos += 1;
                Some(JsVal::Str(Value::Str(part(&t, 1)?)))
            }
            TokKind::TemplateHead => {
                let start = self.pos;
                self.pos += 1;
                let head = part(&t, 2)?;
                // `${__dirname}/x`: with an empty head the first substitution starts the
                // value, like the left operand of `+`.
                let mut acc = (!head.is_empty()).then_some(JsVal::Str(Value::Str(head)));
                loop {
                    let a = self.pos;
                    let v = self.expr()?;
                    let acc_v = match acc.take() {
                        None => {
                            let v = self.concat_left(v);
                            stringy(v, || self.reason(a))
                        }
                        Some(acc) => {
                            let v = stringy_right(v, || self.reason(a));
                            concat(acc, v, &self.reason(start))
                        }
                    };
                    let m = self.peek()?;
                    let trim = match m.kind {
                        TokKind::TemplateMiddle => 2,
                        TokKind::TemplateTail => 1,
                        _ => return None,
                    };
                    self.pos += 1;
                    let acc_v = concat(acc_v, JsVal::Str(Value::Str(part(&m, trim)?)), "");
                    if m.kind == TokKind::TemplateTail {
                        return Some(acc_v);
                    }
                    acc = Some(acc_v);
                }
            }
            _ => None,
        }
    }

    fn member(&mut self, v: JsVal, name: &str, start: usize) -> JsVal {
        match v {
            JsVal::Obj(Obj::ProcessEnv) => {
                self.env = true;
                JsVal::unknown(format!("process.env.{name}"))
            }
            JsVal::Obj(o) => obj_member(o, name, self.sc).unwrap_or_else(|| self.unk(start)),
            JsVal::Url(a, p) if matches!(name, "pathname" | "href") => JsVal::Url(a, p),
            JsVal::Resolved(m, p) if matches!(name, "pathname" | "href") => JsVal::Resolved(m, p),
            JsVal::FileUrl if name == "href" => JsVal::FileUrl,
            JsVal::FileUrl if name == "pathname" => self.sc.this_file(),
            JsVal::Unknown(_) if name == "join" => JsVal::Func(Func::GuessJoin),
            JsVal::Unknown(_) if name == "resolve" => JsVal::Func(Func::GuessResolve),
            JsVal::Alts(vs) => {
                let mut out = Vec::with_capacity(vs.len());
                for x in vs {
                    out.push(self.member(x, name, start));
                }
                alts(out)
            }
            _ => self.unk(start),
        }
    }

    fn method(&mut self, v: JsVal, name: &str, args: Vec<JsVal>, start: usize) -> JsVal {
        match (v, name) {
            (JsVal::Arr(els), "join") => {
                let sep = match args.first() {
                    None => ",".to_string(),
                    Some(JsVal::Str(Value::Str(s))) => s.clone(),
                    Some(_) => return self.unk(start),
                };
                let reason = self.reason(start);
                let mut acc: Option<JsVal> = None;
                for e in els {
                    let e = join_element(e);
                    acc = Some(match acc {
                        None => stringy(e, || reason.clone()),
                        Some(a) => concat(
                            concat(a, JsVal::Str(Value::Str(sep.clone())), &reason),
                            stringy_right(e, || reason.clone()),
                            &reason,
                        ),
                    });
                }
                acc.unwrap_or(JsVal::Str(Value::str("")))
            }
            (
                v @ (JsVal::Url(..) | JsVal::FileUrl | JsVal::Str(_) | JsVal::Resolved(..)),
                "toString",
            ) => v,
            (JsVal::Str(s), "concat") => {
                let reason = self.reason(start);
                let mut acc = JsVal::Str(s);
                for a in args {
                    let a = stringy_right(a, || reason.clone());
                    acc = concat(acc, a, &reason);
                }
                acc
            }
            (v, _) => {
                let f = self.member(v, name, start);
                self.call(f, args, start)
            }
        }
    }

    fn call(&mut self, f: JsVal, args: Vec<JsVal>, start: usize) -> JsVal {
        // `require('path')`, `require('node:url')`, ... inline.
        if f == JsVal::Obj(Obj::Require)
            && let [JsVal::Str(Value::Str(m))] = args.as_slice()
        {
            return match m.strip_prefix("node:").unwrap_or(m) {
                "path" | "path/posix" => JsVal::Obj(Obj::Path),
                "url" => JsVal::Obj(Obj::UrlModule),
                "process" => JsVal::Obj(Obj::Process),
                "resolve-from" => JsVal::Func(Func::ResolveFrom),
                _ => self.unk(start),
            };
        }
        let JsVal::Func(func) = f else {
            return self.unk(start);
        };
        let sc = self.sc;
        let r = distribute(args, |a| apply(func, a, sc));
        if r.is_some() && matches!(func, Func::GuessJoin | Func::GuessResolve) {
            let what = if func == Func::GuessJoin {
                "join"
            } else {
                "resolve"
            };
            let callee = self.reason(start);
            self.assume(|| format!("`{callee}` assumed to be `path.{what}`"));
        }
        r.unwrap_or_else(|| self.unk(start))
    }
}

/// Whether a value can be a number at runtime: unknown, or digits (a numeric
/// literal is kept as its text).
fn maybe_number(v: &JsVal) -> bool {
    match v {
        JsVal::Unknown(_) => true,
        JsVal::Str(Value::Str(s)) => !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
        JsVal::Alts(vs) => vs.iter().any(maybe_number),
        _ => false,
    }
}

/// An array element as `join` writes it: `undefined` and `null` are the empty
/// string.
fn join_element(v: JsVal) -> JsVal {
    fn missing(r: &str) -> bool {
        matches!(r, "undefined" | "null")
    }
    match v {
        JsVal::Unknown(r) if missing(&r) => JsVal::Str(Value::str("")),
        JsVal::Alts(vs) => alts(vs.into_iter().map(join_element)),
        JsVal::Str(Value::Alternatives(vs)) => {
            JsVal::Str(Value::alternatives(vs.into_iter().map(|x| match x {
                Value::Unknown { reason } if missing(&reason) => Value::str(""),
                x => x,
            })))
        }
        v => v,
    }
}

/// Left operand of `+`: values that cannot be strings become unknown.
pub(crate) fn stringy(v: JsVal, reason: impl Fn() -> Box<str>) -> JsVal {
    match v {
        JsVal::Str(_)
        | JsVal::Unknown(_)
        | JsVal::Path(..)
        | JsVal::Url(..)
        | JsVal::Resolved(..)
        | JsVal::Alts(_) => v,
        _ => JsVal::Unknown(reason()),
    }
}

/// Right operand of `+` (and template substitutions): only strings and unknowns
/// keep their value; a path on the right of a string is unknown.
pub(crate) fn stringy_right(v: JsVal, reason: impl Fn() -> Box<str>) -> JsVal {
    match v {
        JsVal::Str(_) | JsVal::Unknown(_) => v,
        JsVal::Alts(vs) if vs.iter().all(|x| matches!(x, JsVal::Str(_))) => JsVal::Alts(vs),
        _ => JsVal::Unknown(reason()),
    }
}

fn too_long(v: &Value) -> bool {
    match v {
        Value::Str(s) => s.len() > MAX_STR,
        Value::Pattern(p) => {
            p.iter()
                .map(|x| match x {
                    PatPart::Lit(s) => s.len(),
                    PatPart::Wild { .. } => 1,
                })
                .sum::<usize>()
                > MAX_STR
        }
        Value::Alternatives(a) => a.iter().any(too_long),
        Value::Unknown { .. } => false,
    }
}

fn starts_with_slash_or_wild(v: &Value) -> bool {
    match v {
        Value::Str(s) => s.is_empty() || s.starts_with('/'),
        Value::Pattern(p) => match p.first() {
            Some(PatPart::Lit(s)) => s.starts_with('/'),
            _ => true,
        },
        Value::Alternatives(a) => a.iter().all(starts_with_slash_or_wild),
        Value::Unknown { .. } => true,
    }
}

/// `a + b` on abstract values.
pub fn concat(a: JsVal, b: JsVal, reason: &str) -> JsVal {
    let r = match (a, b) {
        (JsVal::Alts(xs), b) => {
            if xs.len() * alt_count(&b) > MAX_ALTERNATIVES {
                return JsVal::unknown(reason);
            }
            return alts(xs.into_iter().map(|x| concat(x, b.clone(), reason)));
        }
        (a, JsVal::Alts(ys)) => {
            if ys.len() * alt_count(&a) > MAX_ALTERNATIVES {
                return JsVal::unknown(reason);
            }
            return alts(ys.into_iter().map(|y| concat(a.clone(), y, reason)));
        }
        (JsVal::Str(x), JsVal::Str(y)) => JsVal::Str(x.concat(y)),
        (JsVal::Str(x), JsVal::Unknown(r)) => JsVal::Str(x.concat(Value::unknown(r))),
        (JsVal::Unknown(r), JsVal::Str(y)) => JsVal::Str(Value::unknown(r).concat(y)),
        (JsVal::Unknown(r1), JsVal::Unknown(r2)) => {
            JsVal::Str(Value::unknown(r1).concat(Value::unknown(r2)))
        }
        (JsVal::Path(an, v), JsVal::Str(y)) => {
            if v.as_str() == Some("") && !starts_with_slash_or_wild(&y) {
                // `__dirname + 'x'`: a sibling of the directory, not inside it.
                return JsVal::unknown(reason);
            }
            JsVal::Path(an, v.concat(y))
        }
        (JsVal::Path(an, v), JsVal::Unknown(r)) => JsVal::Path(an, v.concat(Value::unknown(r))),
        // A path next to a resolved module is steps from that file, not text: only what
        // starts a new segment (`root + '/dist/x.js'`), or follows a directory URL, can be
        // appended.
        (JsVal::Resolved(m, v), JsVal::Str(y)) if y.as_str() == Some("") => JsVal::Resolved(m, v),
        (JsVal::Resolved(m, v), JsVal::Str(y)) => {
            let segment = match &y {
                Value::Str(s) => s.starts_with('/'),
                Value::Pattern(p) => {
                    matches!(p.first(), Some(PatPart::Lit(s)) if s.starts_with('/'))
                }
                _ => false,
            };
            if !segment && !ends_with_slash(&v) {
                return JsVal::unknown(reason);
            }
            let dir = ends_with_slash(&y);
            let joined = normalize_value(path::join([v, y]), true);
            JsVal::Resolved(
                m,
                match joined {
                    Value::Str(s) if dir => Value::Str(format!("{s}/")),
                    j => j,
                },
            )
        }
        // `${import.meta.url}/../x`
        (JsVal::Url(an, v), JsVal::Str(y)) => JsVal::Url(an, v.concat(y)),
        (JsVal::Url(an, v), JsVal::Unknown(r)) => JsVal::Url(an, v.concat(Value::unknown(r))),
        _ => return JsVal::unknown(reason),
    };
    match &r {
        JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v) | JsVal::Resolved(_, v)
            if too_long(v) =>
        {
            JsVal::unknown("string too long")
        }
        JsVal::Str(Value::Unknown { reason }) => JsVal::Unknown(reason.clone()),
        _ => r,
    }
}

fn alt_count(v: &JsVal) -> usize {
    match v {
        JsVal::Alts(a) => a.len(),
        _ => 1,
    }
}

/// One of `vals`: merged into a single string or same-anchor path value when
/// possible.
pub fn alts(vals: impl IntoIterator<Item = JsVal>) -> JsVal {
    let mut out: Vec<JsVal> = Vec::new();
    for v in vals {
        match v {
            JsVal::Alts(inner) => {
                for i in inner {
                    if !out.contains(&i) {
                        out.push(i);
                    }
                }
            }
            v => {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
        }
    }
    if out.len() == 1 {
        return out.pop().unwrap_or_else(|| JsVal::unknown(""));
    }
    if out
        .iter()
        .all(|v| matches!(v, JsVal::Str(_) | JsVal::Unknown(_)))
    {
        return JsVal::Str(Value::alternatives(out.into_iter().map(|v| match v {
            JsVal::Str(s) => s,
            JsVal::Unknown(r) => Value::Unknown { reason: r },
            _ => unreachable!("checked above"),
        })));
    }
    if let Some(JsVal::Path(a0, _)) = out.first() {
        let a0 = *a0;
        if out
            .iter()
            .all(|v| matches!(v, JsVal::Path(a, _) if *a == a0))
        {
            return JsVal::Path(
                a0,
                Value::alternatives(out.into_iter().map(|v| match v {
                    JsVal::Path(_, p) => p,
                    _ => unreachable!("checked above"),
                })),
            );
        }
    }
    if out.len() > MAX_ALTERNATIVES {
        return JsVal::unknown(format!("one of {} alternatives", out.len()));
    }
    JsVal::Alts(out)
}

/// Splits alternatives (at the JS and string level) into leaves.
pub fn split_alts(v: JsVal) -> Vec<JsVal> {
    let mut out = Vec::new();
    split_into(v, &mut out);
    out
}

fn split_into(v: JsVal, out: &mut Vec<JsVal>) {
    match v {
        JsVal::Alts(vs) => vs.into_iter().for_each(|x| split_into(x, out)),
        JsVal::Str(Value::Alternatives(vs)) => {
            vs.into_iter().for_each(|x| split_into(JsVal::Str(x), out))
        }
        JsVal::Path(a, Value::Alternatives(vs)) => vs
            .into_iter()
            .for_each(|x| split_into(JsVal::Path(a, x), out)),
        JsVal::Url(a, Value::Alternatives(vs)) => vs
            .into_iter()
            .for_each(|x| split_into(JsVal::Url(a, x), out)),
        v => out.push(v),
    }
}

/// Applies `f` to every combination of alternative arguments (capped).
fn distribute(args: Vec<JsVal>, f: impl Fn(&[JsVal]) -> Option<JsVal>) -> Option<JsVal> {
    let split: Vec<Vec<JsVal>> = args.into_iter().map(split_alts).collect();
    let total = split
        .iter()
        .try_fold(1usize, |n, s| n.checked_mul(s.len().max(1)))?;
    if total == 1 {
        let flat: Vec<JsVal> = split.into_iter().flatten().collect();
        return f(&flat);
    }
    if total > MAX_ALTERNATIVES {
        return None;
    }
    let mut results = Vec::with_capacity(total);
    let mut idx = vec![0usize; split.len()];
    loop {
        let combo: Vec<JsVal> = split
            .iter()
            .zip(&idx)
            .filter_map(|(s, &i)| s.get(i).cloned())
            .collect();
        results.push(f(&combo)?);
        // Next combination.
        let mut k = 0;
        loop {
            if k == idx.len() {
                return Some(alts(results));
            }
            idx[k] += 1;
            if idx[k] < split[k].len() {
                break;
            }
            idx[k] = 0;
            k += 1;
        }
    }
}

pub(crate) fn apply(func: Func, args: &[JsVal], sc: &Scope<'_, '_>) -> Option<JsVal> {
    match func {
        Func::PathJoin => path_join(args),
        Func::PathResolve => path_resolve(args),
        Func::PathDirname => path_dirname(args.first()?),
        Func::PathBasename => path_basename(args.first()?, args.get(1)),
        Func::PathExtname => path_extname(args.first()?),
        Func::PathRelative => path_relative(args.first()?, args.get(1)?),
        Func::PathNormalize => match args.first()? {
            // `path.normalize` keeps a trailing `/`.
            JsVal::Path(a, Value::Str(s)) if s.ends_with('/') => Some(JsVal::Path(
                *a,
                match normalize_value(Value::Str(s.clone()), true) {
                    Value::Str(n) => Value::Str(format!("{n}/")),
                    n => n,
                },
            )),
            JsVal::Path(a, v) => Some(JsVal::Path(*a, normalize_value(v.clone(), true))),
            JsVal::Str(v) => Some(JsVal::Str(normalize_value(v.clone(), false))),
            _ => None,
        },
        Func::FileUrlToPath => match args.first()? {
            JsVal::FileUrl => Some(sc.this_file()),
            JsVal::Url(a, v) => Some(JsVal::Path(*a, v.clone())),
            // The same file, as a path.
            JsVal::Resolved(m, v) => Some(JsVal::Resolved(m.clone(), v.clone())),
            JsVal::Str(Value::Str(s)) => Some(JsVal::Str(Value::Str(strip_file_url(s)?))),
            _ => None,
        },
        Func::PathToFileUrl => match args.first()? {
            JsVal::Path(a, v) => Some(JsVal::Url(*a, v.clone())),
            JsVal::File => Some(JsVal::FileUrl),
            // The same file, as a URL.
            JsVal::Resolved(m, v) => Some(JsVal::Resolved(m.clone(), v.clone())),
            // A relative path is resolved against the working directory.
            JsVal::Str(v @ Value::Str(_)) if !path::starts_absolute(v) => {
                Some(JsVal::Url(Anchor::Cwd, normalize_value(v.clone(), true)))
            }
            _ => None,
        },
        Func::ProcessCwd => Some(JsVal::Path(Anchor::Cwd, Value::str(""))),
        // A second argument (`require.resolve(x, { paths })`, the parent of
        // `import.meta.resolve(x, parent)`) says where the search starts: the value-flow
        // engine reads it ([`resolve_from`]); here the result is not known.
        Func::Resolve | Func::MetaResolve if args.len() > 1 => None,
        Func::Resolve | Func::MetaResolve => match args.first()? {
            JsVal::Str(Value::Str(s)) if s.starts_with("./") || s.starts_with("../") => {
                Some(if func == Func::MetaResolve {
                    JsVal::Url(Anchor::FileDir, Value::Str(s.clone()))
                } else {
                    JsVal::Path(Anchor::FileDir, Value::Str(s.clone()))
                })
            }
            // A package (not a builtin or URL): the resolved file is only known to the
            // resolver.
            JsVal::Str(Value::Str(s)) => Some(JsVal::Resolved(
                Located::new(s, func == Func::MetaResolve)?,
                Value::str(""),
            )),
            _ => None,
        },
        Func::StringRaw => None,
        Func::ResolveFrom => match (args.first()?, args.get(1)?) {
            (JsVal::Path(a, dir), JsVal::Str(Value::Str(s)))
                if s.starts_with("./") || s.starts_with("../") =>
            {
                Some(JsVal::Path(
                    *a,
                    normalize_value(path::join([dir.clone(), Value::Str(s.clone())]), true),
                ))
            }
            _ => None,
        },
        Func::GuessJoin => match args.first()? {
            JsVal::Path(..) => path_join(args),
            _ => None,
        },
        Func::GuessResolve => match args.first()? {
            JsVal::Path(..) => path_resolve(args),
            _ => None,
        },
    }
}

/// What `require.resolve(request)` returns when the search starts at `start`
/// instead of at the file: one entry of `require.resolve(request, { paths })`,
/// or the `require` function `createRequire` made for a module in that
/// directory. A relative request is the path joined onto the start; a package
/// is searched in the `node_modules` directories from the start upward. `None`
/// for anything else (an absolute path, a builtin, a `#` import, a request that
/// is not one known string).
pub fn resolve_from(request: &JsVal, start: &Start) -> Option<JsVal> {
    let JsVal::Str(Value::Str(s)) = request else {
        return None;
    };
    if s.starts_with("./") || s.starts_with("../") {
        let joined = |base: &str| {
            normalize_value(path::join([Value::str(base), Value::Str(s.clone())]), true)
        };
        return Some(match start {
            Start::Dir(d) => JsVal::Path(Anchor::FileDir, joined(d)),
            Start::Next(l, rel) => JsVal::Resolved(l.clone(), joined(rel)),
        });
    }
    Some(JsVal::Resolved(
        Located::from_start(s, start.clone())?,
        Value::str(""),
    ))
}

/// The place a module search starts for the path value `v` (an entry of `{
/// paths }`): a known path below the file's directory, or next to a resolved
/// module. `None` when the value is not one known path (a pattern, the working
/// directory, a string).
pub fn start_at(v: &JsVal) -> Option<Start> {
    match v {
        JsVal::Path(Anchor::FileDir, p) | JsVal::Url(Anchor::FileDir, p) => {
            match normalize_value(p.clone(), true) {
                Value::Str(d) => Some(Start::Dir(d.into())),
                _ => None,
            }
        }
        JsVal::Resolved(l, p) => match normalize_value(p.clone(), true) {
            Value::Str(rel) => Some(Start::Next(l.clone(), rel.into())),
            _ => None,
        },
        _ => None,
    }
}

/// The place the `require` function of `createRequire(v)` searches from: the
/// directory of the file `v` names, or `v` itself when it ends with `/` (Node
/// takes the text up to the last `/`). `None` when `v` is not one known path or
/// URL.
pub fn start_of_file(v: &JsVal) -> Option<Start> {
    let (JsVal::Path(_, p) | JsVal::Url(_, p) | JsVal::Resolved(_, p)) = v else {
        return None;
    };
    let dir = match p {
        Value::Str(s) if s.ends_with('/') => p.clone(),
        Value::Str(_) => path::join([p.clone(), Value::str("..")]),
        _ => return None,
    };
    start_at(&match v {
        JsVal::Path(a, _) => JsVal::Path(*a, dir),
        JsVal::Url(a, _) => JsVal::Url(*a, dir),
        JsVal::Resolved(l, _) => JsVal::Resolved(l.clone(), dir),
        _ => return None,
    })
}

fn as_val(v: &JsVal) -> Option<Value> {
    match v {
        JsVal::Str(x) => Some(x.clone()),
        JsVal::Unknown(r) => Some(Value::Unknown { reason: r.clone() }),
        _ => None,
    }
}

/// [`path::normalize`]: normalizes a path value (`anchored`: relative to an anchor).
pub fn normalize_value(v: Value, anchored: bool) -> Value {
    path::normalize(v, anchored)
}

fn path_join(args: &[JsVal]) -> Option<JsVal> {
    let Some((first, rest)) = args.split_first() else {
        return Some(JsVal::Str(Value::str(".")));
    };
    let rest: Vec<Value> = rest.iter().map(as_val).collect::<Option<_>>()?;
    // `path.join` keeps the `/` its last segment ends with (`path.join(dir,
    // 'sub/')` is `dir/sub/`): what is appended to the result, or resolved
    // against it as a URL or by `createRequire`, lands inside that directory.
    let slash = match rest.iter().rev().find(|v| v.as_str() != Some("")) {
        Some(Value::Str(s)) => s.ends_with('/'),
        Some(_) => false,
        None => {
            matches!(first, JsVal::Path(_, Value::Str(s)) | JsVal::Resolved(_, Value::Str(s)) if s.ends_with('/'))
        }
    };
    let joined = |v: &Value| {
        let j = normalize_value(
            path::join(std::iter::once(v.clone()).chain(rest.iter().cloned())),
            true,
        );
        match j {
            Value::Str(s) if slash => Value::Str(format!("{s}/")),
            j => j,
        }
    };
    match first {
        JsVal::Path(a, v) => Some(JsVal::Path(*a, joined(v))),
        JsVal::Resolved(m, v) => Some(JsVal::Resolved(m.clone(), joined(v))),
        JsVal::File => {
            // `path.join(__filename, '..', x)` with an unknown file name.
            let mut it = rest.into_iter();
            let r0 = it.next()?;
            let s = r0.as_str()?.strip_prefix("..")?;
            if !(s.is_empty() || s.starts_with('/')) {
                return None;
            }
            Some(JsVal::Path(
                Anchor::FileDir,
                normalize_value(path::join(std::iter::once(Value::str(s)).chain(it)), true),
            ))
        }
        other => {
            let v = as_val(other)?;
            Some(JsVal::Str(normalize_value(
                path::join(std::iter::once(v).chain(rest)),
                false,
            )))
        }
    }
}

fn path_resolve(args: &[JsVal]) -> Option<JsVal> {
    let mut base = None;
    for (i, a) in args.iter().enumerate().rev() {
        match a {
            JsVal::Path(..) | JsVal::Resolved(..) => {
                base = Some(i);
                break;
            }
            JsVal::Str(v) if path::starts_absolute(v) => {
                base = Some(i);
                break;
            }
            JsVal::Str(_) | JsVal::Unknown(_) => {}
            _ => return None,
        }
    }
    match base {
        Some(i) => match &args[i] {
            // `path.resolve` never ends with a `/`.
            JsVal::Path(..) | JsVal::Resolved(..) => match path_join(&args[i..])? {
                JsVal::Path(a, v) => Some(JsVal::Path(a, normalize_value(v, true))),
                JsVal::Resolved(m, v) => Some(JsVal::Resolved(m, normalize_value(v, true))),
                v => Some(v),
            },
            _ => {
                let vals: Vec<Value> = args[i..].iter().map(as_val).collect::<Option<_>>()?;
                Some(JsVal::Str(normalize_value(path::join(vals), false)))
            }
        },
        None => {
            let vals: Vec<Value> = args.iter().map(as_val).collect::<Option<_>>()?;
            Some(JsVal::Path(
                Anchor::Cwd,
                normalize_value(path::join(vals), true),
            ))
        }
    }
}

fn path_dirname(a: &JsVal) -> Option<JsVal> {
    match a {
        JsVal::Path(an, v) => Some(JsVal::Path(*an, path::parent(v.clone())?)),
        JsVal::Resolved(m, v) => Some(JsVal::Resolved(
            m.clone(),
            normalize_value(path::join([v.clone(), Value::str("..")]), true),
        )),
        JsVal::File => Some(JsVal::dirname()),
        JsVal::Str(v) => Some(JsVal::Str(path::dirname(v.clone(), false)?)),
        _ => None,
    }
}

fn path_basename(a: &JsVal, ext: Option<&JsVal>) -> Option<JsVal> {
    let mut s = match a {
        JsVal::Path(_, v) => path::basename(&normalize_value(v.clone(), true))?,
        JsVal::Str(v) => path::basename(v)?,
        _ => return None,
    };
    match ext {
        None => {}
        Some(JsVal::Str(Value::Str(e))) => {
            if s.len() > e.len() && s.ends_with(e.as_str()) {
                s.truncate(s.len() - e.len());
            }
        }
        Some(_) => return None,
    }
    Some(JsVal::Str(Value::Str(s)))
}

fn path_extname(a: &JsVal) -> Option<JsVal> {
    let v = match a {
        JsVal::Path(_, v) => normalize_value(v.clone(), true),
        JsVal::Str(v) => v.clone(),
        _ => return None,
    };
    Some(JsVal::Str(Value::Str(path::extname(&v)?)))
}

fn path_relative(a: &JsVal, b: &JsVal) -> Option<JsVal> {
    let r = match (a, b) {
        (JsVal::Str(Value::Str(x)), JsVal::Str(Value::Str(y))) => path::relative(x, y)?,
        (JsVal::Path(p, x), JsVal::Path(q, y)) if p == q => {
            let x = normalize_value(x.clone(), true);
            let y = normalize_value(y.clone(), true);
            path::relative(x.as_str()?, y.as_str()?)?
        }
        _ => return None,
    };
    Some(JsVal::Str(Value::Str(r)))
}

fn strip_file_url(s: &str) -> Option<String> {
    let rest = s.strip_prefix("file://")?;
    rest.starts_with('/').then(|| rest.to_owned())
}

/// `new URL(input, base?)`. `None` when the result is not a file URL the
/// evaluator can express.
///
/// A percent-escape in the input is decoded when the URL is used as a file
/// (`fs.readFile(url)`, `fileURLToPath(url)`) and kept by `url.pathname`: both
/// paths are alternatives.
pub fn url_ctor(args: &[JsVal], sc: &Scope<'_, '_>) -> Option<JsVal> {
    if let (Some(JsVal::Str(Value::Str(s))), Some(base)) = (args.first(), args.get(1))
        && let Some(decoded) = percent_decoded(s)
    {
        let a = url_ctor_raw(&[JsVal::Str(Value::Str(decoded)), base.clone()], sc);
        let b = url_ctor_raw(args, sc);
        return match (a, b) {
            (Some(a), Some(b)) => Some(alts([a, b])),
            (a, b) => a.or(b),
        };
    }
    url_ctor_raw(args, sc)
}

/// `s` with its percent-escapes decoded, when that changes it and gives valid
/// UTF-8.
fn percent_decoded(s: &str) -> Option<String> {
    if !s.contains('%') {
        return None;
    }
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push(h * 16 + l);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().filter(|d| d != s)
}

fn url_ctor_raw(args: &[JsVal], _sc: &Scope<'_, '_>) -> Option<JsVal> {
    let input = args.first()?;
    let Some(base) = args.get(1) else {
        return match input {
            JsVal::FileUrl | JsVal::Url(..) | JsVal::Resolved(..) => Some(input.clone()),
            JsVal::Str(Value::Str(s)) => Some(JsVal::Str(Value::Str(strip_file_url(s)?))),
            _ => None,
        };
    };
    /// The directory a relative input resolves in.
    enum Dir {
        Anchored(Anchor, Value),
        /// Next to a resolved module: the relative part is joined onto the
        /// resolved file, so the directory of a file is `..` from it.
        Next(Located, Value),
    }
    let dir = match base {
        JsVal::FileUrl => Dir::Anchored(Anchor::FileDir, Value::str("")),
        // A base URL that ends with `/` is a directory: relative inputs resolve inside it.
        JsVal::Url(a, v) if ends_with_slash(v) => {
            Dir::Anchored(*a, normalize_value(v.clone(), true))
        }
        JsVal::Url(a, v) => {
            Dir::Anchored(*a, path::dirname(normalize_value(v.clone(), true), true)?)
        }
        JsVal::Resolved(m, v) if ends_with_slash(v) => {
            Dir::Next(m.clone(), normalize_value(v.clone(), true))
        }
        JsVal::Resolved(m, v) => Dir::Next(
            m.clone(),
            normalize_value(path::join([v.clone(), Value::str("..")]), true),
        ),
        _ => return None,
    };
    let rel = url_path(as_val(input)?);
    let first_lit = match &rel {
        Value::Str(s) => s.as_str(),
        Value::Pattern(p) => match p.first() {
            Some(PatPart::Lit(s)) => s.as_str(),
            _ => return None,
        },
        _ => return None,
    };
    if first_lit.starts_with("file:") {
        return match &rel {
            Value::Str(s) => Some(JsVal::Str(Value::Str(strip_file_url(s)?))),
            _ => None,
        };
    }
    // A scheme ends the first segment (`https:`, `data:`); a `:` later is part of a
    // name (`./logs/10:30.txt`).
    if first_lit.starts_with("//") || first_lit.split('/').next().is_some_and(|s| s.contains(':')) {
        return None; // another scheme or host
    }
    if first_lit.starts_with('/') {
        return Some(JsVal::Str(rel));
    }
    // Keep a trailing `/` (a directory URL): it decides how the result works as a
    // base.
    let directory = ends_with_slash(&rel)
        || matches!(rel.as_str(), Some("." | ".."))
        || rel
            .as_str()
            .is_some_and(|s| s.ends_with("/.") || s.ends_with("/.."));
    let slashed = |joined: Value| {
        if directory {
            joined.concat(Value::str("/"))
        } else {
            joined
        }
    };
    Some(match dir {
        Dir::Anchored(anchor, dir) => JsVal::Url(
            anchor,
            slashed(normalize_value(path::join([dir, rel]), true)),
        ),
        Dir::Next(m, dir) => {
            JsVal::Resolved(m, slashed(normalize_value(path::join([dir, rel]), true)))
        }
    })
}

/// The path of a URL reference: without its query and fragment
/// (`./a.json?raw`).
fn url_path(v: Value) -> Value {
    cut_query(v, true)
}

/// A module specifier without its query (`./x.js?v=1`). A `#` stays: in a path
/// given to `require` it is part of a name (`channels/#db`), at the start it is
/// a subpath import; only a `file:` URL has a fragment.
fn module_path(v: Value) -> Value {
    let url = match &v {
        Value::Str(s) => s.starts_with("file:"),
        Value::Pattern(parts) => {
            matches!(parts.first(), Some(PatPart::Lit(s)) if s.starts_with("file:"))
        }
        _ => false,
    };
    cut_query(v, url)
}

/// `v` up to its first `?` (and `#`, with `fragment`); a pattern ends there.
fn cut_query(v: Value, fragment: bool) -> Value {
    let cut = |s: &str| -> usize {
        s.find(|c| c == '?' || (fragment && c == '#'))
            .unwrap_or(s.len())
    };
    match v {
        Value::Str(s) => {
            let end = cut(&s);
            if end == s.len() {
                Value::Str(s)
            } else {
                Value::Str(s[..end].to_owned())
            }
        }
        Value::Pattern(parts) => {
            if !parts
                .iter()
                .any(|p| matches!(p, PatPart::Lit(s) if cut(s) < s.len()))
            {
                return Value::Pattern(parts);
            }
            let mut out = Vec::with_capacity(parts.len());
            for p in parts {
                match p {
                    PatPart::Lit(s) => {
                        let end = cut(&s);
                        if end > 0 {
                            out.push(PatPart::Lit(s[..end].to_owned()));
                        }
                        if end < s.len() {
                            break;
                        }
                    }
                    w => out.push(w),
                }
            }
            Value::pattern(out)
        }
        v => v,
    }
}

/// Whether some alternative of `v` is a path below the temporary directory.
pub fn in_tmp(v: &JsVal) -> bool {
    match v {
        JsVal::Path(Anchor::Tmp, _) | JsVal::Url(Anchor::Tmp, _) => true,
        JsVal::Alts(vs) => vs.iter().any(in_tmp),
        _ => false,
    }
}

/// Whether every alternative of `v` is a string or a path whose text ends with
/// the literal extension `.json` after a file name (`dir + '/a.json'`, ``
/// `${dir}/${name}.json` ``; not `dir + '/.json'`, which has no extension, and
/// not `name + ext`).
pub fn ends_with_json(v: &JsVal) -> bool {
    fn text(s: &str, whole: bool) -> bool {
        match s.strip_suffix(".json") {
            // The file name before the extension: literal, or what the hole before this
            // literal stands for.
            Some(stem) => match stem.as_bytes().last() {
                Some(b) => !matches!(b, b'/' | b'\\'),
                None => !whole,
            },
            None => false,
        }
    }
    fn value(v: &Value) -> bool {
        match v {
            Value::Str(s) => text(s, true),
            Value::Pattern(p) => {
                matches!(p.last(), Some(PatPart::Lit(s)) if text(s, p.len() == 1))
            }
            Value::Alternatives(a) => !a.is_empty() && a.iter().all(value),
            Value::Unknown { .. } => false,
        }
    }
    match v {
        JsVal::Str(val) | JsVal::Path(_, val) => value(val),
        JsVal::Alts(vs) => !vs.is_empty() && vs.iter().all(ends_with_json),
        _ => false,
    }
}

/// Whether a URL path value ends with `/`.
fn ends_with_slash(v: &Value) -> bool {
    match v {
        Value::Str(s) => s.ends_with('/'),
        Value::Pattern(p) => matches!(p.last(), Some(PatPart::Lit(s)) if s.ends_with('/')),
        _ => false,
    }
}

/// `new URL(args...)`, distributing over alternative arguments.
pub fn new_url(args: Vec<JsVal>, sc: &Scope<'_, '_>) -> Option<JsVal> {
    distribute(args, |a| url_ctor(a, sc))
}

/// One alternative of a lowered value: the specifier text and its anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    /// Literal or pattern (never a leading wildcard).
    pub spec: Value,
    /// The anchor.
    pub base: Base,
    /// Relative to the process working directory.
    pub cwd: bool,
}

/// A value lowered for a call site.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lowered {
    /// Usable alternatives.
    pub leaves: Vec<Leaf>,
    /// Reasons of alternatives that were dropped (unknown or leading wildcard),
    /// without origin marks.
    pub dropped: Vec<Box<str>>,
    /// For each dropped alternative, where its unknown part comes from and a
    /// payload for the record detail ([`super::origin::combine`]).
    pub origins: Vec<(Origin, String)>,
    /// Number of alternatives before dropping.
    pub count: usize,
}

/// Wildcard reasons of a value.
pub fn wild_reasons(v: &Value, out: &mut Vec<Box<str>>) {
    match v {
        Value::Unknown { reason } => out.push(reason.clone()),
        Value::Pattern(p) => {
            for x in p {
                if let PatPart::Wild { reason } = x {
                    out.push(reason.clone());
                }
            }
        }
        Value::Alternatives(a) => a.iter().for_each(|x| wild_reasons(x, out)),
        Value::Str(_) => {}
    }
}

fn leading_wild(v: &Value) -> bool {
    match v {
        Value::Unknown { .. } => true,
        Value::Pattern(p) => matches!(p.first(), Some(PatPart::Wild { .. })),
        _ => false,
    }
}

fn prefix(p: &str, v: Value) -> Value {
    match v {
        Value::Str(s) => Value::Str(format!("{p}{s}")),
        Value::Pattern(parts) => {
            Value::pattern(std::iter::once(PatPart::Lit(p.to_owned())).chain(parts))
        }
        Value::Unknown { reason } => {
            Value::pattern([PatPart::Lit(p.to_owned()), PatPart::Wild { reason }])
        }
        a @ Value::Alternatives(_) => a,
    }
}

/// A file-relative path as a module specifier: `./x`, `../x`, `./`.
fn dotted(v: Value) -> Value {
    let first = match &v {
        Value::Str(s) => Some(s.as_str()),
        Value::Pattern(p) => match p.first() {
            Some(PatPart::Lit(s)) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    };
    match first {
        Some(s) if s == ".." || s.starts_with("../") => v,
        _ => prefix("./", v),
    }
}

/// Lowers a value to call-site alternatives for an API with base rule `rule`.
pub fn lower(v: JsVal, rule: BaseRule) -> Lowered {
    let mut out = Lowered::default();
    for leaf in split_alts(v) {
        out.count += 1;
        match lower_leaf(leaf, rule) {
            Ok(l) => out.leaves.push(l),
            Err(Some((r, o))) => {
                out.dropped.push(origin::strip(&r).into());
                out.origins.push(o);
            }
            // Never a file (the empty string) or outside the repository (a temporary
            // directory): no reference and no record.
            Err(None) => {}
        }
    }
    out
}

/// A dropped alternative: its reason and origin. `None`: dropped without a
/// record.
type Dropped = Option<(Box<str>, (Origin, String))>;

fn reasons_of(v: &Value) -> Dropped {
    let mut rs = Vec::new();
    wild_reasons(v, &mut rs);
    let o = origin::combine(rs.iter().map(|r| &**r));
    Some((rs.join(", ").into(), o))
}

fn dropped(r: Box<str>) -> Dropped {
    let o = origin::combine([&*r]);
    Some((r, o))
}

/// A dropped alternative that is not a path string at all (no origin).
fn not_a_path(r: &str) -> Dropped {
    Some((r.into(), (Origin::Unknown, String::new())))
}

fn lower_leaf(v: JsVal, rule: BaseRule) -> Result<Leaf, Dropped> {
    // The query and fragment of a URL or a module specifier select no other file
    // (`import(url + '?t=' + Date.now())`): with them a pattern matches nothing.
    let v = match v {
        JsVal::Url(a, val) => JsVal::Url(a, url_path(val)),
        JsVal::Resolved(m, val) if rule == BaseRule::Module => JsVal::Resolved(m, url_path(val)),

        JsVal::Path(a, val) if rule == BaseRule::Module => JsVal::Path(a, cut_query(val, false)),
        JsVal::Str(val) if rule == BaseRule::Module => JsVal::Str(module_path(val)),
        v => v,
    };
    match v {
        JsVal::Str(val) => {
            let val = match (&val, rule) {
                (Value::Str(s), BaseRule::Path) if s.starts_with("file://") => {
                    Value::Str(strip_file_url(s).ok_or_else(|| not_a_path(s))?)
                }
                _ => val,
            };
            if leading_wild(&val) {
                return Err(reasons_of(&val));
            }
            if val.as_str() == Some("") {
                return Err(None);
            }
            let abs = path::starts_absolute(&val);
            match rule {
                BaseRule::Module => Ok(Leaf {
                    spec: val,
                    base: Base::FileDir,
                    cwd: false,
                }),
                BaseRule::Path if abs => Ok(Leaf {
                    spec: val,
                    base: Base::FileDir,
                    cwd: false,
                }),
                BaseRule::Path => Ok(Leaf {
                    spec: val,
                    base: Base::Cwd,
                    cwd: true,
                }),
                // Not used by the JS table; anchored at the file's directory.
                BaseRule::FileDir | BaseRule::PackageRoot => Ok(Leaf {
                    spec: val,
                    base: Base::FileDir,
                    cwd: false,
                }),
            }
        }
        JsVal::Path(Anchor::Tmp, _) | JsVal::Url(Anchor::Tmp, _) => Err(None),
        JsVal::Path(an, val) | JsVal::Url(an, val) => {
            let val = normalize_value(val, true);
            if an == Anchor::Cwd && leading_wild(&val) {
                return Err(reasons_of(&val));
            }
            let spec = match rule {
                BaseRule::Module => dotted(val),
                BaseRule::Path | BaseRule::FileDir | BaseRule::PackageRoot => match &val {
                    Value::Str(s) if s.is_empty() => Value::str("."),
                    _ if leading_wild(&val) => prefix("./", val),
                    _ => val,
                },
            };
            Ok(Leaf {
                spec,
                base: match an {
                    Anchor::FileDir | Anchor::Tmp => Base::FileDir,
                    Anchor::Cwd => Base::Cwd,
                },
                cwd: an == Anchor::Cwd,
            })
        }
        // A module load of it is the file itself (`import(pathToFileURL(root + '/x.js'))`),
        // a read is the same path: the resolver finds the module, then joins the rest.
        JsVal::Resolved(m, v) => Ok(Leaf {
            spec: match normalize_value(v, true) {
                Value::Str(s) if s.is_empty() => Value::Str(m.to_string()),
                v => prefix(&format!("{m}/"), v),
            },
            base: Base::Language(RESOLVED_BASE),
            cwd: false,
        }),
        JsVal::Unknown(r) => Err(dropped(r)),
        JsVal::File => Err(not_a_path("__filename")),
        JsVal::FileUrl => Err(not_a_path("import.meta.url")),
        JsVal::Obj(_) | JsVal::Func(_) | JsVal::Arr(_) | JsVal::Alts(_) => {
            Err(not_a_path("not a string"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::{Dialect, Lexer};

    fn toks(src: &str) -> Vec<Token> {
        let mut lx = Lexer::new(src.as_bytes(), Dialect::JS);
        let mut out = Vec::new();
        loop {
            let t = lx.next_token();
            if t.kind == TokKind::Eof {
                break;
            }
            out.push(t);
        }
        out
    }

    fn ev_with(
        src: &str,
        consts: &HashMap<&str, (JsVal, bool)>,
        apis: &[ApiBinding<'_>],
    ) -> Outcome {
        let t = toks(src);
        let sc = Scope {
            src: src.as_bytes(),
            consts,
            apis,
            loaders: &[],
            file_name: Some("main.js"),
        };
        evaluate(&sc, &t)
    }

    fn path_apis() -> Vec<ApiBinding<'static>> {
        vec![
            ApiBinding {
                local: "path",
                module: "path".into(),
                member: None,
                depth: 0,
            },
            ApiBinding {
                local: "join",
                module: "path".into(),
                member: Some("join".into()),
                depth: 0,
            },
            ApiBinding {
                local: "fileURLToPath",
                module: "url".into(),
                member: Some("fileURLToPath".into()),
                depth: 0,
            },
            ApiBinding {
                local: "pathToFileURL",
                module: "url".into(),
                member: Some("pathToFileURL".into()),
                depth: 0,
            },
        ]
    }

    /// Renders the lowered value: `base:spec` per leaf, `!reason` per dropped
    /// leaf.
    fn show(src: &str, rule: BaseRule) -> String {
        let consts = HashMap::new();
        let apis = path_apis();
        let o = ev_with(src, &consts, &apis);
        let l = lower(o.val, rule);
        let mut parts: Vec<String> = l
            .leaves
            .iter()
            .map(|x| {
                let b = match x.base {
                    Base::FileDir => "dir",
                    Base::Cwd => "cwd",
                    _ => "?",
                };
                format!("{b}:{}", x.spec.to_specifier())
            })
            .collect();
        parts.extend(l.dropped.iter().map(|r| format!("!{r}")));
        if o.env {
            parts.push("env".into());
        }
        parts.join(" ")
    }

    fn m(src: &str) -> String {
        show(src, BaseRule::Module)
    }

    fn p(src: &str) -> String {
        show(src, BaseRule::Path)
    }

    #[test]
    fn literals_and_concat() {
        assert_eq!(m("'./a'"), "dir:./a");
        assert_eq!(m("'u' + 'rl'"), "dir:url");
        assert_eq!(m("('./a')"), "dir:./a");
        assert_eq!(m("`./x/${'y'}.js`"), "dir:./x/y.js");
        assert_eq!(m("`./x/${name}.js`"), "dir:./x/*.js");
        assert_eq!(m("'./plugins/' + plugin"), "dir:./plugins/*");
        assert_eq!(m("'./a' + 1"), "dir:./a1");
        assert_eq!(
            m("String.raw`./snippets/x/tracer_logger.js`"),
            "dir:./snippets/x/tracer_logger.js"
        );
        assert_eq!(m(r"String.raw`a\b`"), r"dir:a\b");
        assert_eq!(m("'./a'.concat('b')"), "dir:./ab");
        // Leading wildcards are never patterns.
        assert_eq!(m("name + '.js'"), "!name");
        assert_eq!(m("name"), "!name");
        assert_eq!(m("a.b.c"), "!a.b.c");
        assert_eq!(m("foo()"), "!foo()");
        assert_eq!(m("x => x"), "!x => x");
    }

    #[test]
    fn alternatives() {
        assert_eq!(
            m("inAWSLambda && !usingLambdaExtension ? './exporters/log' : './exporters/agent'"),
            "dir:./exporters/log dir:./exporters/agent"
        );
        assert_eq!(m("a || './b'"), "dir:./b !a");
        assert_eq!(m("process.env.X ?? './d'"), "dir:./d !process.env.X env");
        assert_eq!(m("'./' + (a ? 'x' : 'y') + '.js'"), "dir:./x.js dir:./y.js");
        assert_eq!(m("c ? path.join(__dirname, 'a') : 'b'"), "dir:./a dir:b");
    }

    #[test]
    fn file_anchors() {
        assert_eq!(m("__dirname + '/x'"), "dir:./x");
        assert_eq!(m("__dirname + '/../x'"), "dir:../x");
        assert_eq!(
            p("path.join(__dirname, '../data', name + '.json')"),
            "dir:../data/*.json"
        );
        assert_eq!(p("path.join(__dirname, 'data')"), "dir:data");
        assert_eq!(p("join(__dirname)"), "dir:.");
        assert_eq!(p("path.join(__dirname, x)"), "dir:./*");
        assert_eq!(m("path.resolve(__dirname, './lib/a.js')"), "dir:./lib/a.js");
        assert_eq!(p("path.dirname(__filename)"), "dir:.");
        assert_eq!(p("path.join(path.dirname(__filename), 'a')"), "dir:a");
        assert_eq!(p("path.resolve(__dirname, '..', 'x')"), "dir:../x");
        assert_eq!(p("path.posix.join(__dirname, 'x')"), "dir:x");
        assert_eq!(p("(0, path.join)(__dirname, 'x')"), "dir:x");
        assert_eq!(p("[__dirname, 'x'].join('/')"), "dir:x");
        assert_eq!(p("__filename"), "dir:main.js");
        assert_eq!(m("__dirname + 'x'"), "!__dirname + 'x'");
    }

    #[test]
    fn urls() {
        assert_eq!(p("new URL('./x.wasm', import.meta.url)"), "dir:x.wasm");
        assert_eq!(
            p("new URL('x.wasm', import.meta.url).pathname"),
            "dir:x.wasm"
        );
        assert_eq!(
            p("fileURLToPath(new URL('../a', import.meta.url))"),
            "dir:../a"
        );
        assert_eq!(p("path.dirname(fileURLToPath(import.meta.url))"), "dir:.");
        assert_eq!(p("import.meta.dirname + '/x'"), "dir:x");
        assert_eq!(
            p("new URL(`./l/${x}.json`, import.meta.url).href"),
            "dir:l/*.json"
        );
        assert_eq!(
            p("new URL('https://example.com/x', import.meta.url)"),
            "!new URL('https://example.com/x', import.meta.url)"
        );
        assert_eq!(m("new URL('./w.js', import.meta.url)"), "dir:./w.js");
    }

    #[test]
    fn cwd_and_env() {
        assert_eq!(p("'./data.json'"), "cwd:./data.json");
        assert_eq!(p("'/etc/x'"), "dir:/etc/x");
        assert_eq!(p("path.join(process.cwd(), 'x')"), "cwd:x");
        assert_eq!(p("path.resolve('x')"), "cwd:x");
        assert_eq!(p("path.resolve('/a', 'b')"), "dir:/a/b");
        assert_eq!(p("path.resolve(x)"), "!x");
        assert_eq!(p("process.env.HOME + '/x'"), "!process.env.HOME env");
        assert_eq!(p("'./a/' + process.env['L']"), "cwd:./a/* env");
        assert_eq!(p("require.resolve('./a.json')"), "dir:a.json");
        assert_eq!(
            p("require('url').pathToFileURL(__filename).href"),
            "dir:main.js"
        );
        assert_eq!(
            p("new URL('./x.wasm', require('url').pathToFileURL(__filename))"),
            "dir:x.wasm"
        );
        assert_eq!(p("require('node:path').join(__dirname, 'y')"), "dir:y");
        // A package: anchored at the file the resolver finds.
        assert_eq!(p("require.resolve('pkg')"), "?:require.resolve('pkg')");
        assert_eq!(
            p("path.join(require.resolve('pkg'), '../x.wasm')"),
            "?:require.resolve('pkg')/../x.wasm"
        );
        // A module load of it is that file.
        assert_eq!(
            m("path.join(require.resolve('pkg'), '../x')"),
            "?:require.resolve('pkg')/../x"
        );
        assert_eq!(m("require.resolve('pkg')"), "?:require.resolve('pkg')");
        // Options say where the search starts: not known here.
        assert_eq!(
            m("require.resolve('pkg', { paths: [__dirname] })"),
            "!require.resolve('pkg', { paths: [__dirname] })"
        );
    }

    #[test]
    fn next_to_a_resolved_module() {
        // The URL functions keep the file; `new URL(rel, base)` resolves in the
        // directory of a file and inside a directory URL.
        assert_eq!(
            m("new URL('inner.js', new URL('.', import.meta.resolve('eve'))).href"),
            "?:import.meta.resolve('eve')/../inner.js"
        );
        assert_eq!(
            m("new URL('./inner.js', import.meta.resolve('eve'))"),
            "?:import.meta.resolve('eve')/../inner.js"
        );
        assert_eq!(
            m("new URL('dist/', pathToFileURL(require.resolve('eve/package.json')))"),
            "?:require.resolve('eve/package.json')/../dist"
        );
        assert_eq!(
            m("new URL(p, new URL('dist/', pathToFileURL(require.resolve('eve/package.json'))))"),
            "!new URL(p, new URL('dist/', pathToFileURL(require.resolve('eve/package.json'))))"
        );
        assert_eq!(
            m(
                "pathToFileURL(path.join(path.dirname(require.resolve('eve/package.json')), \
                 'dist/x.js')).href"
            ),
            "?:require.resolve('eve/package.json')/../dist/x.js"
        );
        assert_eq!(
            m("fileURLToPath(new URL('../x.js', import.meta.resolve('eve')))"),
            "?:import.meta.resolve('eve')/../../x.js"
        );
        assert_eq!(
            m("pathToFileURL(require.resolve('eve')).pathname"),
            "?:require.resolve('eve')"
        );
        // Another scheme, a host or an absolute path is not next to the module.
        assert_eq!(
            m("new URL('https://x.test/a.js', import.meta.resolve('eve'))"),
            "!new URL('https://x.test/a.js', import.meta.resolve('eve'))"
        );
        assert_eq!(
            m("new URL('/abs/a.js', import.meta.resolve('eve'))"),
            "dir:/abs/a.js"
        );
        // The relative part is steps, not text: only a new segment can be appended.
        assert_eq!(
            m("path.dirname(require.resolve('eve')) + '/x.js'"),
            "?:require.resolve('eve')/../x.js"
        );
        assert_eq!(
            m("`${path.dirname(require.resolve('eve'))}/x.js`"),
            "?:require.resolve('eve')/../x.js"
        );
        assert_eq!(
            m("path.dirname(require.resolve('eve')) + 'x.js'"),
            "!path.dirname(require.resolve('eve')) + 'x.js'"
        );
        assert_eq!(
            m("new URL('.', import.meta.resolve('eve')) + 'x.js'"),
            "?:import.meta.resolve('eve')/../x.js"
        );
        // `path.join` keeps a trailing `/`, `path.resolve` and `path.dirname` drop it.
        assert_eq!(
            m("new URL('a.js', pathToFileURL(path.join(__dirname, 'fx/')))"),
            "dir:./fx/a.js"
        );
        assert_eq!(
            m("new URL('a.js', pathToFileURL(path.resolve(__dirname, 'fx/')))"),
            "dir:./a.js"
        );
        assert_eq!(m("path.join(__dirname, 'fx/') + 'a.js'"), "dir:./fx/a.js");
        assert_eq!(p("path.dirname(path.join(__dirname, 'fx/'))"), "dir:.");
    }

    #[test]
    fn located_text() {
        let eve = Located::new("eve/package.json", false).unwrap();
        let dep = Located::from_start("dep", Start::Next(eve.clone(), "..".into())).unwrap();
        let far = Located::from_start("x", Start::Next(dep.clone(), "".into())).unwrap();
        let dir = Located::from_start("@s/dep/sub", Start::Dir("../other".into())).unwrap();
        for (l, text) in [
            (&eve, "require.resolve('eve/package.json')"),
            (
                &dep,
                "require.resolve('dep', from require.resolve('eve/package.json')/'..')",
            ),
            (
                &far,
                "require.resolve('x', from require.resolve('dep', from \
                 require.resolve('eve/package.json')/'..'))",
            ),
            (&dir, "require.resolve('@s/dep/sub', from '../other')"),
        ] {
            assert_eq!(l.to_string(), text);
            assert_eq!(
                Located::parse(&format!("{text}/../a/b.js")),
                Some((l.clone(), "/../a/b.js"))
            );
            assert_eq!(Located::parse(text), Some((l.clone(), "")));
        }
        let esm = Located::new("eve", true).unwrap();
        assert_eq!(
            Located::parse("import.meta.resolve('eve')/x"),
            Some((esm, "/x"))
        );
        // The file's own directory is no start.
        assert_eq!(
            Located::from_start("dep", Start::Dir("".into())),
            Located::new("dep", false)
        );
        // Not a package, or text that would have two readings.
        for bad in ["./x", "/abs", "node:fs", "file:///x", "a'b", ""] {
            assert_eq!(Located::new(bad, false), None, "{bad}");
        }
        assert_eq!(Located::from_start("#imp", Start::Dir("a".into())), None);
        assert_eq!(Located::from_start("dep", Start::Dir("a'b".into())), None);
        assert_eq!(
            Located::from_start("dep", Start::Next(eve.clone(), "a'b".into())),
            None
        );
        // Nesting is bounded.
        assert_eq!(
            Located::from_start("y", Start::Next(far.clone(), "".into())),
            None
        );
        for bad in [
            "require.resolve('dep'",
            "require.resolve('dep', from )",
            "require.resolve('dep', from '')",
            "require.resolve('dep', from 'a'",
            "import.meta.resolve('dep', from 'a')",
            "resolve('dep')",
        ] {
            assert_eq!(Located::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn searching_from_another_place() {
        let dep = JsVal::Str(Value::str("dep"));
        let rel = JsVal::Str(Value::str("./x.js"));
        let dir = Start::Dir("../other".into());
        assert_eq!(
            resolve_from(&rel, &dir),
            Some(JsVal::Path(Anchor::FileDir, Value::str("../other/x.js")))
        );
        let Some(JsVal::Resolved(l, v)) = resolve_from(&dep, &dir) else {
            panic!("not resolved");
        };
        assert_eq!(l.to_string(), "require.resolve('dep', from '../other')");
        assert_eq!(v, Value::str(""));
        let eve = Located::new("eve", false).unwrap();
        let next = Start::Next(eve.clone(), "..".into());
        assert_eq!(
            resolve_from(&rel, &next),
            Some(JsVal::Resolved(eve.clone(), Value::str("../x.js")))
        );
        for bad in ["/abs", "node:fs", "#imp", ""] {
            assert_eq!(
                resolve_from(&JsVal::Str(Value::str(bad)), &dir),
                None,
                "{bad}"
            );
        }
        assert_eq!(resolve_from(&JsVal::unknown("x"), &dir), None);
        // `createRequire(file)` searches from the file's directory,
        // `createRequire(dir/)` from the directory.
        let at = |v: JsVal| start_of_file(&v);
        assert_eq!(
            at(JsVal::Path(
                Anchor::FileDir,
                Value::str("../other/package.json")
            )),
            Some(Start::Dir("../other".into()))
        );
        assert_eq!(
            at(JsVal::Path(Anchor::FileDir, Value::str("../other/"))),
            Some(Start::Dir("../other".into()))
        );
        assert_eq!(
            at(JsVal::Path(Anchor::FileDir, Value::str("main.js"))),
            Some(Start::Dir("".into()))
        );
        assert_eq!(
            at(JsVal::Path(Anchor::FileDir, Value::str(""))),
            Some(Start::Dir("..".into()))
        );
        assert_eq!(
            at(JsVal::Resolved(eve.clone(), Value::str(""))),
            Some(Start::Next(eve.clone(), "..".into()))
        );
        assert_eq!(at(JsVal::Path(Anchor::Cwd, Value::str("x/y.js"))), None);
        assert_eq!(at(JsVal::Str(Value::str("x/y.js"))), None);
        assert_eq!(at(JsVal::Path(Anchor::FileDir, Value::unknown("x"))), None);
    }

    #[test]
    fn path_functions() {
        assert_eq!(p("path.basename('/a/b.js')"), "cwd:b.js");
        assert_eq!(m("path.basename('/a/b.js', '.js')"), "dir:b");
        assert_eq!(m("path.extname('a/b.json')"), "dir:.json");
        assert_eq!(m("path.extname(x + '.json')"), "dir:.json");
        assert_eq!(m("path.relative('/a/b', '/a/c/d')"), "dir:../c/d");
        assert_eq!(m("path.normalize('./a/../b')"), "dir:b");
        assert_eq!(m("path.join('a', '..', '..', 'b')"), "dir:../b");
        assert_eq!(m("path.join('/a', '..', '..', 'b')"), "dir:/b");
        assert_eq!(m("path.join()"), "dir:.");
    }

    #[test]
    fn constants_and_bindings() {
        let mut consts = HashMap::new();
        consts.insert(
            "DIR",
            (JsVal::Path(Anchor::FileDir, Value::str("assets")), false),
        );
        consts.insert("EXT", (JsVal::Str(Value::str(".json")), false));
        consts.insert("ENV", (JsVal::unknown("process.env.X"), true));
        let apis = path_apis();
        let o = ev_with("path.join(DIR, name + EXT)", &consts, &apis);
        let l = lower(o.val, BaseRule::Path);
        assert_eq!(l.leaves[0].spec.to_specifier().to_string(), "assets/*.json");
        let o = ev_with("ENV + '/x'", &consts, &apis);
        assert!(o.env);
        // An unbound receiver is assumed to be `path` only with an anchored first
        // argument, and the assumption is reported.
        let o = ev_with("p.join(__dirname, 'x')", &consts, &[]);
        assert_eq!(o.val, JsVal::Path(Anchor::FileDir, Value::str("x")));
        assert!(o.guess.unwrap().contains("assumed to be `path.join`"));
        let o = ev_with("p.join('a', 'b')", &consts, &[]);
        assert_eq!(o.val, JsVal::unknown("p.join('a', 'b')"));
        assert!(o.guess.is_none());
    }

    #[test]
    fn caps() {
        let deep = format!("{}'a'{}", "(".repeat(200), ")".repeat(200));
        let consts = HashMap::new();
        let o = ev_with(&deep, &consts, &[]);
        assert!(!o.complete);
        let many = (0..20)
            .map(|i| format!("c{i} ? 'a{i}' : "))
            .collect::<String>()
            + "'z'";
        let o = ev_with(&many, &consts, &[]);
        assert!(o.complete);
        assert!(split_alts(o.val).len() <= MAX_ALTERNATIVES);
        let o = ev_with("[a, b] .join(x)", &consts, &[]);
        assert!(matches!(o.val, JsVal::Unknown(_)));
    }

    /// Found by the flow oracle with the value-flow engine off: what the token
    /// evaluator claims alone.
    #[test]
    fn numbers_and_falsy_operands() {
        // `n + 1` is a sum, not `n` followed by `1`.
        assert_eq!(
            p("path.join(__dirname, 'f' + (n + 1) + '.json')"),
            "dir:f*.json"
        );
        assert_eq!(p("path.join(__dirname, 'f' + (1 + 1))"), "dir:f*");
        assert_eq!(
            p("path.join(__dirname, `page-${page + 1}.json`)"),
            "dir:page-*.json"
        );
        // Digits next to text are text.
        assert_eq!(p("path.join(__dirname, 'f' + 1 + 2)"), "dir:f12");
        // `a && b` is `b`, or the empty string for an empty `a`.
        assert_eq!(
            p("path.join(__dirname, 'x' + (s && '-on') + '.json')"),
            "dir:x-on.json dir:x.json"
        );
        assert_eq!(m("dev && './dev'"), "dir:./dev");
        // `join` writes `undefined` as the empty string.
        assert_eq!(
            p("[__dirname, 'fx', deep ? 'sub' : undefined, 'c.json'].join('/')"),
            "dir:fx/sub/c.json dir:fx/c.json"
        );
    }
}
