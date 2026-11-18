//! A small URL evaluator over lang-js tokens, shared by fetch discovery and
//! Playwright visit extraction.
//!
//! It answers "which URL strings can this expression produce?" as alternatives
//! of [`Piece`] lists (known text and wildcards). Beyond what the shared
//! partial evaluator ([`turborepo_trace_lang_js::extract::evaluator`]) handles,
//! it follows what URL-building code in apps and tests does:
//!
//! - string and template literals, `+`, `?:`, `??`, `||` (alternatives,
//!   capped);
//! - `const`/`let` bindings anywhere in the file (scope-insensitive: the first
//!   binding of a name wins), and constants imported from other first-party
//!   modules;
//! - calls of functions declared in the file or imported from first-party
//!   modules (their `return` expressions; parameters are unknown);
//! - methods of classes (page objects): `obj.path(...)` where `obj = new
//!   XPage(page)` or `obj: XPage`, `this.path()` inside a class, class fields
//!   (`readonly url = '/x'`), and inheritance;
//! - `new URL(x, base)` with `.pathname`/`.href`, `.toString()`, `String(x)`,
//!   `encodeURIComponent(x)` (an unknown value stays a wildcard), and query
//!   helpers (`withQuery(url, q)` → `url`);
//! - everything else through the shared evaluator (`path.join`, `String.raw`,
//!   ...).
//!
//! Evaluation is bounded (depth, alternatives) and never panics.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use turborepo_trace_core::lexkit::{EscapeRules, decode};
use turborepo_trace_lang_js::{
    Dialect,
    extract::evaluator::{self, JsVal, Scope},
    lexer::{Kw, Lexer, NEWLINE_BEFORE, TokKind, Token},
};

use super::{
    source::Source,
    urlpat::{Piece, reason_is_pathish},
};

/// Maximum alternatives per expression.
pub const MAX_ALTS: usize = 48;
const MAX_DEPTH: u32 = 80;
const MAX_TOKENS: usize = 300_000;

/// One alternative: a partially known string.
pub type Tpl = Vec<Piece>;

/// How an import binds a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Imported {
    /// `import x from`, `const x = require()`.
    Default,
    /// `import { a as x }`, `const { a: x } = require()`.
    Named(String),
    /// `import * as x`.
    Namespace,
}

/// One import binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// Local name.
    pub local: String,
    /// Module specifier.
    pub module: String,
    /// What is imported.
    pub imported: Imported,
}

/// A class with its methods.
#[derive(Clone, Debug, Default)]
pub struct ClassDef {
    /// Name.
    pub name: String,
    /// `extends` name.
    pub parent: Option<String>,
    /// Methods (and getters): name → `return` expression ranges.
    pub methods: Vec<(String, Vec<(usize, usize)>)>,
    /// Method bodies: name → token range.
    pub bodies: Vec<(String, (usize, usize))>,
    /// Fields with initializers: name → expression range.
    pub fields: Vec<(String, (usize, usize))>,
    /// The class body: the tokens of its `{` and `}`.
    pub body: (usize, usize),
}

/// A function parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Local name.
    pub name: String,
    /// Argument index.
    pub index: u8,
    /// Set for a destructured object parameter (`({ url }) => ...`): the field;
    /// for an array pattern (`([url, id]) => ...`) the element index.
    pub field: Option<String>,
    /// The default value expression (`opts = {}`, `{ method = 'GET' }`).
    pub default: Option<(usize, usize)>,
    /// A rest parameter (`...args`).
    pub rest: bool,
}

/// How a function is named.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum FnKind {
    /// No name: a callback.
    #[default]
    Anon,
    /// `function name() {}`.
    Decl,
    /// `const name = () => {}` / `name = function () {}`.
    Bound,
    /// An object property: `name: () => {}`.
    Prop,
    /// A method of a class or object literal: `name() {}`.
    Method,
}

/// A function (declaration, expression, arrow or method).
#[derive(Clone, Debug, Default)]
pub struct FnDef {
    /// Its name: a declaration, `const name = ...`, an object property `name:
    /// ...`, or a method.
    pub name: Option<String>,
    /// How it is named.
    pub kind: FnKind,
    /// Parameters.
    pub params: Vec<Param>,
    /// First token of the function.
    pub start: usize,
    /// Body token range.
    pub body: (usize, usize),
    /// The body is a block (`{ ... }`), not an arrow's expression.
    pub block: bool,
}

/// One `const|let|var` binding (a destructured name is one binding).
#[derive(Clone, Debug)]
pub struct Binding {
    /// Token of the bound name.
    pub at: usize,
    /// The `{` of the block the binding is in (`usize::MAX`: module scope).
    pub scope: usize,
    /// The initializer expression.
    pub init: Option<(usize, usize)>,
    /// For a destructured name: the field path into the initializer's value.
    pub path: Vec<String>,
    /// `let` or `var`: may be reassigned.
    pub mutable: bool,
}

/// An assignment to a plain identifier: `x = e` or `x += e`.
#[derive(Clone, Debug)]
pub struct Assign {
    /// Token of the assigned name.
    pub at: usize,
    /// The right-hand side.
    pub value: (usize, usize),
    /// `+=`.
    pub append: bool,
}

/// `export { a as b } from 'm'`, `export * from 'm'`, `export * as ns from
/// 'm'`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReExport {
    /// The exported name; `None` for `export *`.
    pub exported: Option<String>,
    /// The module.
    pub module: String,
    /// The imported name (`*` for a namespace).
    pub imported: String,
}

/// What a name refers to at a use site ([`JsFile::lookup`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    /// Index into `bindings[name]`.
    Binding(usize),
    /// A parameter of `funcs[func]`.
    Param {
        /// Function index.
        func: usize,
        /// Parameter index in `params`.
        param: usize,
    },
    /// `funcs[_]`, a function declaration or a `const`-bound function.
    Func(usize),
    /// Index into `imports`.
    Import(usize),
    /// Index into `classes`.
    Class(usize),
}

/// A condition a `return` is reached under ([`JsFile::guarded_returns`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    /// The condition `[a, b)` holds (`true`) or does not (`false`).
    Cond(usize, usize, bool),
    /// Inside a loop, `switch`, `try` or another block: unknown.
    Opaque,
}

/// A tokenized file with its declarations.
#[derive(Debug)]
pub struct JsFile {
    /// Absolute path.
    pub path: PathBuf,
    /// Source.
    pub src: Vec<u8>,
    /// Tokens.
    pub toks: Vec<Token>,
    /// Closer index for openers (`u32::MAX` otherwise).
    pub close: Vec<u32>,
    /// `const|let|var NAME = expr`: first binding per name.
    pub consts: HashMap<String, (usize, usize)>,
    /// Functions: name → return expression ranges.
    pub fns: HashMap<String, Vec<(usize, usize)>>,
    /// Exported names (`export const|function|class`, `export { a as b }`):
    /// exported → local.
    pub exports: HashMap<String, String>,
    /// Imports.
    pub imports: Vec<Import>,
    /// Classes.
    pub classes: Vec<ClassDef>,
    /// `const x = new Class(...)` / `x: Class` / `x = new Class()`: variable →
    /// class.
    pub var_class: HashMap<String, String>,
    /// `const x = f()` where `f` is imported: variable → (module, path) of the
    /// call result.
    pub call_bindings: HashMap<String, (String, String)>,
    /// Functions, in token order.
    pub funcs: Vec<FnDef>,
    /// The innermost opener enclosing each token (`u32::MAX` at the top level).
    pub open_of: Vec<u32>,
    /// Every `const|let|var` binding, by name, in token order.
    pub bindings: HashMap<String, Vec<Binding>>,
    /// Assignments to plain identifiers, by name.
    pub assigns: HashMap<String, Vec<Assign>>,
    /// Re-exports.
    pub reexports: Vec<ReExport>,
    /// The module directive: `use server` or `use client`.
    pub directive: Option<String>,
    /// The file's id in the cache that parsed it (0 outside a cache).
    pub id: u32,
}

fn match_brackets(toks: &[Token]) -> (Vec<u32>, Vec<u32>) {
    let mut close = vec![u32::MAX; toks.len()];
    let mut open_of = vec![u32::MAX; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                open_of[i] = stack.last().map_or(u32::MAX, |&o| o as u32);
                stack.push(i)
            }
            TokKind::RParen | TokKind::RBracket | TokKind::RBrace | TokKind::TemplateTail => {
                if let Some(o) = stack.pop() {
                    close[o] = i as u32;
                }
                open_of[i] = stack.last().map_or(u32::MAX, |&o| o as u32);
            }
            _ => open_of[i] = stack.last().map_or(u32::MAX, |&o| o as u32),
        }
    }
    (close, open_of)
}

impl JsFile {
    /// Tokenizes and indexes `src`.
    pub fn parse(path: &Path, src: Vec<u8>) -> JsFile {
        let mut lx = Lexer::new(&src, Dialect::from_path(path));
        let mut toks = Vec::new();
        loop {
            let t = lx.next_token();
            if t.kind == TokKind::Eof || toks.len() >= MAX_TOKENS {
                break;
            }
            if t.kind == TokKind::TripleSlash {
                continue;
            }
            toks.push(t);
        }
        let (close, open_of) = match_brackets(&toks);
        let mut f = JsFile {
            path: path.to_path_buf(),
            src,
            toks,
            close,
            open_of,
            bindings: HashMap::new(),
            assigns: HashMap::new(),
            reexports: Vec::new(),
            directive: None,
            id: 0,
            consts: HashMap::new(),
            fns: HashMap::new(),
            exports: HashMap::new(),
            imports: Vec::new(),
            classes: Vec::new(),
            var_class: HashMap::new(),
            call_bindings: HashMap::new(),
            funcs: Vec::new(),
        };
        f.index();
        f
    }

    /// Reads and parses a file.
    pub fn read(path: &Path) -> Option<JsFile> {
        let src = crate::fsx::read(path).ok()?;
        Some(JsFile::parse(path, src))
    }

    /// Token text.
    pub fn text(&self, i: usize) -> &str {
        self.toks
            .get(i)
            .and_then(|t| std::str::from_utf8(t.text(&self.src)).ok())
            .unwrap_or("")
    }

    /// Token kind (`Eof` past the end).
    pub fn kind(&self, i: usize) -> TokKind {
        self.toks.get(i).map_or(TokKind::Eof, |t| t.kind)
    }

    fn kw(&self, i: usize) -> Kw {
        self.toks.get(i).map_or(Kw::None, |t| t.kw)
    }

    /// The closer of the opener at `i`.
    pub fn closer(&self, i: usize) -> Option<usize> {
        self.close
            .get(i)
            .copied()
            .filter(|&c| c != u32::MAX)
            .map(|c| c as usize)
    }

    /// 1-based line of token `i`.
    pub fn line(&self, i: usize) -> u32 {
        let off = self.toks.get(i).map_or(0, |t| t.start as usize);
        1 + self.src[..off.min(self.src.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count() as u32
    }

    /// Source text of a token range (collapsed whitespace, capped).
    pub fn range_text(&self, a: usize, b: usize) -> String {
        if a >= b || b > self.toks.len() {
            return String::new();
        }
        let s = &self.src[self.toks[a].start as usize..self.toks[b - 1].end as usize];
        let s = String::from_utf8_lossy(s);
        let mut out = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if out.len() > 120 {
            let mut n = 120;
            while !out.is_char_boundary(n) {
                n -= 1;
            }
            out.truncate(n);
            out.push('…');
        }
        out
    }

    fn ends_value(&self, i: usize) -> bool {
        match self.kind(i) {
            TokKind::Ident => !matches!(
                self.kw(i),
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
            ),
            TokKind::Str
            | TokKind::Template
            | TokKind::TemplateTail
            | TokKind::Number
            | TokKind::Regex
            | TokKind::RParen
            | TokKind::RBracket
            | TokKind::RBrace
            | TokKind::Jsx
            | TokKind::IncDec => true,
            _ => false,
        }
    }

    /// If the `<` at `k` opens the type arguments of a call (`f<A, B<C>>(x)`):
    /// the token of the closing `>`.
    pub fn type_args_end(&self, k: usize) -> Option<usize> {
        if k == 0 || !matches!(self.kind(k - 1), TokKind::Ident) || self.kw(k - 1) != Kw::None {
            return None;
        }
        let n = self.toks.len();
        let mut depth = 0i32;
        let mut j = k;
        let mut steps = 0;
        while j < n {
            steps += 1;
            if steps > 400 {
                return None;
            }
            match self.kind(j) {
                TokKind::Lt => depth += 1,
                TokKind::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return (self.kind(j + 1) == TokKind::LParen).then_some(j);
                    }
                }
                TokKind::Op => {
                    let t = self.text(j);
                    if t.bytes().all(|c| c == b'>') {
                        depth -= t.len() as i32;
                        if depth <= 0 {
                            return (depth == 0 && self.kind(j + 1) == TokKind::LParen)
                                .then_some(j);
                        }
                    } else if !matches!(t, "|" | "&") {
                        return None;
                    }
                }
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace => j = self.closer(j)?,
                TokKind::Ident
                | TokKind::Comma
                | TokKind::Dot
                | TokKind::Str
                | TokKind::Number
                | TokKind::Colon
                | TokKind::Question
                | TokKind::Arrow
                | TokKind::Template => {}
                _ => return None,
            }
            j += 1;
        }
        None
    }

    /// The end (exclusive) of the expression starting at `i`, before `limit`.
    pub fn expr_end(&self, i: usize, limit: usize) -> usize {
        let mut k = i;
        while k < limit {
            if self.kind(k) == TokKind::Lt
                && let Some(g) = self.type_args_end(k).filter(|&g| g < limit)
            {
                k = g + 1;
                continue;
            }
            match self.kind(k) {
                TokKind::Semi
                | TokKind::Comma
                | TokKind::RParen
                | TokKind::RBrace
                | TokKind::RBracket
                | TokKind::TemplateTail
                | TokKind::TemplateMiddle => return k,
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    match self.closer(k) {
                        Some(c) if c < limit => k = c + 1,
                        _ => return limit,
                    }
                    continue;
                }
                _ => {}
            }
            if k > i
                && self.toks[k].flags & NEWLINE_BEFORE != 0
                && self.ends_value(k - 1)
                && matches!(
                    self.kind(k),
                    TokKind::Ident
                        | TokKind::Str
                        | TokKind::Template
                        | TokKind::TemplateHead
                        | TokKind::Number
                )
                && !matches!(self.kw(k), Kw::In | Kw::Instanceof)
                && !(self.kind(k) == TokKind::Ident && matches!(self.text(k), "as" | "satisfies"))
            {
                return k;
            }
            k += 1;
        }
        limit
    }

    /// Splits `[a, b)` at top-level commas.
    pub fn split_commas(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut s = a;
        let mut k = a;
        while k < b {
            match self.kind(k) {
                TokKind::Lt => {
                    if let Some(g) = self.type_args_end(k).filter(|&g| g < b) {
                        k = g;
                    }
                }
                TokKind::Comma => {
                    out.push((s, k));
                    s = k + 1;
                }
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    if let Some(c) = self.closer(k).filter(|&c| c < b) {
                        k = c;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        if s < b {
            out.push((s, b));
        }
        out
    }

    /// The `return` expressions of a function body `[a, b)`, skipping nested
    /// functions.
    fn returns_in(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut k = a;
        while k < b {
            if self.kw(k) == Kw::Function {
                let mut j = k + 1;
                while j < b && self.kind(j) != TokKind::LBrace {
                    if self.kind(j) == TokKind::LParen {
                        j = self.closer(j).unwrap_or(j);
                    }
                    j += 1;
                }
                k = self.closer(j).unwrap_or(j) + 1;
                continue;
            }
            if self.kind(k) == TokKind::Arrow {
                if self.kind(k + 1) == TokKind::LBrace
                    && let Some(c) = self.closer(k + 1)
                {
                    k = c + 1;
                } else {
                    k = self.expr_end(k + 1, b).max(k + 1);
                }
                continue;
            }
            if self.kw(k) == Kw::Return {
                let e = self.expr_end(k + 1, b);
                if e > k + 1 {
                    out.push((k + 1, e));
                }
                k = e.max(k + 1);
                continue;
            }
            k += 1;
        }
        out
    }

    /// For a function value starting at `i` (arrow or `function`), its result
    /// expressions.
    pub fn function_results(&self, i: usize, limit: usize) -> Option<Vec<(usize, usize)>> {
        let mut k = i;
        if self.text(k) == "async" {
            k += 1;
        }
        if self.kw(k) == Kw::Function {
            let mut j = k + 1;
            while j < limit && self.kind(j) != TokKind::LParen {
                if self.kind(j) == TokKind::Semi {
                    return None;
                }
                j += 1;
            }
            let pc = self.closer(j)?;
            // An overload signature or a `declare function` has no body.
            let b = self.body_brace(pc + 1, limit.max(pc + 1))?;
            let c = self.closer(b)?;
            return Some(self.returns_in(b + 1, c));
        }
        // Arrow: `(params) =>` or `x =>`, optional return type.
        let mut j = match self.kind(k) {
            TokKind::LParen => self.closer(k)? + 1,
            TokKind::Ident => k + 1,
            TokKind::Lt => {
                // Generic arrow `<T>(...) =>`.
                let mut j = k;
                while j < limit && self.kind(j) != TokKind::LParen {
                    j += 1;
                }
                self.closer(j)? + 1
            }
            _ => return None,
        };
        if self.kind(j) == TokKind::Colon {
            while j < limit && self.kind(j) != TokKind::Arrow {
                if matches!(
                    self.kind(j),
                    TokKind::LParen | TokKind::LBrace | TokKind::LBracket
                ) {
                    j = self.closer(j)?;
                }
                j += 1;
            }
        }
        if self.kind(j) != TokKind::Arrow {
            return None;
        }
        if self.kind(j + 1) == TokKind::LBrace {
            let c = self.closer(j + 1)?;
            return Some(self.returns_in(j + 2, c));
        }
        let e = self.expr_end(j + 1, limit);
        Some(vec![(j + 1, e)])
    }

    /// The first top-level `=` in `[a, b)`.
    fn top_assign(&self, a: usize, b: usize) -> Option<usize> {
        let mut k = a;
        while k < b {
            match self.kind(k) {
                TokKind::Assign => return Some(k),
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    k = self.closer(k).filter(|&c| c < b)?;
                }
                _ => {}
            }
            k += 1;
        }
        None
    }

    /// Splits a parameter list at its commas; commas inside type arguments
    /// (`f?: Fetcher<Data, Key>`) do not split.
    fn split_params(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let (mut s, mut k) = (a, a);
        let mut in_type = false;
        let mut angle = 0i32;
        while k < b {
            match self.kind(k) {
                TokKind::Comma if angle == 0 => {
                    out.push((s, k));
                    s = k + 1;
                    in_type = false;
                }
                TokKind::Colon if angle == 0 => in_type = true,
                TokKind::Assign if angle == 0 => in_type = false,
                TokKind::Lt if in_type => angle += 1,
                TokKind::Gt if in_type && angle > 0 => angle -= 1,
                TokKind::Op if in_type && angle > 0 => {
                    let t = self.text(k);
                    if t.bytes().all(|c| c == b'>') {
                        angle = (angle - t.len() as i32).max(0);
                    }
                }
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    if let Some(c) = self.closer(k).filter(|&c| c < b) {
                        k = c;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        if s < b {
            out.push((s, b));
        }
        out
    }

    fn params_of(&self, a: usize, b: usize) -> Vec<Param> {
        let mut out = Vec::new();
        for (idx, (x, y)) in self.split_params(a, b).into_iter().enumerate() {
            let idx = idx.min(255) as u8;
            let mut x = x;
            while x + 1 < y
                && matches!(
                    self.text(x),
                    "public" | "private" | "protected" | "readonly" | "override"
                )
                && self.kind(x + 1) == TokKind::Ident
            {
                x += 1;
            }
            let mut rest = false;
            if self.kind(x) == TokKind::Ellipsis {
                x += 1;
                rest = true;
            }
            let default = self.top_assign(x, y).map(|k| (k + 1, y));
            match self.kind(x) {
                TokKind::Ident => out.push(Param {
                    name: self.text(x).to_string(),
                    index: idx,
                    field: None,
                    default,
                    rest,
                }),
                TokKind::LBrace => {
                    let c = self.closer(x).unwrap_or(y).min(y);
                    for (p, q) in self.split_commas(x + 1, c) {
                        if p >= q || self.kind(p) != TokKind::Ident {
                            continue;
                        }
                        let field = self.text(p).to_string();
                        let local = if q >= p + 3
                            && self.kind(p + 1) == TokKind::Colon
                            && self.kind(p + 2) == TokKind::Ident
                        {
                            self.text(p + 2).to_string()
                        } else {
                            field.clone()
                        };
                        out.push(Param {
                            name: local,
                            index: idx,
                            field: Some(field),
                            default: self.top_assign(p, q).map(|k| (k + 1, q)),
                            rest: false,
                        });
                    }
                }
                TokKind::LBracket => {
                    let c = self.closer(x).unwrap_or(y).min(y);
                    for (n, (p, q)) in self.split_commas(x + 1, c).into_iter().enumerate() {
                        if p < q && self.kind(p) == TokKind::Ident {
                            out.push(Param {
                                name: self.text(p).to_string(),
                                index: idx,
                                field: Some(n.to_string()),
                                default: None,
                                rest: false,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn index_funcs(&mut self) {
        let n = self.toks.len();
        let mut opener = vec![usize::MAX; n];
        for (i, &c) in self.close.iter().enumerate() {
            if c != u32::MAX {
                opener[c as usize] = i;
            }
        }
        let name_before = |f: &JsFile, k: usize| -> Option<(String, FnKind)> {
            // `name = <fn>`, `name: <fn>` (property), `const name = async <fn>`.
            let mut j = k;
            if j > 0 && f.text(j - 1) == "async" {
                j -= 1;
            }
            if j >= 2
                && matches!(f.kind(j - 1), TokKind::Assign | TokKind::Colon)
                && matches!(f.kind(j - 2), TokKind::Ident | TokKind::Str)
            {
                let kind = if f.kind(j - 1) == TokKind::Assign {
                    FnKind::Bound
                } else {
                    FnKind::Prop
                };
                return Some((f.text(j - 2).trim_matches(['"', '\'']).to_string(), kind));
            }
            None
        };
        for k in 0..n {
            if self.kw(k) == Kw::Function {
                let mut j = k + 1;
                if self.kind(j) == TokKind::Star {
                    j += 1;
                }
                let mut name = None;
                if self.kind(j) == TokKind::Ident {
                    name = Some((self.text(j).to_string(), FnKind::Decl));
                    j += 1;
                }
                if self.kind(j) == TokKind::Lt {
                    // Type parameters.
                    let mut d = 0;
                    while j < n {
                        match self.kind(j) {
                            TokKind::Lt => d += 1,
                            TokKind::Gt => {
                                d -= 1;
                                if d == 0 {
                                    j += 1;
                                    break;
                                }
                            }
                            TokKind::LParen | TokKind::Semi => break,
                            _ => {}
                        }
                        j += 1;
                    }
                }
                if self.kind(j) != TokKind::LParen {
                    continue;
                }
                let Some(pc) = self.closer(j) else { continue };
                let params = self.params_of(j + 1, pc);
                let Some(b) = self.body_brace(pc + 1, n) else {
                    continue;
                };
                let Some(bc) = self.closer(b) else { continue };
                let name = name.or_else(|| name_before(self, k));
                let (name, kind) = match name {
                    Some((n, k)) => (Some(n), k),
                    None => (None, FnKind::Anon),
                };
                self.funcs.push(FnDef {
                    name,
                    kind,
                    params,
                    start: k,
                    body: (b + 1, bc),
                    block: true,
                });
            } else if self.kind(k) == TokKind::Arrow && k > 0 {
                // `(params): ReturnType =>`: find the `)` before the return type.
                let mut pe = k - 1;
                // A plain `(...) =>` or `x =>` (also a property `key: x =>`); else a return
                // type sits between `)` and `=>`.
                let return_type_ident = self.kind(pe) == TokKind::Ident
                    && k >= 3
                    && self.kind(k - 2) == TokKind::Colon
                    && self.kind(k - 3) == TokKind::RParen;
                let plain = self.kind(pe) == TokKind::RParen
                    || (self.kind(pe) == TokKind::Ident && !return_type_ident);
                if !plain {
                    let mut j = k - 1;
                    let mut found = None;
                    let mut steps = 0;
                    while j > 0 && steps < 60 {
                        steps += 1;
                        if self.kind(j) == TokKind::Colon && self.kind(j - 1) == TokKind::RParen {
                            found = Some(j - 1);
                            break;
                        }
                        match self.kind(j) {
                            TokKind::RParen | TokKind::RBracket | TokKind::RBrace
                                if opener[j] != usize::MAX =>
                            {
                                j = opener[j];
                            }
                            TokKind::Semi | TokKind::LBrace | TokKind::Assign | TokKind::Arrow => {
                                break;
                            }
                            _ => {}
                        }
                        if j == 0 {
                            break;
                        }
                        j -= 1;
                    }
                    match found {
                        Some(r) => pe = r,
                        None => continue,
                    }
                }
                let (pstart, params) = match self.kind(pe) {
                    TokKind::Ident if pe == k - 1 => (
                        k - 1,
                        vec![Param {
                            name: self.text(k - 1).to_string(),
                            index: 0,
                            field: None,
                            default: None,
                            rest: false,
                        }],
                    ),
                    TokKind::RParen => {
                        let o = opener[pe];
                        if o == usize::MAX {
                            continue;
                        }
                        (o, self.params_of(o + 1, pe))
                    }
                    _ => continue,
                };
                let block = self.kind(k + 1) == TokKind::LBrace;
                let body = if block {
                    match self.closer(k + 1) {
                        Some(c) => (k + 2, c),
                        None => continue,
                    }
                } else {
                    (k + 1, self.expr_end(k + 1, n))
                };
                // Type parameters before the parameter list: `<T,>(x: T) =>`.
                let mut nstart = pstart;
                if nstart > 0 && self.kind(nstart - 1) == TokKind::Gt {
                    let mut j = nstart - 1;
                    let mut d = 0;
                    let mut steps = 0;
                    loop {
                        steps += 1;
                        match self.kind(j) {
                            TokKind::Gt => d += 1,
                            TokKind::Lt => {
                                d -= 1;
                                if d == 0 {
                                    nstart = j;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        if j == 0 || steps > 40 {
                            break;
                        }
                        j -= 1;
                    }
                }
                let (name, kind) = match name_before(self, nstart) {
                    Some((n, k)) => (Some(n), k),
                    None => (None, FnKind::Anon),
                };
                self.funcs.push(FnDef {
                    name,
                    kind,
                    params,
                    start: pstart,
                    body,
                    block,
                });
            } else if self.kind(k) == TokKind::LParen
                && k >= 1
                && matches!(self.kind(k - 1), TokKind::Ident | TokKind::Str)
                && self.open_of[k - 1] != u32::MAX
                && self.kind(self.open_of[k - 1] as usize) == TokKind::LBrace
            {
                // A method: `name(params) {` in a class body or an object literal.
                let nm = k - 1;
                let name = self.text(nm).trim_matches(['"', '\'']).to_string();
                if matches!(
                    name.as_str(),
                    "if" | "for"
                        | "while"
                        | "switch"
                        | "catch"
                        | "with"
                        | "function"
                        | "return"
                        | "super"
                        | "import"
                        | "await"
                        | "typeof"
                        | "new"
                ) || (self.kw(nm) != Kw::None
                    && !matches!(
                        self.kw(nm),
                        Kw::Type | Kw::From | Kw::As | Kw::Of | Kw::Module | Kw::Default
                    ))
                {
                    continue;
                }
                let before_ok = nm == 0
                    || matches!(
                        self.kind(nm - 1),
                        TokKind::LBrace
                            | TokKind::Comma
                            | TokKind::Semi
                            | TokKind::RBrace
                            | TokKind::Star
                    )
                    || (self.kind(nm - 1) == TokKind::Ident
                        && matches!(
                            self.text(nm - 1),
                            "async"
                                | "static"
                                | "get"
                                | "set"
                                | "public"
                                | "private"
                                | "protected"
                                | "override"
                                | "readonly"
                                | "abstract"
                        ));
                if !before_ok {
                    continue;
                }
                let Some(pc) = self.closer(k) else { continue };
                if !matches!(self.kind(pc + 1), TokKind::LBrace | TokKind::Colon) {
                    continue;
                }
                let Some(b) = self.body_brace(pc + 1, n) else {
                    continue;
                };
                let Some(bc) = self.closer(b) else { continue };
                let params = self.params_of(k + 1, pc);
                self.funcs.push(FnDef {
                    name: Some(name),
                    kind: FnKind::Method,
                    params,
                    start: nm,
                    body: (b + 1, bc),
                    block: true,
                });
            }
        }
        self.funcs.sort_by_key(|f| f.start);
    }

    /// The body `{` of a function whose parameter list ended before `from`:
    /// skips a return type annotation, whose object type literals
    /// (`Promise<{ a: string }>`) are braces too (a type literal's `}` is
    /// followed by `>`, `|`, `&`, `[` or `,`).
    fn body_brace(&self, from: usize, limit: usize) -> Option<usize> {
        let mut b = from;
        while b < limit {
            match self.kind(b) {
                TokKind::LBrace => {
                    let c = self.closer(b)?;
                    let after = self.kind(c + 1);
                    let in_type = self.kind(from) == TokKind::Colon
                        && (matches!(after, TokKind::Gt | TokKind::LBracket | TokKind::Comma)
                            || matches!(self.text(c + 1), "|" | "&"));
                    if !in_type {
                        return Some(b);
                    }
                    b = c;
                }
                TokKind::LParen | TokKind::LBracket => b = self.closer(b)?,
                TokKind::Semi => return None,
                _ => {}
            }
            b += 1;
        }
        None
    }

    /// The innermost function whose body contains token `i`.
    pub fn enclosing_fn(&self, i: usize) -> Option<&FnDef> {
        self.funcs
            .iter()
            .filter(|f| f.body.0 <= i && i < f.body.1)
            .min_by_key(|f| f.body.1 - f.body.0)
    }

    /// The call whose argument list contains token `i` directly: `(callee
    /// start, args)`.
    pub fn enclosing_call(&self, i: usize) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for (o, &c) in self.close.iter().enumerate() {
            if c == u32::MAX || self.kind(o) != TokKind::LParen {
                continue;
            }
            let c = c as usize;
            if o < i && i < c && best.is_none_or(|(bo, _)| o > bo) {
                best = Some((o, c));
            }
        }
        let (o, _) = best?;
        (o > 0 && self.kind(o - 1) == TokKind::Ident).then_some(o - 1)
    }

    fn index(&mut self) {
        self.index_funcs();
        let n = self.toks.len();
        if self.kind(0) == TokKind::Str
            && let Some(d) = self.str_value(0)
            && matches!(d.as_str(), "use server" | "use client")
        {
            self.directive = Some(d);
        }
        let mut i = 0;
        while i < n {
            let kw = self.kw(i);
            // `x = e` / `x += e` in statement position.
            if self.kind(i) == TokKind::Ident
                && kw == Kw::None
                && (self.kind(i + 1) == TokKind::Assign
                    || (self.kind(i + 1) == TokKind::Op && self.text(i + 1) == "+="))
                && (i == 0
                    || self.toks[i].flags & NEWLINE_BEFORE != 0
                    || matches!(
                        self.kind(i - 1),
                        TokKind::Semi | TokKind::LBrace | TokKind::RBrace | TokKind::RParen
                    )
                    || matches!(self.kw(i - 1), Kw::Else | Kw::Do))
                && !(i > 0 && matches!(self.kw(i - 1), Kw::Const | Kw::Let | Kw::Var))
            {
                let e = self.expr_end(i + 2, n);
                if e > i + 2 {
                    let name = self.text(i).to_string();
                    let append = self.kind(i + 1) == TokKind::Op;
                    self.assigns.entry(name).or_default().push(Assign {
                        at: i,
                        value: (i + 2, e),
                        append,
                    });
                }
            }
            // Imports.
            if kw == Kw::Import && self.toks[i].depth == 0 && self.kind(i + 1) != TokKind::LParen {
                i = self.index_import(i);
                continue;
            }
            if kw == Kw::Export {
                self.index_export(i);
            }
            if matches!(kw, Kw::Const | Kw::Let | Kw::Var) {
                self.index_decl(i);
            }
            if kw == Kw::Function && self.kind(i + 1) == TokKind::Ident {
                let name = self.text(i + 1).to_string();
                if let Some(r) = self.function_results(i, n) {
                    self.fns.entry(name).or_insert(r);
                }
            }
            if kw == Kw::Class && self.kind(i + 1) == TokKind::Ident {
                self.index_class(i);
            }
            // `x = new C(...)`
            if self.kind(i) == TokKind::Ident
                && self.kind(i + 1) == TokKind::Assign
                && self.kw(i + 2) == Kw::New
                && self.kind(i + 3) == TokKind::Ident
            {
                let v = self.text(i).to_string();
                let c = self.text(i + 3).to_string();
                self.var_class.entry(v).or_insert(c);
            }
            // `x: C` (parameters, fields): a type reference to a class name.
            if self.kind(i) == TokKind::Ident
                && self.kind(i + 1) == TokKind::Colon
                && self.kind(i + 2) == TokKind::Ident
                && self
                    .text(i + 2)
                    .chars()
                    .next()
                    .is_some_and(char::is_uppercase)
                && matches!(
                    self.kind(i + 3),
                    TokKind::Comma | TokKind::RParen | TokKind::Semi | TokKind::Assign
                )
            {
                let v = self.text(i).to_string();
                let c = self.text(i + 2).to_string();
                self.var_class.entry(v).or_insert(c);
            }
            i += 1;
        }
    }

    fn index_import(&mut self, i: usize) -> usize {
        let n = self.toks.len();
        // Find `from 'm'` or a bare `import 'm'`.
        let mut k = i + 1;
        let mut end = n;
        let mut module = None;
        while k < n {
            if self.kind(k) == TokKind::Str && (self.kw(k - 1) == Kw::From || k == i + 1) {
                module = Some(self.str_value(k));
                end = k;
                break;
            }
            if self.kind(k) == TokKind::Semi || (k > i + 1 && self.kw(k) == Kw::Import) {
                break;
            }
            k += 1;
        }
        let Some(module) = module.flatten() else {
            return k.max(i + 1);
        };
        let mut j = i + 1;
        if self.kw(j) == Kw::Type {
            return end + 1;
        }
        while j < end {
            match self.kind(j) {
                TokKind::Ident if self.kw(j) == Kw::From => break,
                TokKind::Ident if self.text(j) != "type" => {
                    self.imports.push(Import {
                        local: self.text(j).to_string(),
                        module: module.clone(),
                        imported: Imported::Default,
                    });
                }
                TokKind::Star => {
                    if self.kw(j + 1) == Kw::As {
                        self.imports.push(Import {
                            local: self.text(j + 2).to_string(),
                            module: module.clone(),
                            imported: Imported::Namespace,
                        });
                        j += 2;
                    }
                }
                TokKind::LBrace => {
                    let c = self.closer(j).unwrap_or(end);
                    for (a, b) in self.split_commas(j + 1, c) {
                        let mut a = a;
                        if self.text(a) == "type" && b > a + 1 {
                            continue;
                        }
                        if a >= b {
                            continue;
                        }
                        let imported = self.text(a).to_string();
                        let local = if b >= a + 3 && self.kw(a + 1) == Kw::As {
                            self.text(a + 2).to_string()
                        } else {
                            imported.clone()
                        };
                        a += 1;
                        let _ = a;
                        self.imports.push(Import {
                            local,
                            module: module.clone(),
                            imported: if imported == "default" {
                                Imported::Default
                            } else {
                                Imported::Named(imported)
                            },
                        });
                    }
                    j = c;
                }
                _ => {}
            }
            j += 1;
        }
        end + 1
    }

    fn index_export(&mut self, i: usize) {
        let mut k = i + 1;
        // `export * from 'm'`, `export * as ns from 'm'`.
        if self.kind(k) == TokKind::Star {
            let (exported, from) = if self.kw(k + 1) == Kw::As {
                (Some(self.text(k + 2).to_string()), k + 3)
            } else {
                (None, k + 1)
            };
            if self.kw(from) == Kw::From
                && let Some(module) = self.str_value(from + 1)
            {
                self.reexports.push(ReExport {
                    exported,
                    module,
                    imported: "*".into(),
                });
            }
            return;
        }
        if self.kw(k) == Kw::Type && self.kind(k + 1) == TokKind::LBrace {
            return;
        }
        if matches!(self.text(k), "declare" | "abstract") {
            k += 1;
        }
        if matches!(
            self.kw(k),
            Kw::Const | Kw::Let | Kw::Var | Kw::Function | Kw::Class
        ) || self.text(k) == "enum"
        {
            let mut j = k + 1;
            if self.kind(j) == TokKind::Star {
                j += 1;
            }
            if self.kind(j) == TokKind::Ident {
                let n = self.text(j).to_string();
                self.exports.insert(n.clone(), n);
            }
        } else if self.text(k) == "async" && self.kw(k + 1) == Kw::Function {
            let mut j = k + 2;
            if self.kind(j) == TokKind::Star {
                j += 1;
            }
            let n = self.text(j).to_string();
            self.exports.insert(n.clone(), n);
        } else if self.kw(k) == Kw::Default {
            let mut j = k + 1;
            if self.text(j) == "async" && self.kw(j + 1) == Kw::Function {
                j += 1;
            }
            if matches!(self.kw(j), Kw::Function | Kw::Class) {
                let mut q = j + 1;
                if self.kind(q) == TokKind::Star {
                    q += 1;
                }
                if self.kind(q) == TokKind::Ident && self.kw(q) != Kw::Extends {
                    self.exports
                        .insert("default".into(), self.text(q).to_string());
                    return;
                }
            } else if self.kind(j) == TokKind::Ident
                && matches!(
                    self.kind(j + 1),
                    TokKind::Semi | TokKind::Eof | TokKind::Ident
                )
                && self.text(j) != "async"
            {
                // `export default name`.
                self.exports
                    .insert("default".into(), self.text(j).to_string());
                return;
            }
            // An anonymous default function: name it `default`.
            let start = k + 1;
            if let Some(fd) = self.funcs.iter_mut().find(|fd| {
                fd.name.is_none() && (fd.start == start || fd.start == start + 1 || fd.start == j)
            }) {
                fd.name = Some("default".into());
                fd.kind = FnKind::Decl;
            }
            self.exports.insert("default".into(), "default".into());
        } else if self.kind(k) == TokKind::LBrace
            && let Some(c) = self.closer(k)
        {
            let module = if self.kw(c + 1) == Kw::From {
                self.str_value(c + 2)
            } else {
                None
            };
            for (a, b) in self.split_commas(k + 1, c) {
                let mut a = a;
                if a >= b {
                    continue;
                }
                if self.text(a) == "type" && b > a + 1 && self.kind(a + 1) == TokKind::Ident {
                    if self.kw(a + 1) != Kw::As {
                        continue;
                    }
                    a += 0;
                }
                let local = self.text(a).to_string();
                let exported = if b >= a + 3 && self.kw(a + 1) == Kw::As {
                    self.text(a + 2).to_string()
                } else {
                    local.clone()
                };
                match &module {
                    Some(m) => self.reexports.push(ReExport {
                        exported: Some(exported),
                        module: m.clone(),
                        imported: local,
                    }),
                    None => {
                        self.exports.insert(exported, local);
                    }
                }
            }
        }
    }

    /// The `{` of the innermost block containing token `i` (`usize::MAX`:
    /// module scope).
    pub fn block_of(&self, i: usize) -> usize {
        let mut o = self.open_of.get(i).copied().unwrap_or(u32::MAX);
        while o != u32::MAX {
            if self.kind(o as usize) == TokKind::LBrace {
                return o as usize;
            }
            o = self.open_of[o as usize];
        }
        usize::MAX
    }

    fn bind(
        &mut self,
        at: usize,
        scope: usize,
        init: Option<(usize, usize)>,
        path: Vec<String>,
        mutable: bool,
    ) {
        let name = self.text(at).to_string();
        self.bindings.entry(name).or_default().push(Binding {
            at,
            scope,
            init,
            path,
            mutable,
        });
    }

    /// Bindings of a destructuring pattern `[open, close]`.
    fn destructure(
        &mut self,
        open: usize,
        close: usize,
        init: (usize, usize),
        scope: usize,
        mutable: bool,
        path: &mut Vec<String>,
    ) {
        if path.len() > 4 {
            return;
        }
        let object = self.kind(open) == TokKind::LBrace;
        for (n, (p, q)) in self.split_commas(open + 1, close).into_iter().enumerate() {
            if p >= q || self.kind(p) == TokKind::Ellipsis {
                continue;
            }
            if object {
                if !matches!(self.kind(p), TokKind::Ident | TokKind::Str) {
                    continue;
                }
                let key = self.text(p).trim_matches(['"', '\'']).to_string();
                path.push(key);
                if self.kind(p + 1) == TokKind::Colon {
                    match self.kind(p + 2) {
                        TokKind::Ident => {
                            self.bind(p + 2, scope, Some(init), path.clone(), mutable)
                        }
                        TokKind::LBrace | TokKind::LBracket => {
                            if let Some(c) = self.closer(p + 2) {
                                self.destructure(p + 2, c, init, scope, mutable, path);
                            }
                        }
                        _ => {}
                    }
                } else if self.kind(p) == TokKind::Ident {
                    self.bind(p, scope, Some(init), path.clone(), mutable);
                }
                path.pop();
            } else {
                path.push(n.to_string());
                match self.kind(p) {
                    TokKind::Ident => self.bind(p, scope, Some(init), path.clone(), mutable),
                    TokKind::LBrace | TokKind::LBracket => {
                        if let Some(c) = self.closer(p) {
                            self.destructure(p, c, init, scope, mutable, path);
                        }
                    }
                    _ => {}
                }
                path.pop();
            }
        }
    }

    fn index_decl(&mut self, i: usize) {
        let n = self.toks.len();
        let mutable = self.kw(i) != Kw::Const;
        let scope = self.block_of(i);
        let mut k = i + 1;
        let mut first = true;
        loop {
            match self.kind(k) {
                TokKind::LBrace | TokKind::LBracket => {
                    let Some(c) = self.closer(k) else { return };
                    let mut j = c + 1;
                    if self.kind(j) == TokKind::Colon {
                        while j < n && !matches!(self.kind(j), TokKind::Assign | TokKind::Semi) {
                            if matches!(
                                self.kind(j),
                                TokKind::LParen | TokKind::LBrace | TokKind::LBracket
                            ) {
                                match self.closer(j) {
                                    Some(x) => j = x,
                                    None => return,
                                }
                            }
                            j += 1;
                        }
                    }
                    if self.kind(j) != TokKind::Assign {
                        return;
                    }
                    let a = j + 1;
                    let e = self.expr_end(a, n);
                    if a >= e {
                        return;
                    }
                    // Destructured require: `const { a, b: c } = require('m')`.
                    if self.kind(k) == TokKind::LBrace
                        && self.kw(a) == Kw::Require
                        && self.kind(a + 2) == TokKind::Str
                    {
                        if let Some(module) = self.str_value(a + 2) {
                            for (x, y) in self.split_commas(k + 1, c) {
                                if x >= y {
                                    continue;
                                }
                                let imported = self.text(x).to_string();
                                let local = if y >= x + 3 && self.kind(x + 1) == TokKind::Colon {
                                    self.text(x + 2).to_string()
                                } else {
                                    imported.clone()
                                };
                                self.imports.push(Import {
                                    local,
                                    module: module.clone(),
                                    imported: Imported::Named(imported),
                                });
                            }
                        }
                    } else {
                        self.destructure(k, c, (a, e), scope, mutable, &mut Vec::new());
                    }
                    k = e;
                }
                TokKind::Ident => {
                    let name = self.text(k).to_string();
                    let mut j = k + 1;
                    if self.kind(j) == TokKind::Bang {
                        j += 1;
                    }
                    if self.kind(j) == TokKind::Colon {
                        // Type annotation.
                        if self.kind(j + 1) == TokKind::Ident
                            && self
                                .text(j + 1)
                                .chars()
                                .next()
                                .is_some_and(char::is_uppercase)
                        {
                            let c = self.text(j + 1).to_string();
                            self.var_class.entry(name.clone()).or_insert(c);
                        }
                        while j < n
                            && self.kind(j) != TokKind::Assign
                            && self.kind(j) != TokKind::Semi
                        {
                            if matches!(
                                self.kind(j),
                                TokKind::LParen | TokKind::LBrace | TokKind::LBracket
                            ) {
                                match self.closer(j) {
                                    Some(c) => j = c,
                                    None => return,
                                }
                            }
                            if self.toks[j].flags & NEWLINE_BEFORE != 0 && j > k + 2 {
                                break;
                            }
                            j += 1;
                        }
                    }
                    if self.kind(j) != TokKind::Assign {
                        self.bind(k, scope, None, Vec::new(), mutable);
                        if self.kind(j) == TokKind::Comma {
                            k = j + 1;
                            first = false;
                            continue;
                        }
                        return;
                    }
                    let a = j + 1;
                    let e = self.expr_end(a, n);
                    if a >= e {
                        return;
                    }
                    self.bind(k, scope, Some((a, e)), Vec::new(), mutable);
                    if first {
                        self.index_init(name, a, e);
                    }
                    k = e;
                }
                _ => return,
            }
            first = false;
            if self.kind(k) == TokKind::Comma {
                k += 1;
                continue;
            }
            return;
        }
    }

    /// The legacy per-name indexes of `const NAME = <a..e>`.
    fn index_init(&mut self, name: String, a: usize, e: usize) {
        let n = self.toks.len();
        // `require('m')`.
        if self.kw(a) == Kw::Require && self.kind(a + 2) == TokKind::Str {
            if let Some(module) = self.str_value(a + 2) {
                self.imports.push(Import {
                    local: name,
                    module,
                    imported: Imported::Default,
                });
            }
            return;
        }
        if let Some(r) = self.function_results(a, e.max(a + 1).min(n)) {
            self.fns.entry(name).or_insert(r);
            return;
        }
        if self.kw(a) == Kw::New && self.kind(a + 1) == TokKind::Ident {
            let c = self.text(a + 1).to_string();
            if c != "URL" && c != "Request" {
                self.var_class.entry(name.clone()).or_insert(c);
            }
        }
        // `const r = useRouter()` / `const api = axios.create()`.
        let mut c = a;
        if self.kw(c) == Kw::Await {
            c += 1;
        }
        if self.kind(c) == TokKind::Ident {
            let mut chain = vec![self.text(c).to_string()];
            let mut q = c + 1;
            while self.kind(q) == TokKind::Dot && self.kind(q + 1) == TokKind::Ident {
                chain.push(self.text(q + 1).to_string());
                q += 2;
            }
            if self.kind(q) == TokKind::LParen
                && self.closer(q).map(|x| x + 1) == Some(e)
                && let Some(imp) = self.imports.iter().find(|im| im.local == chain[0])
            {
                let base = match &imp.imported {
                    Imported::Default => "default".to_string(),
                    Imported::Named(n) => n.clone(),
                    Imported::Namespace => String::new(),
                };
                let mut path = base;
                for x in &chain[1..] {
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(x);
                }
                path.push_str("()");
                self.call_bindings
                    .insert(name.clone(), (imp.module.clone(), path));
            }
        }
        self.consts.entry(name).or_insert((a, e));
    }

    /// The function whose definition is the expression `[a, e)` (`async`
    /// allowed).
    pub fn fn_at(&self, a: usize, e: usize) -> Option<usize> {
        self.funcs.iter().position(|fd| {
            fd.body.1 <= e
                && (fd.start == a
                    || (fd.start == a + 1 && self.text(a) == "async")
                    || (self.kind(a) == TokKind::Lt && fd.start > a && fd.start < e && {
                        // `<T,>(x) => ...`
                        (a..fd.start).all(|k| self.kind(k) != TokKind::LParen)
                    }))
        })
    }

    /// What `name` refers to at token `at`: the declaration in the innermost
    /// enclosing scope (block scopes for bindings, function scopes for
    /// parameters), then imports.
    pub fn lookup(&self, name: &str, at: usize) -> Option<Decl> {
        let n = self.toks.len();
        let mut best: Option<(usize, Decl)> = None;
        let mut consider = |len: usize, d: Decl| {
            if best.is_none_or(|(l, _)| len < l) {
                best = Some((len, d));
            }
        };
        if let Some(bs) = self.bindings.get(name) {
            for (i, b) in bs.iter().enumerate() {
                let (s, e) = if b.scope == usize::MAX {
                    (0, n)
                } else {
                    (b.scope, self.closer(b.scope).unwrap_or(n))
                };
                if s <= at && at <= e {
                    let d = match b.init {
                        Some((x, y)) if b.path.is_empty() => match self.fn_at(x, y) {
                            Some(fi) => Decl::Func(fi),
                            None => Decl::Binding(i),
                        },
                        _ => Decl::Binding(i),
                    };
                    consider(2 * (e - s), d);
                }
            }
        }
        for (fi, fd) in self.funcs.iter().enumerate() {
            if fd.start > at {
                break;
            }
            if at < fd.body.1
                && let Some(pi) = fd.params.iter().position(|p| p.name == name)
            {
                // A parameter shadows a binding of the enclosing block, and is shadowed by
                // one in its own body.
                consider(
                    2 * (fd.body.1 - fd.start) + 1,
                    Decl::Param {
                        func: fi,
                        param: pi,
                    },
                );
            }
        }
        for (fi, fd) in self.funcs.iter().enumerate() {
            if fd.kind == FnKind::Decl && fd.name.as_deref() == Some(name) {
                let o = self.block_of(fd.start);
                let (s, e) = if o == usize::MAX {
                    (0, n)
                } else {
                    (o, self.closer(o).unwrap_or(n))
                };
                if s <= at && at <= e {
                    consider(2 * (e - s), Decl::Func(fi));
                }
            }
        }
        if best.is_none() {
            if let Some(i) = self.imports.iter().position(|im| im.local == name) {
                return Some(Decl::Import(i));
            }
            if let Some(i) = self.classes.iter().position(|c| c.name == name) {
                return Some(Decl::Class(i));
            }
        }
        best.map(|x| x.1)
    }

    /// The module-scope binding of `name`.
    pub fn module_binding(&self, name: &str) -> Option<&Binding> {
        self.bindings
            .get(name)?
            .iter()
            .find(|b| b.scope == usize::MAX)
    }

    /// The module-scope function named `name` (a declaration or a `const`-bound
    /// function).
    pub fn module_fn(&self, name: &str) -> Option<usize> {
        if let Some(b) = self.module_binding(name)
            && let Some((x, y)) = b.init
            && b.path.is_empty()
            && let Some(fi) = self.fn_at(x, y)
        {
            return Some(fi);
        }
        self.funcs.iter().position(|fd| {
            fd.kind == FnKind::Decl
                && fd.name.as_deref() == Some(name)
                && self.block_of(fd.start) == usize::MAX
        })
    }

    /// The statement starting at `i`: `(content start, content end, index
    /// after)`. A block's content is its inside.
    fn stmt_range(&self, i: usize, limit: usize) -> (usize, usize, usize) {
        if self.kind(i) == TokKind::LBrace
            && let Some(c) = self.closer(i).filter(|&c| c < limit)
        {
            return (i + 1, c, c + 1);
        }
        if self.kw(i) == Kw::If
            && self.kind(i + 1) == TokKind::LParen
            && let Some(c) = self.closer(i + 1).filter(|&c| c < limit)
        {
            let (_, _, mut after) = self.stmt_range(c + 1, limit);
            if self.kw(after) == Kw::Else {
                after = self.stmt_range(after + 1, limit).2;
            }
            return (i, after.min(limit), after.min(limit));
        }
        let mut k = i;
        while k < limit {
            match self.kind(k) {
                TokKind::Semi => return (i, k, k + 1),
                TokKind::RBrace => return (i, k, k),
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    match self.closer(k).filter(|&c| c < limit) {
                        Some(c) => k = c,
                        None => return (i, limit, limit),
                    }
                }
                _ => {}
            }
            if k > i
                && self
                    .toks
                    .get(k + 1)
                    .is_some_and(|t| t.flags & NEWLINE_BEFORE != 0)
                && self.ends_value(k)
                && !matches!(
                    self.kind(k + 1),
                    TokKind::Dot | TokKind::OptChain | TokKind::Op
                )
            {
                return (i, k + 1, k + 1);
            }
            k += 1;
        }
        (i, limit, limit)
    }

    /// Whether the statements `[a, b)` always leave the function: the last one
    /// is a `return` or a `throw`.
    fn exits(&self, a: usize, b: usize) -> bool {
        let mut last = None;
        let mut k = a;
        while k < b {
            match self.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    match self.closer(k).filter(|&c| c < b) {
                        Some(c) => k = c,
                        None => return false,
                    }
                }
                _ if matches!(self.kw(k), Kw::Return | Kw::Throw) => last = Some(k),
                _ => {}
            }
            k += 1;
        }
        let Some(r) = last else { return false };
        let e = self.stmt_range(r, b);
        e.2 >= b
    }

    /// The `return` expressions of the function body `[a, b)` with the
    /// conditions each is reached under (`if` conditions around it, and the
    /// negated conditions of earlier `if`s that always return). Nested
    /// functions are skipped.
    pub fn guarded_returns(&self, a: usize, b: usize) -> Vec<(Vec<Guard>, (usize, usize))> {
        let mut out = Vec::new();
        let mut guards = Vec::new();
        self.walk_returns(a, b, &mut guards, &mut out, 0);
        out
    }

    fn walk_returns(
        &self,
        a: usize,
        b: usize,
        guards: &mut Vec<Guard>,
        out: &mut Vec<(Vec<Guard>, (usize, usize))>,
        depth: u32,
    ) {
        if depth > 40 {
            return;
        }
        let base = guards.len();
        let mut k = a;
        while k < b {
            let kw = self.kw(k);
            if kw == Kw::If
                && self.kind(k + 1) == TokKind::LParen
                && let Some(c) = self.closer(k + 1).filter(|&c| c < b)
            {
                let (ts, te, after) = self.stmt_range(c + 1, b);
                guards.push(Guard::Cond(k + 2, c, true));
                self.walk_returns(ts, te, guards, out, depth + 1);
                guards.pop();
                let mut next = after;
                if self.kw(next) == Kw::Else {
                    let (es, ee, eafter) = self.stmt_range(next + 1, b);
                    guards.push(Guard::Cond(k + 2, c, false));
                    self.walk_returns(es, ee, guards, out, depth + 1);
                    guards.pop();
                    next = eafter;
                } else if self.exits(ts, te) {
                    guards.push(Guard::Cond(k + 2, c, false));
                }
                k = next.max(k + 1);
                continue;
            }
            if kw == Kw::Return {
                let e = self.expr_end(k + 1, b);
                if e > k + 1 {
                    out.push((guards.clone(), (k + 1, e)));
                }
                k = e.max(k + 1);
                continue;
            }
            if kw == Kw::Function {
                let mut j = k + 1;
                while j < b && self.kind(j) != TokKind::LBrace {
                    if self.kind(j) == TokKind::LParen {
                        j = self.closer(j).unwrap_or(j);
                    }
                    j += 1;
                }
                k = self.closer(j).unwrap_or(j) + 1;
                continue;
            }
            match self.kind(k) {
                TokKind::Arrow => {
                    if self.kind(k + 1) == TokKind::LBrace
                        && let Some(c) = self.closer(k + 1)
                    {
                        k = c + 1;
                    } else {
                        k = self.expr_end(k + 1, b).max(k + 1);
                    }
                    continue;
                }
                TokKind::LBrace => {
                    if let Some(c) = self.closer(k).filter(|&c| c <= b) {
                        // A method body inside an object literal or class is another
                        // function.
                        let method = self.funcs.iter().any(|fd| fd.body.0 == k + 1);
                        if !method {
                            guards.push(Guard::Opaque);
                            self.walk_returns(k + 1, c, guards, out, depth + 1);
                            guards.pop();
                        }
                        k = c + 1;
                        continue;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        guards.truncate(base);
    }

    fn index_class(&mut self, i: usize) {
        let name = self.text(i + 1).to_string();
        let mut k = i + 2;
        let mut parent = None;
        while k < self.toks.len() && self.kind(k) != TokKind::LBrace {
            if self.kw(k) == Kw::Extends && self.kind(k + 1) == TokKind::Ident {
                parent = Some(self.text(k + 1).to_string());
            }
            k += 1;
        }
        let Some(c) = self.closer(k) else { return };
        let mut def = ClassDef {
            name,
            parent,
            body: (k, c),
            ..ClassDef::default()
        };
        let mut j = k + 1;
        while j < c {
            // Skip modifiers and decorators.
            while j < c
                && matches!(
                    self.text(j),
                    "public"
                        | "private"
                        | "protected"
                        | "readonly"
                        | "static"
                        | "async"
                        | "override"
                        | "get"
                        | "set"
                        | "declare"
                        | "abstract"
                )
                && matches!(
                    self.kind(j + 1),
                    TokKind::Ident | TokKind::PrivateName | TokKind::Star
                )
            {
                j += 1;
            }
            if self.kind(j) == TokKind::Star {
                j += 1;
            }
            if !matches!(
                self.kind(j),
                TokKind::Ident | TokKind::PrivateName | TokKind::Str
            ) {
                j += 1;
                continue;
            }
            let mname = self.text(j).trim_matches(['"', '\'']).to_string();
            let mut q = j + 1;
            if self.kind(q) == TokKind::Question || (self.kind(q) == TokKind::Bang) {
                q += 1;
            }
            if self.kind(q) == TokKind::Lt {
                while q < c && self.kind(q) != TokKind::LParen {
                    q += 1;
                }
            }
            if self.kind(q) == TokKind::LParen {
                let Some(pc) = self.closer(q) else { break };
                let b = self.body_brace(pc + 1, c).unwrap_or(c);
                if self.kind(b) == TokKind::LBrace
                    && let Some(bc) = self.closer(b)
                {
                    def.methods
                        .push((mname.clone(), self.returns_in(b + 1, bc)));
                    def.bodies.push((mname, (b + 1, bc)));
                    j = bc + 1;
                    continue;
                }
                j = b + 1;
                continue;
            }
            // Field: `name (: type)? = expr`.
            let mut q2 = q;
            if self.kind(q2) == TokKind::Colon {
                while q2 < c && !matches!(self.kind(q2), TokKind::Assign | TokKind::Semi) {
                    if matches!(
                        self.kind(q2),
                        TokKind::LParen | TokKind::LBrace | TokKind::LBracket
                    ) {
                        q2 = self.closer(q2).unwrap_or(q2);
                    }
                    if q2 > q && self.toks[q2].flags & NEWLINE_BEFORE != 0 {
                        break;
                    }
                    q2 += 1;
                }
            }
            if self.kind(q2) == TokKind::Assign {
                let e = self.expr_end(q2 + 1, c);
                if let Some(r) = self.function_results(q2 + 1, e) {
                    def.methods.push((mname, r));
                } else {
                    def.fields.push((mname, (q2 + 1, e)));
                }
                j = e.max(q2 + 1);
                continue;
            }
            j = q2.max(j + 1);
        }
        self.classes.push(def);
    }

    /// The decoded value of a string token.
    pub fn str_value(&self, i: usize) -> Option<String> {
        let t = self.toks.get(i)?;
        let raw = t.text(&self.src);
        if raw.len() < 2 {
            return None;
        }
        decode(&raw[1..raw.len() - 1], EscapeRules::JS)
            .ok()
            .map(Cow::into_owned)
    }

    pub(crate) fn template_text(&self, i: usize) -> String {
        let t = &self.toks[i];
        let raw = t.text(&self.src);
        let (a, b) = match t.kind {
            TokKind::Template => (1, 1),
            TokKind::TemplateHead => (1, 2),
            TokKind::TemplateMiddle => (1, 2),
            TokKind::TemplateTail => (1, 1),
            _ => (0, 0),
        };
        if raw.len() < a + b {
            return String::new();
        }
        decode(&raw[a..raw.len() - b], EscapeRules::JS)
            .map(Cow::into_owned)
            .unwrap_or_default()
    }

    /// The class definition named `name`.
    pub fn class(&self, name: &str) -> Option<&ClassDef> {
        self.classes.iter().find(|c| c.name == name)
    }

    /// The import binding of `local`.
    pub fn import(&self, local: &str) -> Option<&Import> {
        self.imports.iter().find(|i| i.local == local)
    }

    /// The class whose body contains token `i`.
    pub fn enclosing_class(&self, i: usize) -> Option<&ClassDef> {
        // Class bodies are recorded through their method bodies.
        self.classes.iter().find(|c| {
            c.bodies.iter().any(|(_, (a, b))| *a <= i && i < *b)
                || c.fields.iter().any(|(_, (a, b))| *a <= i && i < *b)
        })
    }
}

/// Resolves first-party module specifiers to files.
pub trait ImportResolver {
    /// The file `spec` imported from `from` refers to, if first-party.
    fn resolve(&self, from: &Path, spec: &str) -> Option<PathBuf>;
}

impl ImportResolver for crate::common::SimpleResolver {
    fn resolve(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        crate::common::ModuleResolver::resolve(self, spec, from.parent()?)
    }
}

/// `(importing file id, local name)` → `(declaring file id, local name there)`.
type DeclMemo = HashMap<(u32, String), Option<(u32, String)>>;

/// A cache of parsed files plus the import resolver.
pub struct Files<'r> {
    resolver: &'r dyn ImportResolver,
    cache: RefCell<HashMap<PathBuf, Option<Rc<JsFile>>>>,
    by_id: RefCell<Vec<Rc<JsFile>>>,
    /// `(importing file id, local name)` → `(declaring file id, local name
    /// there)`.
    decls: RefCell<DeclMemo>,
}

impl<'r> Files<'r> {
    /// A cache over `resolver`.
    pub fn new(resolver: &'r dyn ImportResolver) -> Self {
        Files {
            resolver,
            cache: RefCell::new(HashMap::new()),
            by_id: RefCell::new(Vec::new()),
            decls: RefCell::new(HashMap::new()),
        }
    }

    /// The names under which the exports named in `names` can be imported from
    /// some module of `paths`: the names themselves and the names barrels
    /// re-export them under (`export { a as b } from './x'`, or imported
    /// and exported again). An import of any other name cannot refer to one
    /// of them.
    pub fn export_aliases(
        &self,
        paths: &[PathBuf],
        mut names: std::collections::HashSet<String>,
    ) -> std::collections::HashSet<String> {
        for _ in 0..6 {
            let before = names.len();
            for p in paths {
                let Some(f) = self.get(p) else { continue };
                for r in &f.reexports {
                    if let Some(e) = &r.exported
                        && names.contains(&r.imported)
                        && !names.contains(e)
                    {
                        names.insert(e.clone());
                    }
                }
                for (e, l) in &f.exports {
                    if e != l && names.contains(l) && !names.contains(e) {
                        names.insert(e.clone());
                    }
                }
            }
            if names.len() == before {
                break;
            }
        }
        names
    }

    /// Reads and parses `paths` on every core, so that later lookups find them
    /// parsed. Ids follow the order of `paths`.
    pub fn preload(&self, paths: &[PathBuf]) {
        use rayon::prelude::*;
        let todo: Vec<&PathBuf> = {
            let cache = self.cache.borrow();
            paths.iter().filter(|p| !cache.contains_key(*p)).collect()
        };
        let parsed: Vec<Option<JsFile>> = todo.par_iter().map(|p| JsFile::read(p)).collect();
        for (p, f) in todo.into_iter().zip(parsed) {
            let f = f.map(|f| self.register(f));
            self.cache.borrow_mut().insert(p.clone(), f);
        }
    }

    fn register(&self, mut f: JsFile) -> Rc<JsFile> {
        let mut ids = self.by_id.borrow_mut();
        f.id = ids.len() as u32 + 1;
        let rc = Rc::new(f);
        ids.push(rc.clone());
        rc
    }

    /// The parsed file at `path`.
    pub fn get(&self, path: &Path) -> Option<Rc<JsFile>> {
        if let Some(f) = self.cache.borrow().get(path) {
            return f.clone();
        }
        let f = JsFile::read(path).map(|f| self.register(f));
        self.cache
            .borrow_mut()
            .insert(path.to_path_buf(), f.clone());
        f
    }

    /// Inserts a parsed file.
    pub fn insert(&self, f: JsFile) -> Rc<JsFile> {
        let rc = self.register(f);
        self.cache
            .borrow_mut()
            .insert(rc.path.clone(), Some(rc.clone()));
        rc
    }

    /// The file with this id ([`JsFile::id`]).
    pub fn by_id(&self, id: u32) -> Option<Rc<JsFile>> {
        self.by_id
            .borrow()
            .get((id as usize).checked_sub(1)?)
            .cloned()
    }

    /// Resolves an import of `from`.
    pub fn resolve(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        self.resolver.resolve(from, spec)
    }

    /// The file and local name the export `name` of `f` is declared under,
    /// following `export { x } from`, `export * from` and
    /// imported-then-exported names.
    pub fn resolve_export(&self, f: &Rc<JsFile>, name: &str) -> Option<(Rc<JsFile>, String)> {
        self.resolve_export_d(f, name, 0)
    }

    fn resolve_export_d(
        &self,
        f: &Rc<JsFile>,
        name: &str,
        depth: u32,
    ) -> Option<(Rc<JsFile>, String)> {
        if depth > 8 {
            return None;
        }
        if let Some(local) = f.exports.get(name) {
            // `import { a } from './x'; export { a }`.
            if f.module_binding(local).is_none()
                && f.module_fn(local).is_none()
                && f.class(local).is_none()
                && let Some(imp) = f.import(local)
                && let Some(p) = self.resolve(&f.path, &imp.module)
                && let Some(g) = self.get(&p)
            {
                return match &imp.imported {
                    Imported::Named(n) => self.resolve_export_d(&g, n, depth + 1),
                    Imported::Default => self.resolve_export_d(&g, "default", depth + 1),
                    Imported::Namespace => None,
                };
            }
            return Some((f.clone(), local.clone()));
        }
        for r in &f.reexports {
            if r.exported.as_deref() == Some(name) {
                if r.imported == "*" {
                    return None;
                }
                let p = self.resolve(&f.path, &r.module)?;
                let g = self.get(&p)?;
                return self.resolve_export_d(&g, &r.imported, depth + 1);
            }
        }
        if name != "default" {
            for r in f.reexports.iter().filter(|r| r.exported.is_none()) {
                if let Some(p) = self.resolve(&f.path, &r.module)
                    && let Some(g) = self.get(&p)
                    && let Some(x) = self.resolve_export_d(&g, name, depth + 1)
                {
                    return Some(x);
                }
            }
        }
        None
    }

    /// The file and local name an imported binding refers to (following
    /// re-exports).
    pub fn imported_decl(&self, from: &JsFile, local: &str) -> Option<(Rc<JsFile>, String)> {
        if from.id == 0 {
            return self.imported_decl_uncached(from, local);
        }
        let key = (from.id, local.to_string());
        if let Some(hit) = self.decls.borrow().get(&key) {
            return hit
                .as_ref()
                .and_then(|(id, n)| Some((self.by_id(*id)?, n.clone())));
        }
        let r = self.imported_decl_uncached(from, local);
        self.decls
            .borrow_mut()
            .insert(key, r.as_ref().map(|(g, n)| (g.id, n.clone())));
        r
    }

    fn imported_decl_uncached(&self, from: &JsFile, local: &str) -> Option<(Rc<JsFile>, String)> {
        let imp = from.import(local)?;
        let path = self.resolve(&from.path, &imp.module)?;
        let f = self.get(&path)?;
        match &imp.imported {
            Imported::Named(n) => Some(
                self.resolve_export(&f, n)
                    .unwrap_or_else(|| (f.clone(), n.clone())),
            ),
            Imported::Default => self.resolve_export(&f, "default"),
            Imported::Namespace => None,
        }
    }

    /// The file a namespace import (`import * as x`) refers to.
    pub fn imported_namespace(&self, from: &JsFile, local: &str) -> Option<Rc<JsFile>> {
        let imp = from.import(local)?;
        if imp.imported != Imported::Namespace {
            return None;
        }
        let path = self.resolve(&from.path, &imp.module)?;
        self.get(&path)
    }

    /// Finds the class `name` visible from `file` (declared or imported).
    pub fn find_class(&self, file: &Rc<JsFile>, name: &str) -> Option<(Rc<JsFile>, usize)> {
        if let Some(i) = file.classes.iter().position(|c| c.name == name) {
            return Some((file.clone(), i));
        }
        let (f, n) = self.imported_decl(file, name)?;
        let i = f.classes.iter().position(|c| c.name == n)?;
        Some((f, i))
    }
}

/// One function activation: the callee's parameters are bound to the caller's
/// argument expressions, which are evaluated lazily in the caller's file and
/// environment.
pub struct Frame<'e> {
    /// The callee's file.
    pub file: Rc<JsFile>,
    /// The callee's index in `file.funcs`.
    pub func: usize,
    /// The caller's file.
    pub caller: Rc<JsFile>,
    /// Argument expression ranges in the caller.
    pub args: Vec<(usize, usize)>,
    /// The environment the arguments evaluate in.
    pub parent: Option<&'e Frame<'e>>,
    /// Values given directly for `(parameter index, field path)` instead of by
    /// an argument expression (a fetcher's first parameter bound to an SWR
    /// key).
    pub direct: Vec<(u8, Vec<String>, Vec<Tpl>)>,
    /// A unique id ([`Eval::frame_id`]): values are memoized per activation.
    pub id: u64,
}

const MAX_STEPS: u32 = 200_000;

type Memo = HashMap<(u64, u32, usize, Vec<String>), Vec<Tpl>>;

/// Leaf expression `(file id, first token, end token)` → where its value comes
/// from.
type SourceMemo = HashMap<(u32, usize, usize), Source>;

/// Binding `(file id, name token)` → the value an update of it sees.
type Overrides = Vec<((u32, usize), Vec<Tpl>)>;

/// The evaluator.
pub struct Eval<'a, 'r> {
    /// Files.
    pub files: &'a Files<'r>,
    /// The innermost function activation, if any.
    pub env: Option<&'a Frame<'a>>,
    /// Unbound parameters evaluate to [`Piece::Param`] instead of an unknown
    /// value.
    pub symbolic: bool,
    steps: Rc<Cell<u32>>,
    visiting: Rc<RefCell<Vec<(u32, usize)>>>,
    /// Values of bindings and parameters per activation: `(frame id, file id,
    /// token or parameter, field path)`.
    memo: Rc<RefCell<Memo>>,
    frame_ids: Rc<Cell<u64>>,
    /// Values of bindings while one of their updates (`x = x.replace(..)`) is
    /// evaluated: the update sees the value so far.
    overrides: Rc<RefCell<Overrides>>,
    /// Evaluation steps allowed per [`Eval::eval`].
    budget: u32,
    /// Sources of leaf expressions: `(file id, first token, end token)`.
    srcs: Rc<RefCell<SourceMemo>>,
}

/// What a piece means for a URL pattern: unknown parts with the same properties
/// are the same.
fn piece_class(p: &Piece) -> String {
    match p {
        Piece::Lit(s) => format!("L{s}"),
        Piece::Wild { multi, reason, src } => format!(
            "W{}{}{}{}{}",
            // Unknown parts supplied from outside the code stay apart from computed ones.
            if src.outside() { src.name() } else { "" },
            u8::from(*multi),
            u8::from(super::urlpat::reason_is_origin(reason)),
            u8::from(reason_is_pathish(reason)),
            u8::from(super::urlpat::reason_is_query(reason)),
        ),
        Piece::Param(r) => format!("P{}:{}:{}:{}", r.file, r.func, r.index, r.field.join(".")),
    }
}

fn cap(mut v: Vec<Tpl>) -> Vec<Tpl> {
    v.dedup();
    if v.len() > 4 {
        // Alternatives that differ only in the names of their unknown parts, or after
        // the start of a query string, are one.
        let mut seen: std::collections::HashSet<Vec<String>> = std::collections::HashSet::new();
        v.retain(|t| {
            let mut key = Vec::with_capacity(t.len());
            for p in t {
                if let Piece::Lit(l) = p
                    && let Some(q) = l.find(['?', '#'])
                {
                    if q > 0 {
                        key.push(format!("L{}", &l[..q]));
                    }
                    break;
                }
                if let Piece::Wild { reason, .. } = p
                    && !key.is_empty()
                    && super::urlpat::reason_is_query(reason)
                {
                    break;
                }
                key.push(piece_class(p));
            }
            seen.insert(key)
        });
    }
    if v.len() > MAX_ALTS {
        let src = source_of(&v[MAX_ALTS - 1..]);
        v.truncate(MAX_ALTS - 1);
        v.push(vec![Piece::Wild {
            multi: true,
            reason: format!("more than {MAX_ALTS} alternatives"),
            src,
        }]);
    }
    v
}

fn wild(reason: &str) -> Vec<Tpl> {
    vec![vec![Piece::wild(reason)]]
}

fn wild_from(reason: &str, src: Source) -> Vec<Tpl> {
    vec![vec![Piece::wild_from(reason, src)]]
}

/// The highest source of the unknown parts of `v`.
pub fn source_of(v: &[Tpl]) -> Source {
    v.iter()
        .flatten()
        .map(|p| match p {
            Piece::Lit(_) => Source::Code,
            Piece::Wild { src, .. } => *src,
            Piece::Param(r) => r.src,
        })
        .max()
        .unwrap_or_default()
}

/// Whether every alternative is a single wildcard.
pub fn all_unknown(v: &[Tpl]) -> bool {
    !v.is_empty()
        && v.iter()
            .all(|t| t.len() == 1 && matches!(t[0], Piece::Wild { .. }))
}

fn concat(a: &[Tpl], b: &[Tpl]) -> Vec<Tpl> {
    let mut out = Vec::new();
    for x in a {
        for y in b {
            let mut t = x.clone();
            t.extend(y.iter().cloned());
            out.push(t);
            if out.len() > MAX_ALTS {
                return cap(out);
            }
        }
    }
    out
}

/// The literal text of a fully known alternative.
fn full_lit(t: &Tpl) -> Option<String> {
    let mut s = String::new();
    for p in t {
        match p {
            Piece::Lit(l) => s.push_str(l),
            _ => return None,
        }
    }
    Some(s)
}

/// Whether the string `t` stands for starts with `s`, if that is decided.
fn tpl_starts_with(t: &Tpl, s: &str) -> Option<bool> {
    // A value that starts with an origin (`${process.env.API_URL}/v2/x`,
    // `${baseUrl}/x`) is an absolute URL: it does not start with a path.
    if s.starts_with('/')
        && let Some(Piece::Wild { reason, .. }) = t
            .iter()
            .find(|p| !matches!(p, Piece::Lit(l) if l.is_empty()))
        && super::urlpat::reason_is_origin(reason)
    {
        return Some(false);
    }
    let mut lead = String::new();
    let mut complete = true;
    for p in t {
        match p {
            Piece::Lit(l) => lead.push_str(l),
            _ => {
                complete = false;
                break;
            }
        }
    }
    if complete || lead.len() >= s.len() {
        Some(lead.starts_with(s))
    } else if s.starts_with(&lead) {
        None
    } else {
        Some(false)
    }
}

/// Whether the string `t` stands for ends with `s`, if that is decided.
fn tpl_ends_with(t: &Tpl, s: &str) -> Option<bool> {
    let mut tail = String::new();
    let mut complete = true;
    for p in t.iter().rev() {
        match p {
            Piece::Lit(l) => tail.insert_str(0, l),
            _ => {
                complete = false;
                break;
            }
        }
    }
    if complete || tail.len() >= s.len() {
        Some(tail.ends_with(s))
    } else if s.ends_with(&tail) {
        None
    } else {
        Some(false)
    }
}

/// `t.replace(from, to)` (the first occurrence; every one with `all`). An
/// occurrence inside an unknown part is not seen.
fn tpl_replace(t: &Tpl, from: &str, to: &str, all: bool) -> Tpl {
    if from.is_empty() {
        return t.clone();
    }
    let mut out = Vec::with_capacity(t.len());
    let mut done = false;
    for p in t {
        match p {
            Piece::Lit(l) if !done && l.contains(from) => {
                if all {
                    out.push(Piece::Lit(l.replace(from, to)));
                } else {
                    out.push(Piece::Lit(l.replacen(from, to, 1)));
                    done = true;
                }
            }
            other => out.push(other.clone()),
        }
    }
    out
}

/// A placeholder needle with an unknown name: `(prefix, suffix)` of `{${key}}`
/// or `:${key}`.
fn placeholder_shape(t: &Tpl) -> Option<(String, String)> {
    match t.as_slice() {
        [Piece::Lit(pre), w, Piece::Lit(suf)] if !matches!(w, Piece::Lit(_)) => {
            Some((pre.clone(), suf.clone()))
        }
        [Piece::Lit(pre), w] if !matches!(w, Piece::Lit(_)) && !pre.is_empty() => {
            Some((pre.clone(), String::new()))
        }
        _ => None,
    }
}

/// Replaces every `<pre>name<suf>` placeholder in the known parts of `t` by an
/// unknown segment part.
fn subst_placeholders(t: &Tpl, pre: &str, suf: &str) -> Tpl {
    let mut out = Vec::with_capacity(t.len());
    for p in t {
        let Piece::Lit(l) = p else {
            out.push(p.clone());
            continue;
        };
        let mut rest = l.as_str();
        let mut cur = String::new();
        while let Some(a) = rest.find(pre) {
            let after = &rest[a + pre.len()..];
            let end = if suf.is_empty() {
                let n = after
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (n > 0).then_some((n, 0))
            } else {
                after
                    .find(suf)
                    .filter(|&n| n > 0 && !after[..n].contains('/'))
                    .map(|n| (n, suf.len()))
            };
            let Some((n, slen)) = end else {
                cur.push_str(&rest[..a + pre.len()]);
                rest = after;
                continue;
            };
            cur.push_str(&rest[..a]);
            if !cur.is_empty() {
                out.push(Piece::Lit(std::mem::take(&mut cur)));
            }
            out.push(Piece::Wild {
                multi: false,
                reason: after[..n].to_string(),
                src: Source::Code,
            });
            rest = &after[n + slen..];
        }
        cur.push_str(rest);
        if !cur.is_empty() {
            out.push(Piece::Lit(cur));
        }
    }
    out
}

/// The parameters an evaluation result depends on.
pub fn params_in(v: &[Tpl]) -> Vec<super::urlpat::ParamRef> {
    let mut out: Vec<super::urlpat::ParamRef> = Vec::new();
    for t in v {
        for p in t {
            if let Piece::Param(r) = p
                && !out.contains(r)
            {
                out.push(r.clone());
            }
        }
    }
    out
}

/// Unbound parameters as unknown values.
pub fn demote(v: &[Tpl]) -> Vec<Tpl> {
    v.iter()
        .map(|t| t.iter().map(Piece::demote).collect())
        .collect()
}

struct Link {
    name: String,
    args: Option<(usize, usize)>,
    tok: usize,
}

enum Prop {
    Value(usize, usize),
    Method(usize),
    Shorthand(usize),
}

/// The property `key` of the object literal opened at `open`: the last
/// definition, the spread sources, and whether the literal has computed keys.
fn obj_prop(f: &JsFile, open: usize, key: &str) -> (Option<Prop>, Vec<(usize, usize)>, bool) {
    let mut found = None;
    let mut spreads = Vec::new();
    let mut computed = false;
    let Some(close) = f.closer(open) else {
        return (None, spreads, true);
    };
    for (p, q) in f.split_commas(open + 1, close) {
        if p >= q {
            continue;
        }
        if f.kind(p) == TokKind::Ellipsis {
            spreads.push((p + 1, q));
            continue;
        }
        if f.kind(p) == TokKind::LBracket {
            computed = true;
            continue;
        }
        let mut n = p;
        while n + 1 < q
            && matches!(f.text(n), "async" | "get" | "set" | "static")
            && matches!(f.kind(n + 1), TokKind::Ident | TokKind::Str | TokKind::Star)
        {
            n += 1;
        }
        if f.kind(n) == TokKind::Star {
            n += 1;
        }
        if f.text(n).trim_matches(['"', '\'']) != key {
            continue;
        }
        if f.kind(n + 1) == TokKind::Colon {
            found = Some(Prop::Value(n + 2, q));
        } else if f.kind(n + 1) == TokKind::LParen {
            if let Some(fi) = f
                .funcs
                .iter()
                .position(|fd| fd.start == n && fd.kind == FnKind::Method)
            {
                found = Some(Prop::Method(fi));
            }
        } else if q == n + 1 {
            found = Some(Prop::Shorthand(n));
        }
    }
    (found, spreads, computed)
}

impl<'a, 'r> Eval<'a, 'r> {
    /// An evaluator without an environment: parameters are unknown values.
    pub fn new(files: &'a Files<'r>) -> Self {
        Eval {
            files,
            env: None,
            symbolic: false,
            steps: Rc::new(Cell::new(0)),
            visiting: Rc::new(RefCell::new(Vec::new())),
            memo: Rc::new(RefCell::new(HashMap::new())),
            frame_ids: Rc::new(Cell::new(0)),
            overrides: Rc::new(RefCell::new(Vec::new())),
            budget: MAX_STEPS,
            srcs: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    /// The same evaluator with at most `steps` evaluation steps per expression
    /// (for evaluating many expressions of which few matter).
    pub fn with_budget(mut self, steps: u32) -> Self {
        self.budget = steps;
        self
    }

    /// A fresh id for a [`Frame`].
    pub fn frame_id(&self) -> u64 {
        let n = self.frame_ids.get() + 1;
        self.frame_ids.set(n);
        n
    }

    fn env_id(&self) -> u64 {
        self.env.map_or(0, |f| f.id)
    }

    /// An evaluator whose unbound parameters stay symbolic ([`Piece::Param`]).
    pub fn symbolic(files: &'a Files<'r>) -> Self {
        Eval {
            symbolic: true,
            ..Eval::new(files)
        }
    }

    /// The same evaluator in another environment.
    pub fn within<'b>(&'b self, env: Option<&'b Frame<'b>>) -> Eval<'b, 'r> {
        Eval {
            files: self.files,
            env,
            symbolic: self.symbolic,
            steps: self.steps.clone(),
            visiting: self.visiting.clone(),
            memo: self.memo.clone(),
            frame_ids: self.frame_ids.clone(),
            overrides: self.overrides.clone(),
            budget: self.budget,
            srcs: self.srcs.clone(),
        }
    }

    /// Where the value of the leaf expression `[a, b)` comes from
    /// ([`super::source`]).
    fn src(&self, f: &JsFile, a: usize, b: usize) -> Source {
        if a >= b {
            return Source::Code;
        }
        let key = (f.id, a, b);
        if let Some(s) = self.srcs.borrow().get(&key) {
            return *s;
        }
        let s = super::source::of_expr(self.files, f, a, b);
        self.srcs.borrow_mut().insert(key, s);
        s
    }

    /// The unknown value of the leaf expression `[a, b)`, named `text`.
    fn unk(&self, f: &JsFile, a: usize, b: usize, text: &str) -> Vec<Tpl> {
        wild_from(text, self.src(f, a, b))
    }

    /// The unknown value of `name` (resolved at token `at`) or of its field
    /// `path`.
    fn unk_name(&self, f: &JsFile, name: &str, at: usize, path: &[String], text: &str) -> Vec<Tpl> {
        wild_from(text, super::source::of_name(self.files, f, name, at, path))
    }

    fn tick(&self) -> bool {
        let s = self.steps.get();
        if s >= self.budget {
            return false;
        }
        self.steps.set(s + 1);
        true
    }

    /// Evaluates the token range `[a, b)` of `f`.
    pub fn eval(&self, f: &Rc<JsFile>, a: usize, b: usize) -> Vec<Tpl> {
        self.steps.set(0);
        self.visiting.borrow_mut().clear();
        self.ev(f, a, b, 0)
    }

    /// Evaluates the field `path` of the value of `[a, b)`.
    pub fn eval_field(&self, f: &Rc<JsFile>, a: usize, b: usize, path: &[String]) -> Vec<Tpl> {
        self.steps.set(0);
        self.visiting.borrow_mut().clear();
        self.field_of_expr(f, a, b, path, 0)
    }

    /// Strips `await`, wrapping parens, and trailing `!` / `as T` / `satisfies
    /// T`.
    pub(crate) fn strip(&self, f: &JsFile, mut a: usize, mut b: usize) -> (usize, usize) {
        loop {
            if a < b && (f.kw(a) == Kw::Await || f.kw(a) == Kw::Void) {
                a += 1;
                continue;
            }
            if a < b && f.kind(a) == TokKind::LParen && f.closer(a) == Some(b - 1) {
                a += 1;
                b -= 1;
                continue;
            }
            if b > a + 1 && f.kind(b - 1) == TokKind::Bang {
                b -= 1;
                continue;
            }
            if a < b
                && let Some(k) = (a + 1..b).find(|&k| {
                    f.kind(k) == TokKind::Ident
                        && matches!(f.text(k), "as" | "satisfies")
                        && f.toks[k].depth == f.toks[a].depth
                        && f.ends_value(k - 1)
                })
            {
                b = k;
                continue;
            }
            break;
        }
        (a, b)
    }

    /// The top-level `?`, `+`, `||`/`??` and `&&` operators of `[a, b)`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn top_ops(
        &self,
        f: &JsFile,
        a: usize,
        b: usize,
    ) -> (Option<usize>, Vec<usize>, Vec<usize>, Vec<usize>) {
        let mut q = None;
        let mut plus = Vec::new();
        let mut orr = Vec::new();
        let mut and = Vec::new();
        let mut k = a;
        while k < b {
            match f.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    if let Some(c) = f.closer(k).filter(|&c| c < b) {
                        k = c;
                    }
                }
                TokKind::Arrow => {
                    // A function value: its body is not this expression's operators.
                    return (None, Vec::new(), Vec::new(), Vec::new());
                }
                TokKind::Question if q.is_none() => q = Some(k),
                TokKind::Op => match f.text(k) {
                    "+" if k > a && f.ends_value(k - 1) => plus.push(k),
                    "??" | "||" => orr.push(k),
                    "&&" => and.push(k),
                    _ => {}
                },
                _ => {}
            }
            k += 1;
        }
        (q, plus, orr, and)
    }

    /// The `:` of the conditional whose `?` is at `qi`.
    pub(crate) fn ternary_colon(&self, f: &JsFile, qi: usize, b: usize) -> Option<usize> {
        let mut k = qi + 1;
        let mut nest = 0;
        while k < b {
            match f.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    if let Some(c) = f.closer(k).filter(|&c| c < b) {
                        k = c;
                    }
                }
                TokKind::Question => nest += 1,
                TokKind::Colon if nest == 0 => return Some(k),
                TokKind::Colon => nest -= 1,
                _ => {}
            }
            k += 1;
        }
        None
    }

    fn ev(&self, f: &Rc<JsFile>, a: usize, b: usize, depth: u32) -> Vec<Tpl> {
        if a >= b || depth > MAX_DEPTH || !self.tick() {
            if a < b && turborepo_trace_core::debug::var_os("EVAL_TRACE").is_some() {
                eprintln!(
                    "eval: limit depth {depth} steps {} {}:{}",
                    self.steps.get(),
                    f.path.display(),
                    f.line(a)
                );
            }
            return self.unk(f, a, b, &f.range_text(a, b));
        }
        let (a, b) = self.strip(f, a, b);
        if a >= b {
            return wild("");
        }
        // An immediately invoked function: `(() => { ... })()`.
        if f.kind(a) == TokKind::LParen
            && let Some(c) = f.closer(a)
            && f.kind(c + 1) == TokKind::LParen
            && f.closer(c + 1) == Some(b - 1)
            && let Some(fi) = f.fn_at(a + 1, c)
        {
            return self.call(f, f.split_commas(c + 2, b - 1), f, fi, depth);
        }
        let (q, plus, orr, and) = self.top_ops(f, a, b);
        if let Some(qi) = q
            && let Some(k) = self.ternary_colon(f, qi, b)
        {
            let c = self.cond(f, a, qi, depth + 1);
            let mut v = Vec::new();
            if c != Some(false) {
                v.extend(self.ev(f, qi + 1, k, depth + 1));
            }
            if c != Some(true) {
                v.extend(self.ev(f, k + 1, b, depth + 1));
            }
            return cap(v);
        }
        if !orr.is_empty() {
            let mut v = Vec::new();
            let mut s = a;
            for &o in orr.iter().chain(std::iter::once(&b)) {
                v.extend(self.ev(f, s, o, depth + 1));
                s = o + 1;
            }
            return cap(v);
        }
        if let Some(&last) = and.last() {
            // `cond && value`: the value, or nothing.
            return self.ev(f, last + 1, b, depth + 1);
        }
        if !plus.is_empty() {
            let mut acc: Vec<Tpl> = vec![Vec::new()];
            let mut s = a;
            for &o in plus.iter().chain(std::iter::once(&b)) {
                let part = self.ev(f, s, o, depth + 1);
                let part = if part.is_empty() {
                    self.unk(f, s, o, &f.range_text(s, o))
                } else {
                    part
                };
                acc = concat(&acc, &part);
                s = o + 1;
            }
            return cap(acc);
        }
        // Literals.
        match f.kind(a) {
            TokKind::Str if b == a + 1 => {
                return vec![vec![Piece::Lit(f.str_value(a).unwrap_or_default())]];
            }
            TokKind::Number if b == a + 1 => return vec![vec![Piece::Lit(f.text(a).to_string())]],
            TokKind::Template if b == a + 1 => return vec![vec![Piece::Lit(f.template_text(a))]],
            TokKind::TemplateHead if f.closer(a) == Some(b - 1) => {
                return self.template(f, a, b - 1, depth);
            }
            TokKind::Ident if b == a + 1 => return self.ident(f, a, depth),
            TokKind::LBracket => {
                if let Some(v) = self.array_join(f, a, b, depth) {
                    return v;
                }
            }
            _ => {}
        }
        // `new URL(x, base)` / `new URL(x).pathname`.
        if f.kw(a) == Kw::New && f.text(a + 1) == "URL" && f.kind(a + 2) == TokKind::LParen {
            let c = f.closer(a + 2).unwrap_or(b);
            let args = f.split_commas(a + 3, c);
            if let Some(&(x, y)) = args.first() {
                let v = self.ev(f, x, y, depth + 1);
                if c + 1 == b
                    || (f.kind(c + 1) == TokKind::Dot
                        && matches!(f.text(c + 2), "pathname" | "href" | "toString"))
                {
                    return v;
                }
            }
        }
        // Member / call chains.
        if let Some(v) = self.chain(f, a, b, depth) {
            return v;
        }
        // Fallback: the shared evaluator.
        let consts = HashMap::new();
        let sc = Scope {
            src: &f.src,
            consts: &consts,
            apis: &[],
            loaders: &[],
            file_name: None,
        };
        let out = evaluator::evaluate(&sc, &f.toks[a..b]);
        match out.val {
            JsVal::Str(v) => value_tpls(&v),
            _ => self.unk(f, a, b, &f.range_text(a, b)),
        }
    }

    /// `[a, b, c].join('/')`, with an optional `.filter(Boolean)` before.
    fn array_join(&self, f: &Rc<JsFile>, a: usize, b: usize, depth: u32) -> Option<Vec<Tpl>> {
        let c = f.closer(a)?;
        let mut k = c + 1;
        let mut sep = None;
        while k < b {
            if !matches!(f.kind(k), TokKind::Dot | TokKind::OptChain)
                || f.kind(k + 1) != TokKind::Ident
                || f.kind(k + 2) != TokKind::LParen
            {
                return None;
            }
            let pc = f.closer(k + 2)?;
            match f.text(k + 1) {
                "filter" => {}
                "map"
                    if pc == k + 4
                        && matches!(
                            f.text(k + 3),
                            "encodeURIComponent" | "encodeURI" | "String"
                        ) => {}
                "join" if pc + 1 == b => {
                    sep = Some(if pc == k + 3 {
                        ",".to_string()
                    } else {
                        full_lit(self.ev(f, k + 3, pc, depth + 1).first()?)?
                    });
                }
                _ => return None,
            }
            k = pc + 1;
        }
        let sep = sep?;
        let mut acc: Vec<Tpl> = vec![Vec::new()];
        let mut first = true;
        for (x, y) in f.split_commas(a + 1, c) {
            if x >= y {
                continue;
            }
            let part = if f.kind(x) == TokKind::Ellipsis {
                vec![vec![Piece::Wild {
                    multi: true,
                    reason: f.range_text(x, y),
                    src: self.src(f, x, y),
                }]]
            } else {
                self.ev(f, x, y, depth + 1)
            };
            if part.is_empty() {
                continue;
            }
            if !first {
                acc = concat(&acc, &[vec![Piece::Lit(sep.clone())]]);
            }
            first = false;
            acc = concat(&acc, &part);
        }
        Some(cap(acc))
    }

    fn template(&self, f: &Rc<JsFile>, head: usize, tail: usize, depth: u32) -> Vec<Tpl> {
        let mut acc: Vec<Tpl> = vec![vec![Piece::Lit(f.template_text(head))]];
        let mut s = head + 1;
        let mut k = head + 1;
        while k <= tail {
            match f.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead
                    if k < tail =>
                {
                    if let Some(c) = f.closer(k).filter(|&c| c < tail) {
                        k = c;
                    }
                }
                TokKind::TemplateMiddle | TokKind::TemplateTail
                    if k == tail || f.kind(k) == TokKind::TemplateMiddle =>
                {
                    let sub = self.ev(f, s, k, depth + 1);
                    let sub = if sub.is_empty() || all_unknown(&sub) {
                        wild_from(&f.range_text(s, k), source_of(&sub).max(self.src(f, s, k)))
                    } else {
                        sub
                    };
                    acc = concat(&acc, &sub);
                    acc = concat(&acc, &[vec![Piece::Lit(f.template_text(k))]]);
                    s = k + 1;
                    if k == tail {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        cap(acc)
    }

    fn ident(&self, f: &Rc<JsFile>, i: usize, depth: u32) -> Vec<Tpl> {
        let name = f.text(i);
        // No value: drops out of alternatives (`cond ? url : null` SWR keys).
        if matches!(name, "null" | "undefined") {
            return Vec::new();
        }
        self.ident_field(f, i, name, &[], depth)
    }

    /// The value of `name` (resolved at token `at`), or of its field `path`.
    fn ident_field(
        &self,
        f: &Rc<JsFile>,
        at: usize,
        name: &str,
        path: &[String],
        depth: u32,
    ) -> Vec<Tpl> {
        let full = || {
            if path.is_empty() {
                name.to_string()
            } else {
                format!("{name}.{}", path.join("."))
            }
        };
        let unknown = || self.unk_name(f, name, at, path, &full());
        if depth > MAX_DEPTH {
            return unknown();
        }
        match f.lookup(name, at) {
            Some(Decl::Param { func, param }) => self.param_value(f, func, param, path, depth),
            Some(Decl::Binding(bi)) => {
                let bnd = &f.bindings[name][bi];
                let key = (f.id, bnd.at);
                if path.is_empty()
                    && let Some((_, v)) = self
                        .overrides
                        .borrow()
                        .iter()
                        .rev()
                        .find(|(k, _)| *k == key)
                {
                    return v.clone();
                }
                if self.visiting.borrow().contains(&key) {
                    if turborepo_trace_core::debug::var_os("EVAL_TRACE").is_some() {
                        eprintln!("eval: cycle {}:{} {}", f.path.display(), f.line(at), full());
                    }
                    return unknown();
                }
                let mkey = (self.env_id(), f.id, bnd.at, path.to_vec());
                if !bnd.mutable
                    && let Some(v) = self.memo.borrow().get(&mkey)
                {
                    return v.clone();
                }
                let mut v = match bnd.init {
                    Some((x, y)) if !(x <= at && at < y) => {
                        self.visiting.borrow_mut().push(key);
                        let v = if bnd.path.is_empty() {
                            self.field_of_expr(f, x, y, path, depth + 1)
                        } else {
                            let mut p = bnd.path.clone();
                            p.extend_from_slice(path);
                            self.field_of_expr(f, x, y, &p, depth + 1)
                        };
                        self.visiting.borrow_mut().pop();
                        v
                    }
                    Some(_) => {
                        if turborepo_trace_core::debug::var_os("EVAL_TRACE").is_some() {
                            eprintln!("eval: self {}:{} {}", f.path.display(), f.line(at), full());
                        }
                        return unknown();
                    }
                    None => Vec::new(),
                };
                // React state holds what its setter is given: unknown parts of the value
                // come from there too.
                if super::source::is_state(f, name, bi) && !v.is_empty() {
                    let s = super::source::of_binding(self.files, f, name, bi);
                    for p in v.iter_mut().flatten() {
                        if let Piece::Wild { src, .. } = p {
                            *src = (*src).max(s);
                        }
                    }
                }
                if bnd.mutable
                    && path.is_empty()
                    && let Some(asg) = f.assigns.get(name)
                {
                    let (s, e) = if bnd.scope == usize::MAX {
                        (0, f.toks.len())
                    } else {
                        (bnd.scope, f.closer(bnd.scope).unwrap_or(f.toks.len()))
                    };
                    self.visiting.borrow_mut().push(key);
                    for x in asg.iter().filter(|x| s <= x.at && x.at <= e) {
                        if x.value.0 <= at && at < x.value.1 {
                            continue;
                        }
                        self.overrides.borrow_mut().push((key, v.clone()));
                        let rhs = self.ev(f, x.value.0, x.value.1, depth + 1);
                        self.overrides.borrow_mut().pop();
                        if x.append {
                            let base = if v.is_empty() {
                                vec![Vec::new()]
                            } else {
                                v.clone()
                            };
                            let rhs = if rhs.is_empty() { wild(name) } else { rhs };
                            v.extend(concat(&base, &rhs));
                        } else {
                            v.extend(rhs);
                        }
                    }
                    self.visiting.borrow_mut().pop();
                    v = cap(v);
                }
                if bnd.init.is_none() && v.is_empty() {
                    if turborepo_trace_core::debug::var_os("EVAL_TRACE").is_some() {
                        eprintln!(
                            "eval: noinit {}:{} {}",
                            f.path.display(),
                            f.line(at),
                            full()
                        );
                    }
                    return unknown();
                }
                if !bnd.mutable
                    && self.visiting.borrow().is_empty()
                    && self.overrides.borrow().is_empty()
                {
                    self.memo.borrow_mut().insert(mkey, v.clone());
                }
                v
            }
            Some(Decl::Import(ii)) => {
                let imp = &f.imports[ii];
                if imp.imported == Imported::Namespace {
                    // `ns.CONST`.
                    if let Some((first, rest)) = path.split_first()
                        && let Some(g) = self.files.imported_namespace(f, name)
                        && let Some((h, n)) = self.files.resolve_export(&g, first)
                        && let Some(bnd) = h.module_binding(&n)
                        && let Some((x, y)) = bnd.init
                    {
                        let mut p = bnd.path.clone();
                        p.extend_from_slice(rest);
                        return self.within(None).field_of_expr(&h, x, y, &p, depth + 1);
                    }
                    return unknown();
                }
                if let Some((g, n)) = self.files.imported_decl(f, name)
                    && let Some(bnd) = g.module_binding(&n)
                    && let Some((x, y)) = bnd.init
                    && g.fn_at(x, y).is_none()
                {
                    let mut p = bnd.path.clone();
                    p.extend_from_slice(path);
                    return self.within(None).field_of_expr(&g, x, y, &p, depth + 1);
                }
                unknown()
            }
            _ => unknown(),
        }
    }

    /// The value of a parameter (or of its field `path`): the caller's argument
    /// if the function is active in the environment, else symbolic or
    /// unknown.
    fn param_value(
        &self,
        f: &Rc<JsFile>,
        func: usize,
        param: usize,
        path: &[String],
        depth: u32,
    ) -> Vec<Tpl> {
        let p = &f.funcs[func].params[param];
        let mut full: Vec<String> = p.field.iter().cloned().collect();
        full.extend_from_slice(path);
        let name = if path.is_empty() {
            p.name.clone()
        } else {
            format!("{}.{}", p.name, path.join("."))
        };
        let chain: Vec<&str> = path.iter().map(String::as_str).collect();
        let src = super::source::param_source(f, func, param, &chain);
        if p.rest {
            return wild_from(&name, src);
        }
        let mut fr = self.env;
        while let Some(x) = fr {
            if x.file.id == f.id && x.func == func {
                if let Some((_, _, v)) = x
                    .direct
                    .iter()
                    .find(|(i, fp, _)| *i == p.index && *fp == full)
                {
                    return v.clone();
                }
                let mkey = (x.id, f.id, usize::MAX - p.index as usize, full.clone());
                if let Some(v) = self.memo.borrow().get(&mkey) {
                    return v.clone();
                }
                let v = match x.args.get(p.index as usize) {
                    Some(&(a, b)) if a < b && x.caller.kind(a) != TokKind::Ellipsis => {
                        let e2 = self.within(x.parent);
                        e2.field_of_expr(&x.caller, a, b, &full, depth + 1)
                    }
                    Some(_) => return wild_from(&name, src),
                    None => Vec::new(),
                };
                if !v.is_empty()
                    && self.visiting.borrow().is_empty()
                    && self.overrides.borrow().is_empty()
                {
                    self.memo.borrow_mut().insert(mkey, v.clone());
                }
                if v.is_empty()
                    && let Some((a, b)) = p.default
                {
                    // `undefined`: the default value.
                    return self.field_of_expr(f, a, b, path, depth + 1);
                }
                if v.is_empty() && !full.is_empty() && x.args.get(p.index as usize).is_none() {
                    // A field of a missing argument with a whole-parameter default (`opts = {}`).
                    return Vec::new();
                }
                return v;
            }
            fr = x.parent;
        }
        if self.symbolic {
            return vec![vec![Piece::Param(super::urlpat::ParamRef {
                file: f.id,
                func: func as u32,
                index: p.index,
                field: full,
                name,
                src,
            })]];
        }
        wild_from(&name, src)
    }

    /// The value of the field `path` of the expression `[a, b)`.
    fn field_of_expr(
        &self,
        f: &Rc<JsFile>,
        a: usize,
        b: usize,
        path: &[String],
        depth: u32,
    ) -> Vec<Tpl> {
        if path.is_empty() {
            return self.ev(f, a, b, depth);
        }
        let unknown = |f: &JsFile, a: usize, b: usize| {
            wild_from(
                &format!("{}.{}", f.range_text(a, b), path.join(".")),
                super::source::of_member(self.files, f, a, b),
            )
        };
        if a >= b || depth > MAX_DEPTH || !self.tick() {
            return unknown(f, a, b);
        }
        let (a, b) = self.strip(f, a, b);
        if a >= b {
            return unknown(f, a, b);
        }
        let (q, _, orr, and) = self.top_ops(f, a, b);
        if let Some(qi) = q
            && let Some(k) = self.ternary_colon(f, qi, b)
        {
            let c = self.cond(f, a, qi, depth + 1);
            let mut v = Vec::new();
            if c != Some(false) {
                v.extend(self.field_of_expr(f, qi + 1, k, path, depth + 1));
            }
            if c != Some(true) {
                v.extend(self.field_of_expr(f, k + 1, b, path, depth + 1));
            }
            return cap(v);
        }
        if !orr.is_empty() {
            let mut v = Vec::new();
            let mut s = a;
            for &o in orr.iter().chain(std::iter::once(&b)) {
                v.extend(self.field_of_expr(f, s, o, path, depth + 1));
                s = o + 1;
            }
            return cap(v);
        }
        if let Some(&last) = and.last() {
            return self.field_of_expr(f, last + 1, b, path, depth + 1);
        }
        match f.kind(a) {
            TokKind::LBrace if f.closer(a) == Some(b - 1) => {
                let (prop, spreads, computed) = obj_prop(f, a, &path[0]);
                match prop {
                    Some(Prop::Value(x, y)) => self.field_of_expr(f, x, y, &path[1..], depth + 1),
                    Some(Prop::Shorthand(t)) => {
                        self.field_of_expr(f, t, t + 1, &path[1..], depth + 1)
                    }
                    Some(Prop::Method(_)) => unknown(f, a, a + 1),
                    None => {
                        let mut v = Vec::new();
                        for (x, y) in spreads {
                            v.extend(self.field_of_expr(f, x, y, path, depth + 1));
                        }
                        if computed {
                            v.extend(wild(&path.join(".")));
                        }
                        cap(v)
                    }
                }
            }
            TokKind::LBracket if f.closer(a) == Some(b - 1) => {
                let Ok(n) = path[0].parse::<usize>() else {
                    return unknown(f, a, b);
                };
                let items = f.split_commas(a + 1, b - 1);
                if items
                    .iter()
                    .take(n + 1)
                    .any(|&(x, _)| f.kind(x) == TokKind::Ellipsis)
                {
                    return unknown(f, a, b);
                }
                match items.get(n) {
                    Some(&(x, y)) if x < y => self.field_of_expr(f, x, y, &path[1..], depth + 1),
                    _ => Vec::new(),
                }
            }
            TokKind::Ident => {
                if let Some(links) = self.links(f, a, b)
                    && links.iter().all(|l| l.args.is_none())
                {
                    let mut full: Vec<String> = links[1..].iter().map(|l| l.name.clone()).collect();
                    full.extend_from_slice(path);
                    return self.ident_field(f, a, &links[0].name, &full, depth);
                }
                // A field of a call's result: `const { url } = build(path)`.
                if let Some(links) = self.links(f, a, b)
                    && links.len() == 1
                    && let Some((x, y)) = links[0].args
                {
                    let callee = match f.lookup(&links[0].name, a) {
                        Some(Decl::Func(fi)) => Some((f.clone(), fi)),
                        Some(Decl::Import(_)) => self
                            .files
                            .imported_decl(f, &links[0].name)
                            .and_then(|(g, n)| g.module_fn(&n).map(|fi| (g, fi))),
                        _ => None,
                    };
                    if let Some((g, fi)) = callee {
                        if depth > MAX_DEPTH {
                            return unknown(f, a, b);
                        }
                        let frame = Frame {
                            file: g.clone(),
                            func: fi,
                            caller: f.clone(),
                            args: f.split_commas(x, y),
                            parent: self.env,
                            direct: Vec::new(),
                            id: self.frame_id(),
                        };
                        let e2 = self.within(Some(&frame));
                        return e2.fn_results_field(&g, fi, path, depth + 1);
                    }
                }
                unknown(f, a, b)
            }
            _ => unknown(f, a, b),
        }
    }

    /// Whether the condition `[a, b)` holds, if that is decided.
    pub fn cond(&self, f: &Rc<JsFile>, a: usize, b: usize, depth: u32) -> Option<bool> {
        if a >= b || depth > MAX_DEPTH || !self.tick() {
            return None;
        }
        let (a, b) = self.strip(f, a, b);
        if a >= b {
            return None;
        }
        let (q, _, orr, and) = self.top_ops(f, a, b);
        if q.is_some() {
            return None;
        }
        if !orr.is_empty() {
            if orr.iter().any(|&o| f.text(o) == "??") {
                return None;
            }
            let mut all_false = true;
            let mut s = a;
            for &o in orr.iter().chain(std::iter::once(&b)) {
                match self.cond(f, s, o, depth + 1) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => all_false = false,
                }
                s = o + 1;
            }
            return all_false.then_some(false);
        }
        if !and.is_empty() {
            let mut all_true = true;
            let mut s = a;
            for &o in and.iter().chain(std::iter::once(&b)) {
                match self.cond(f, s, o, depth + 1) {
                    Some(false) => return Some(false),
                    Some(true) => {}
                    None => all_true = false,
                }
                s = o + 1;
            }
            return all_true.then_some(true);
        }
        if f.kind(a) == TokKind::Bang {
            return self.cond(f, a + 1, b, depth + 1).map(|x| !x);
        }
        // `x === 'lit'`, `x !== 'lit'`.
        let mut k = a;
        while k < b {
            match f.kind(k) {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    k = f.closer(k).filter(|&c| c < b)?;
                }
                TokKind::Op if matches!(f.text(k), "===" | "==" | "!==" | "!=") => {
                    if f.kw(a) == Kw::Typeof {
                        return None;
                    }
                    let l = self.ev(f, a, k, depth + 1);
                    let r = self.ev(f, k + 1, b, depth + 1);
                    let (Some(l), Some(r)) = (
                        (l.len() == 1).then(|| full_lit(&l[0])).flatten(),
                        (r.len() == 1).then(|| full_lit(&r[0])).flatten(),
                    ) else {
                        return None;
                    };
                    let eq = l == r;
                    return Some(if f.text(k).starts_with('!') { !eq } else { eq });
                }
                TokKind::Op | TokKind::Lt | TokKind::Gt => return None,
                _ => {}
            }
            k += 1;
        }
        // `x.startsWith('lit')`, `x.endsWith('lit')`.
        if let Some(links) = self.links(f, a, b)
            && links.len() >= 2
            && let Some(last) = links.last()
            && let Some((x, y)) = last.args
            && matches!(last.name.as_str(), "startsWith" | "endsWith")
            && links[..links.len() - 1].iter().all(|l| l.args.is_none())
        {
            let arg = self.ev(f, x, y, depth + 1);
            let s = (arg.len() == 1).then(|| full_lit(&arg[0])).flatten()?;
            let recv = self.ev(f, a, last.tok - 1, depth + 1);
            if recv.is_empty() {
                return None;
            }
            let mut res: Option<bool> = None;
            for t in &recv {
                let r = if last.name == "startsWith" {
                    tpl_starts_with(t, &s)
                } else {
                    tpl_ends_with(t, &s)
                }?;
                match res {
                    None => res = Some(r),
                    Some(x) if x != r => return None,
                    _ => {}
                }
            }
            return res;
        }
        // Truthiness of a plain name or member: a known non-empty string, or no value.
        if let Some(links) = self.links(f, a, b)
            && links.iter().all(|l| l.args.is_none())
        {
            let v = self.ev(f, a, b, depth + 1);
            let lits: Option<Vec<String>> = v.iter().map(full_lit).collect();
            let lits = lits?;
            if !lits.is_empty() && lits.iter().all(|l| !l.is_empty()) {
                return Some(true);
            }
            if !lits.is_empty() && lits.iter().all(String::is_empty) {
                return Some(false);
            }
        }
        None
    }

    /// Calls `g.funcs[fi]` with the caller's argument expressions.
    pub fn call(
        &self,
        caller: &Rc<JsFile>,
        args: Vec<(usize, usize)>,
        g: &Rc<JsFile>,
        fi: usize,
        depth: u32,
    ) -> Vec<Tpl> {
        if depth > MAX_DEPTH {
            return wild(g.funcs[fi].name.as_deref().unwrap_or("call"));
        }
        let frame = Frame {
            file: g.clone(),
            func: fi,
            caller: caller.clone(),
            args,
            parent: self.env,
            direct: Vec::new(),
            id: self.frame_id(),
        };
        let e2 = self.within(Some(&frame));
        e2.fn_results(g, fi, depth + 1)
    }

    /// The results of `g.funcs[fi]` in the current environment: its `return`
    /// expressions whose guards may hold.
    pub fn fn_results(&self, g: &Rc<JsFile>, fi: usize, depth: u32) -> Vec<Tpl> {
        self.fn_results_field(g, fi, &[], depth)
    }

    /// [`Eval::fn_results`] for the field `path` of the results.
    fn fn_results_field(&self, g: &Rc<JsFile>, fi: usize, path: &[String], depth: u32) -> Vec<Tpl> {
        let fd = &g.funcs[fi];
        if !fd.block {
            return self.field_of_expr(g, fd.body.0, fd.body.1, path, depth);
        }
        let rets = g.guarded_returns(fd.body.0, fd.body.1);
        if rets.is_empty() {
            return wild("function without return");
        }
        let mut v = Vec::new();
        for (guards, (x, y)) in rets {
            let mut possible = true;
            let mut certain = true;
            for gd in guards {
                match gd {
                    Guard::Opaque => certain = false,
                    Guard::Cond(ca, cb, want) => match self.cond(g, ca, cb, depth + 1) {
                        Some(t) if t != want => {
                            possible = false;
                            break;
                        }
                        Some(_) => {}
                        None => certain = false,
                    },
                }
            }
            if !possible {
                continue;
            }
            v.extend(self.field_of_expr(g, x, y, path, depth + 1));
            if certain {
                break;
            }
        }
        cap(v)
    }

    fn results(&self, f: &Rc<JsFile>, rs: &[(usize, usize)], depth: u32) -> Vec<Tpl> {
        if rs.is_empty() {
            return wild("function without return");
        }
        let mut v = Vec::new();
        for &(a, b) in rs {
            v.extend(self.ev(f, a, b, depth + 1));
        }
        cap(v)
    }

    /// The class methods named `method` of class `cls` (with inheritance),
    /// evaluated.
    pub fn class_method(
        &self,
        file: &Rc<JsFile>,
        cls: &str,
        method: &str,
        depth: u32,
    ) -> Option<Vec<Tpl>> {
        let mut cur = self.files.find_class(file, cls);
        let mut guard = 0;
        while let Some((cf, ci)) = cur {
            guard += 1;
            if guard > 8 {
                break;
            }
            let c = &cf.classes[ci];
            if let Some((_, rs)) = c.methods.iter().find(|(n, _)| n == method) {
                return Some(self.results(&cf, rs, depth));
            }
            if let Some((_, (a, b))) = c.fields.iter().find(|(n, _)| n == method) {
                return Some(self.ev(&cf, *a, *b, depth + 1));
            }
            let parent = c.parent.clone();
            cur = parent.and_then(|p| self.files.find_class(&cf, &p));
        }
        None
    }

    /// Parses `root(.name | ['name'] | (args) | <T> | !)*` over exactly `[a,
    /// b)`.
    fn links(&self, f: &JsFile, a: usize, b: usize) -> Option<Vec<Link>> {
        if f.kind(a) != TokKind::Ident || f.kw(a) == Kw::New {
            return None;
        }
        let mut names = vec![Link {
            name: f.text(a).to_string(),
            args: None,
            tok: a,
        }];
        let mut k = a + 1;
        while k < b {
            match f.kind(k) {
                TokKind::Dot | TokKind::OptChain
                    if f.kind(k + 1) == TokKind::Ident || f.kind(k + 1) == TokKind::PrivateName =>
                {
                    names.push(Link {
                        name: f.text(k + 1).to_string(),
                        args: None,
                        tok: k + 1,
                    });
                    k += 2;
                }
                TokKind::OptChain if f.kind(k + 1) == TokKind::LParen => k += 1,
                TokKind::Bang => k += 1,
                TokKind::LBracket => {
                    let c = f.closer(k)?;
                    if c != k + 2 || c >= b {
                        return None;
                    }
                    let name = match f.kind(k + 1) {
                        TokKind::Str => f.str_value(k + 1)?,
                        TokKind::Number => f.text(k + 1).to_string(),
                        _ => return None,
                    };
                    names.push(Link {
                        name,
                        args: None,
                        tok: k + 1,
                    });
                    k = c + 1;
                }
                TokKind::LParen => {
                    let c = f.closer(k)?;
                    if c >= b {
                        return None;
                    }
                    let last = names.last_mut()?;
                    if last.args.is_some() {
                        // A call of a call result.
                        return None;
                    }
                    last.args = Some((k + 1, c));
                    k = c + 1;
                }
                TokKind::Lt => {
                    // Type arguments before a call.
                    let g = f.type_args_end(k).filter(|&g| g < b)?;
                    k = g + 1;
                }
                _ => return None,
            }
        }
        Some(names)
    }

    fn chain(&self, f: &Rc<JsFile>, a: usize, b: usize, depth: u32) -> Option<Vec<Tpl>> {
        let mut names = self.links(f, a, b)?;
        // Strip trailing `.toString()`, `.pathname`, `.href`.
        while names.len() > 1 {
            let l = names.last()?;
            let strip = matches!(l.name.as_str(), "toString" | "trim" | "valueOf")
                && l.args.is_some()
                || matches!(l.name.as_str(), "pathname" | "href") && l.args.is_none();
            if !strip {
                break;
            }
            names.pop();
        }
        // String methods on the value before them.
        if names.len() >= 2
            && let Some(last) = names.last()
            && let Some((x, y)) = last.args
        {
            let recv_end = last.tok - 1;
            match last.name.as_str() {
                "replace" | "replaceAll" => {
                    let args = f.split_commas(x, y);
                    if args.len() != 2 {
                        return None;
                    }
                    let recv = self.ev(f, a, recv_end, depth + 1);
                    let to = self.ev(f, args[1].0, args[1].1, depth + 1);
                    let to = (to.len() == 1).then(|| full_lit(&to[0])).flatten();
                    let (p, q) = args[0];
                    if q == p + 1 && f.kind(p) == TokKind::Regex {
                        // Slash trimming: `/^\//`, `/^\/+/`, `/\/$/`, `/\/+$/`.
                        let re = f.text(p);
                        let re = re.rsplit_once('/').map_or(re, |x| x.0);
                        let lead = matches!(re, "/^\\/" | "/^\\/+" | "/^\\/*");
                        let trail = matches!(re, "/\\/$" | "/\\/+$" | "/\\/*$");
                        if (lead || trail) && to.as_deref() == Some("") {
                            return Some(
                                recv.into_iter()
                                    .map(|mut t| {
                                        if lead && let Some(Piece::Lit(l)) = t.first_mut() {
                                            *l = l.trim_start_matches('/').to_string();
                                        }
                                        if trail && let Some(Piece::Lit(l)) = t.last_mut() {
                                            *l = l.trim_end_matches('/').to_string();
                                        }
                                        t
                                    })
                                    .collect(),
                            );
                        }
                        return None;
                    }
                    let from = self.ev(f, p, q, depth + 1);
                    // A placeholder with an unknown name (`{${key}}`, `:${key}`): path
                    // parameter substitution. Every such placeholder becomes an unknown
                    // segment part.
                    if let [t] = from.as_slice()
                        && let Some((pre, suf)) = placeholder_shape(t)
                    {
                        return Some(
                            recv.iter()
                                .map(|t| subst_placeholders(t, &pre, &suf))
                                .collect(),
                        );
                    }
                    let from = (from.len() == 1).then(|| full_lit(&from[0])).flatten()?;
                    let all = last.name == "replaceAll";
                    if let Some(to) = to {
                        return Some(
                            recv.iter()
                                .map(|t| tpl_replace(t, &from, &to, all))
                                .collect(),
                        );
                    }
                    // A computed replacement: known when the receiver starts with the
                    // needle (`url.replace('https://host', base)`), unchanged when a fully
                    // known receiver does not contain it.
                    let to = self.ev(f, args[1].0, args[1].1, depth + 1);
                    let mut out = Vec::new();
                    for t in &recv {
                        match t.first() {
                            Some(Piece::Lit(l)) if l.starts_with(&from) => {
                                let mut rest = t.clone();
                                rest[0] = Piece::Lit(l[from.len()..].to_string());
                                out.extend(concat(&to, &[rest]));
                            }
                            _ if full_lit(t).is_some_and(|l| !l.contains(&from)) => {
                                out.push(t.clone())
                            }
                            _ => return None,
                        }
                    }
                    return Some(cap(out));
                }
                "toUpperCase" | "toLowerCase" => {
                    let upper = last.name == "toUpperCase";
                    let recv = self.ev(f, a, recv_end, depth + 1);
                    return Some(
                        recv.into_iter()
                            .map(|t| {
                                t.into_iter()
                                    .map(|p| match p {
                                        Piece::Lit(l) if upper => Piece::Lit(l.to_uppercase()),
                                        Piece::Lit(l) => Piece::Lit(l.to_lowercase()),
                                        other => other,
                                    })
                                    .collect()
                            })
                            .collect(),
                    );
                }
                "concat" => {
                    let mut acc = self.ev(f, a, recv_end, depth + 1);
                    for (p, q) in f.split_commas(x, y) {
                        let part = self.ev(f, p, q, depth + 1);
                        if part.is_empty() {
                            return None;
                        }
                        acc = concat(&acc, &part);
                    }
                    return Some(cap(acc));
                }
                _ => {}
            }
        }
        let first_arg = |args: &Option<(usize, usize)>| -> Option<(usize, usize)> {
            let (x, y) = (*args)?;
            f.split_commas(x, y).into_iter().next()
        };
        let root = &names[0].name;
        let root_args = &names[0].args;
        if names.len() == 1 {
            let Some(args) = root_args.as_ref() else {
                // `x.toString()`, `x.href`.
                return Some(self.ident(f, a, depth));
            };
            // Pass-through helpers.
            let lower = root.to_ascii_lowercase();
            if matches!(
                root.as_str(),
                "String" | "encodeURI" | "encodeURIComponent" | "decodeURIComponent"
            ) || [
                "withquery",
                "addquery",
                "appendquery",
                "setquery",
                "withsearchparams",
                "withparams",
                "addsearchparams",
                "appendsearchparams",
                "withqs",
            ]
            .contains(&lower.as_str())
            {
                let (x, y) = first_arg(&Some(*args))?;
                let v = self.ev(f, x, y, depth + 1);
                return Some(if all_unknown(&v) {
                    wild_from(root, source_of(&v).max(self.src(f, x, y)))
                } else {
                    v
                });
            }
            // `useMemo(() => value, deps)`, `useCallback(fn, deps)`: the function's
            // results.
            if matches!(root.as_str(), "useMemo")
                && let Some((x, y)) = first_arg(&Some(*args))
                && let Some(fi) = f.fn_at(x, y)
            {
                return Some(self.fn_results(f, fi, depth + 1));
            }
            let call_args = f.split_commas(args.0, args.1);
            match f.lookup(root, a) {
                Some(Decl::Func(fi)) => return Some(self.call(f, call_args, f, fi, depth)),
                Some(Decl::Import(_)) => {
                    if let Some((g, n)) = self.files.imported_decl(f, root)
                        && let Some(fi) = g.module_fn(&n)
                    {
                        return Some(self.call(f, call_args, &g, fi, depth));
                    }
                }
                _ => {}
            }
            if let Some(rs) = f.fns.get(root) {
                return Some(self.results(f, rs, depth));
            }
            if let Some((g, n)) = self.files.imported_decl(f, root)
                && let Some(rs) = g.fns.get(&n)
            {
                return Some(self.results(&g, rs, depth));
            }
            return None;
        }
        // `this.x(...)` inside a class.
        if root == "this" {
            let cls = f.enclosing_class(a).map(|c| c.name.clone())?;
            let m = &names[1].name;
            if names.len() == 2 {
                return self.class_method(f, &cls, m, depth);
            }
            // `this.page.goto` etc. are not values.
            return None;
        }
        if root_args.is_none() {
            let plain = names[1..].iter().all(|l| l.args.is_none());
            let call_last = names[1..names.len() - 1].iter().all(|l| l.args.is_none())
                && names.last().is_some_and(|l| l.args.is_some());
            let path: Vec<String> = names[1..].iter().map(|l| l.name.clone()).collect();
            match f.lookup(root, a) {
                Some(Decl::Param { func, param }) if plain => {
                    return Some(self.param_value(f, func, param, &path, depth));
                }
                Some(Decl::Binding(bi)) => {
                    let bnd = &f.bindings[root.as_str()][bi];
                    let is_obj = bnd.init.is_some_and(|(x, _)| f.kind(x) == TokKind::LBrace);
                    if plain && (is_obj || !bnd.path.is_empty()) {
                        return Some(self.ident_field(f, a, root, &path, depth));
                    }
                    if call_last
                        && is_obj
                        && let Some((x, _)) = bnd.init
                        && let Some(fi) = self.object_fn(f, x, &path)
                        && let Some((p, q)) = names.last().and_then(|l| l.args)
                    {
                        return Some(self.call(f, f.split_commas(p, q), f, fi, depth));
                    }
                }
                Some(Decl::Import(ii)) => {
                    let ns = f.imports[ii].imported == Imported::Namespace;
                    if !ns
                        && let Some((g, n)) = self.files.imported_decl(f, root)
                        && let Some(bnd) = g.module_binding(&n)
                        && let Some((x, _)) = bnd.init
                        && g.kind(x) == TokKind::LBrace
                    {
                        if plain {
                            return Some(self.ident_field(f, a, root, &path, depth));
                        }
                        if call_last
                            && let Some(fi) = self.object_fn(&g, x, &path)
                            && let Some((p, q)) = names.last().and_then(|l| l.args)
                        {
                            return Some(self.call(f, f.split_commas(p, q), &g, fi, depth));
                        }
                    }
                    if ns && names.len() == 2 {
                        let m = &names[1];
                        if let Some(g) = self.files.imported_namespace(f, root)
                            && let Some((h, n)) = self.files.resolve_export(&g, &m.name)
                        {
                            match m.args {
                                Some((p, q)) => {
                                    if let Some(fi) = h.module_fn(&n) {
                                        return Some(self.call(
                                            f,
                                            f.split_commas(p, q),
                                            &h,
                                            fi,
                                            depth,
                                        ));
                                    }
                                }
                                None => return Some(self.ident_field(f, a, root, &path, depth)),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // `obj.method(...)` on a page object.
        if names.len() == 2
            && let Some(cls) = f.var_class.get(root)
        {
            let m = &names[1].name;
            return self.class_method(f, cls, m, depth);
        }
        // `X.method()` on a class (static) or a namespace import.
        if names.len() == 2 {
            let m = &names[1].name;
            if let Some(v) = self.class_method(f, root, m, depth) {
                return Some(v);
            }
            if let Some(imp) = f.import(root)
                && imp.imported == Imported::Namespace
                && let Some(p) = self.files.resolve(&f.path, &imp.module)
                && let Some(g) = self.files.get(&p)
            {
                if let Some(rs) = g.fns.get(m) {
                    return Some(self.results(&g, rs, depth));
                }
                if let Some(&(x, y)) = g.consts.get(m) {
                    return Some(self.ev(&g, x, y, depth + 1));
                }
            }
        }
        None
    }

    /// The function at the property path `path` of the object literal starting
    /// at `x`.
    pub fn object_fn(&self, f: &JsFile, x: usize, path: &[String]) -> Option<usize> {
        let mut open = x;
        for (i, key) in path.iter().enumerate() {
            if f.kind(open) != TokKind::LBrace {
                return None;
            }
            let last = i + 1 == path.len();
            match obj_prop(f, open, key).0? {
                Prop::Method(fi) if last => return Some(fi),
                Prop::Value(p, q) if last => return f.fn_at(p, q),
                Prop::Value(p, _) => open = p,
                _ => return None,
            }
        }
        None
    }
}

/// Converts an evaluator value to alternatives.
pub fn value_tpls(v: &turborepo_trace_core::eval::Value) -> Vec<Tpl> {
    use turborepo_trace_core::eval::{PatPart, Value};
    match v {
        Value::Str(s) => vec![vec![Piece::Lit(s.clone())]],
        Value::Pattern(ps) => vec![
            ps.iter()
                .map(|p| match p {
                    PatPart::Lit(s) => Piece::Lit(s.clone()),
                    PatPart::Wild { reason } => Piece::wild(reason),
                })
                .collect(),
        ],
        Value::Alternatives(vs) => cap(vs.iter().flat_map(value_tpls).collect()),
        Value::Unknown { reason } => wild(reason),
    }
}

/// A call site: `callee(args)` with the callee resolved to a module path.
#[derive(Clone, Debug)]
pub struct CallSite {
    /// `Some(module)` for imported callees, `None` for globals.
    pub module: Option<String>,
    /// Dotted path (`default.get`, `useRouter().push`, `fetch`,
    /// `navigator.sendBeacon`).
    pub path: String,
    /// `new` call.
    pub is_new: bool,
    /// Argument ranges.
    pub args: Vec<(usize, usize)>,
    /// Token index of the callee start.
    pub at: usize,
    /// Receiver identifier (`page` in `page.goto`), if any.
    pub receiver: Option<String>,
    /// The method name (last chain element).
    pub method: String,
    /// Token of the `(`.
    pub open: usize,
}

/// Every call site in `f`.
pub fn call_sites(f: &JsFile) -> Vec<CallSite> {
    let n = f.toks.len();
    let mut out = Vec::new();
    for i in 0..n {
        if f.kind(i) != TokKind::LParen || i == 0 {
            continue;
        }
        // Walk back over `ident(.ident)*` and type args.
        let mut j = i - 1;
        // `>`, or `>>` / `>>>` closing nested type arguments.
        let closers = |k: usize| -> i32 {
            match f.kind(k) {
                TokKind::Gt => 1,
                TokKind::Op => {
                    let t = f.text(k);
                    if !t.is_empty() && t.bytes().all(|c| c == b'>') {
                        t.len() as i32
                    } else {
                        0
                    }
                }
                _ => 0,
            }
        };
        if closers(j) > 0 {
            let mut d = 0;
            // Brackets inside the type arguments (`<{ a: T[] } | null>`), walking back.
            let mut nest = 0;
            let mut steps = 0;
            loop {
                steps += 1;
                match f.kind(j) {
                    TokKind::Gt | TokKind::Op if nest == 0 && closers(j) > 0 => d += closers(j),
                    TokKind::Lt if nest == 0 => {
                        d -= 1;
                        if d == 0 {
                            break;
                        }
                    }
                    TokKind::RBrace | TokKind::RParen | TokKind::RBracket => nest += 1,
                    TokKind::LBrace | TokKind::LParen | TokKind::LBracket => {
                        if nest == 0 {
                            break;
                        }
                        nest -= 1;
                    }
                    TokKind::Semi if nest == 0 => break,
                    _ => {}
                }
                if j == 0 || steps > 400 {
                    break;
                }
                j -= 1;
            }
            if f.kind(j) != TokKind::Lt || j == 0 {
                continue;
            }
            j -= 1;
        }
        if f.kind(j) != TokKind::Ident {
            continue;
        }
        let end = j;
        let mut start = j;
        while start >= 2
            && matches!(f.kind(start - 1), TokKind::Dot | TokKind::OptChain)
            && f.kind(start - 2) == TokKind::Ident
        {
            start -= 2;
        }
        // A call result receiver (`useRouter().push(`) is not followed here.
        if start >= 1 && matches!(f.kind(start - 1), TokKind::Dot | TokKind::OptChain) {
            // `x().push(` or `foo.bar().baz(`: record with an unknown receiver.
            let method = f.text(end).to_string();
            out.push(CallSite {
                module: None,
                path: format!("?.{method}"),
                is_new: false,
                args: f
                    .closer(i)
                    .map(|c| f.split_commas(i + 1, c))
                    .unwrap_or_default(),
                at: start,
                receiver: None,
                method,
                open: i,
            });
            continue;
        }
        let mut chain: Vec<String> = Vec::new();
        let mut k = start;
        while k <= end {
            chain.push(f.text(k).to_string());
            k += 2;
        }
        let is_new = start >= 1 && f.kw(start - 1) == Kw::New;
        // Declarations are not calls.
        if start >= 1
            && (f.kw(start - 1) == Kw::Function
                || f.kind(start - 1) == TokKind::Ident
                    && f.text(start - 1) == "async"
                    && chain.len() == 1
                    && f.toks[start].depth == 0)
        {
            continue;
        }
        let Some(c) = f.closer(i) else { continue };
        let args = f.split_commas(i + 1, c);
        let method = chain.last().cloned().unwrap_or_default();
        let receiver = (chain.len() >= 2).then(|| chain[chain.len() - 2].clone());
        let (module, path) = resolve_chain(f, &chain);
        out.push(CallSite {
            module,
            path,
            is_new,
            args,
            at: start,
            receiver,
            method,
            open: i,
        });
    }
    out
}

/// Resolves `a.b.c` to `(module, path)` through imports and call bindings;
/// globals drop a `window.` / `globalThis.` / `self.` prefix.
pub fn resolve_chain(f: &JsFile, chain: &[String]) -> (Option<String>, String) {
    let join = |base: &str, rest: &[String]| {
        let mut p = base.to_string();
        for r in rest {
            if !p.is_empty() {
                p.push('.');
            }
            p.push_str(r);
        }
        p
    };
    if let Some(imp) = f.import(&chain[0]) {
        let base = match &imp.imported {
            Imported::Default => "default".to_string(),
            Imported::Named(n) => n.clone(),
            Imported::Namespace => String::new(),
        };
        return (Some(imp.module.clone()), join(&base, &chain[1..]));
    }
    if let Some((m, p)) = f.call_bindings.get(&chain[0]) {
        return (Some(m.clone()), join(p, &chain[1..]));
    }
    let mut c = chain;
    while c.len() > 1 && matches!(c[0].as_str(), "window" | "globalThis" | "self" | "global") {
        c = &c[1..];
    }
    (None, join(&c[0], &c[1..]))
}
