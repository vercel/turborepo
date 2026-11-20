//! Module shapes: what a file exports, which names it uses from each module it
//! imports, and whether its top level is free of side effects. Input for
//! export-level tracing through barrel files in `turbo trace affected`
//! (`turborepo_trace_graph::barrel`).
//!
//! [`scan_shape`] lexes the file once (the lexer of [`crate::lexer`]) and makes
//! two linear passes over the tokens:
//!
//! 1. **Module syntax.** Top-level `import` / `export` statements,
//!    `require('x')` calls and test-runner mock calls:
//!    - [`Shape::uses`]: per static import, TS `import x = require()` and
//!      `require('x')` call, the names used from the module ([`Names`]), keyed
//!      by the byte offset where the recognizer's reference span starts (the
//!      `import` / `require` keyword) and the specifier. `import {a, b as c}`
//!      uses `a` and `b`, a default import uses `default`, `import 'x'` uses
//!      nothing, `import * as ns` uses the static members accessed on `ns`
//!      (`ns.a`) or everything when `ns` is used any other way (or the file has
//!      JSX, whose tag names the lexer skips). `const {a, b: c} = require('x')`
//!      uses `a` and `b`, `require('x').a` uses `a`, any other `require('x')`
//!      everything. Dynamic `import()`, `require.resolve()` and every other
//!      reference are not listed: callers treat references without an entry as
//!      using everything.
//!    - [`Shape::reexports`]: `export * from`, `export * as ns from`, `export
//!      {a as b} from`, also keyed by the start of the `export` keyword.
//!    - [`Shape::exports`]: names declared by the file itself, types included
//!      (`export const a`, `export function f`, `export interface I`, `export
//!      {a as b}`, `export default` as `default`). [`Shape::exports_open`] is
//!      set when the export set is not fully known from the file itself:
//!      `export * from`, CommonJS `module.exports` / `exports.x`, `export =`,
//!      destructuring exports, unrecognized forms.
//!    - [`Shape::mocked`]: specifiers passed to `jest.mock`, `vi.mock`,
//!      `jest.requireActual` and friends. Uses of a mocked specifier use
//!      everything; a mock call whose target is not a literal sets
//!      [`Shape::uses_all`].
//! 2. **Side effects.** [`Shape::side_effect_free`] holds when every top-level
//!    statement is one of:
//!    - an import declaration, a re-export or a local export list (they load
//!      modules, which callers compare between versions with
//!      [`Shape::executes`]);
//!    - a function declaration or overload signature;
//!    - `const` / `let` / `var` with identifier bindings whose initializers are
//!      *pure*: literals, substitution-free templates or templates over
//!      primitive constants, `true` / `false` / `null` / `undefined`, arrow
//!      functions and function or class expressions (bodies are not evaluated),
//!      array and object literals of pure values without spreads or computed
//!      keys (methods and accessors allowed), references to earlier top-level
//!      declarations or (hoisted) function declarations, arithmetic and
//!      comparisons between primitives, `!`, `void`, `typeof`, `&&` / `||` /
//!      `??`, ternaries and TS `as` / `satisfies` / non-null `!`;
//!    - a class declaration without decorators, static blocks, computed keys or
//!      index signatures, whose static field initializers are pure and whose
//!      `extends` names an earlier local class or function or a built-in
//!      constructor (`Error`, `Map`, ...);
//!    - an `enum` with literal initializers;
//!    - type-only constructs: `type`, `interface`, `declare` (except `declare
//!      global`, `declare module` and namespaces), `import type`, `export
//!      type`;
//!    - `export default` of a function, class or pure expression; directives;
//!      `;`.
//!    - Anything else (calls, `new`, member access, tagged templates,
//!      assignments, JSX, top-level `await`, decorators, namespaces, global
//!      augmentations, imported bindings used as values) clears the flag. The
//!      checker is deliberately incomplete: when in doubt, the file is not
//!      side-effect free.
//!
//! Lexer errors and failed heuristic JSX make the whole shape unknown
//! ([`Shape::understood`] false). Linear time; never panics.

use std::collections::{HashMap, HashSet};

use crate::lexer::{Dialect, ESCAPES, Jsx, Kw, Lexer, NEWLINE_BEFORE, TokKind, Token};

/// Names used from a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Names<'s> {
    /// Everything (namespace use, a plain `require`, `import x = require()`, a
    /// mock).
    All,
    /// These export names, sorted and deduplicated (`default` for the default
    /// export).
    Some(Vec<&'s str>),
}

/// One import of a module: a static import declaration, TS `import x =
/// require('x')` or a `require('x')` call with a literal argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use<'s> {
    /// Byte offset of the `import` / `require` keyword: the start of the
    /// recognizer's reference span.
    pub start: u32,
    /// The specifier as written (escape-free literals only).
    pub spec: &'s str,
    /// The names used.
    pub names: Names<'s>,
    /// `import type` (also TS `import type x = require()`).
    pub type_only: bool,
    /// `import * as ns`: the names are the members read from the module object.
    /// Unlike the names of `import {a, b}`, they are not asked of the
    /// module when the modules are linked: a member that is not there is
    /// `undefined`, a named import that is not there is an error.
    pub namespace: bool,
}

/// What a re-export forwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReExportKind<'s> {
    /// `export * from 'x'`: every name of `x` except `default`.
    Star,
    /// `export * as ns from 'x'`: the module object of `x` as `ns`.
    Namespace(&'s str),
    /// `export {a, b as c} from 'x'`: `(name in x, exported name)` pairs.
    Named(Vec<(&'s str, &'s str)>),
}

/// One `export ... from` statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReExport<'s> {
    /// Byte offset of the `export` keyword (start of the recognizer's reference
    /// span).
    pub start: u32,
    /// The specifier.
    pub spec: &'s str,
    /// What is forwarded.
    pub kind: ReExportKind<'s>,
    /// `export type ... from`.
    pub type_only: bool,
}

/// The module shape of one file. See the module documentation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape<'s> {
    /// False when the lexer reported an error (or heuristic JSX failed on both
    /// readings): nothing else in the shape may be relied on.
    pub understood: bool,
    /// The file has `import` or `export` statements (an ES module, not a
    /// script).
    pub module_syntax: bool,
    /// Imports and requires with the names they use, in source order.
    pub uses: Vec<Use<'s>>,
    /// Some module reference could not be attributed (a mock call with a
    /// computed target): every reference of the file uses everything.
    pub uses_all: bool,
    /// Re-exports, in source order.
    pub reexports: Vec<ReExport<'s>>,
    /// Names the file exports itself (not through `export ... from`), sorted,
    /// deduplicated.
    pub exports: Vec<&'s str>,
    /// The export set is not fully known from [`Shape::exports`] and
    /// [`Shape::reexports`].
    pub exports_open: bool,
    /// Specifiers of the runtime module loads of the top level, in order:
    /// non-type import declarations, re-exports and TS `import x =
    /// require()`.
    pub executes: Vec<&'s str>,
    /// Specifiers passed to test-runner mock calls (`jest.mock`, `vi.mock`,
    /// ...).
    pub mocked: Vec<&'s str>,
    /// The top level has no side effects (see the module documentation).
    pub side_effect_free: bool,
}

/// Mock-like calls of jest (`jest.x`) and vitest (`vi.x`) whose first argument
/// is a module.
const MOCK_FNS: &[&str] = &[
    "mock",
    "doMock",
    "setMock",
    "unmock",
    "dontMock",
    "deepUnmock",
    "unstable_mockModule",
    "requireActual",
    "requireMock",
    "importActual",
    "importMock",
    "createMockFromModule",
    "genMockFromModule",
];

/// Built-in constructors a side-effect-free class may extend.
const SAFE_BASES: &[&str] = &[
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
    "AggregateError",
    "Array",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Promise",
    "Object",
    "Function",
    "RegExp",
    "Date",
];

/// Class member modifiers.
const MODIFIERS: &[&str] = &[
    "static",
    "public",
    "private",
    "protected",
    "readonly",
    "abstract",
    "override",
    "declare",
    "async",
    "get",
    "set",
    "accessor",
];

const NONE: u32 = u32::MAX;

/// Scans the module shape of `src`. Linear time; never panics.
pub fn scan_shape(src: &[u8], dialect: Dialect) -> Shape<'_> {
    let mut lx = Lexer::new(src, dialect);
    let mut toks = lex_all(&mut lx);
    let mut dialect = dialect;
    let mut error = lx.take_error().is_some();
    if lx.jsx_failed() {
        dialect = Dialect {
            jsx: Jsx::Off,
            ..dialect
        };
        let mut lx = Lexer::new(src, dialect);
        toks = lex_all(&mut lx);
        error = lx.take_error().is_some() || lx.jsx_failed();
    }
    let Some(mate) = (!error).then(|| mates(&toks)).flatten() else {
        return Shape::default();
    };
    let has_jsx = dialect.jsx == Jsx::On || toks.iter().any(|t| t.kind == TokKind::Jsx);
    let mut s = Scanner {
        src,
        ts: dialect.ts,
        t: toks,
        mate,
        shape: Shape {
            understood: true,
            ..Shape::default()
        },
        namespaces: Vec::new(),
        has_jsx,
        locals: HashSet::new(),
        prims: HashSet::new(),
        fns: HashSet::new(),
        pending: Vec::new(),
        exporting: false,
        decl_exports: Vec::new(),
        nesting: 0,
    };
    s.module_pass();
    s.shape.side_effect_free = s.sef_pass();
    s.finish();
    s.shape
}

/// Maximum nesting of pure expressions the side-effect checker follows.
const MAX_NESTING: u32 = 256;

fn lex_all(lx: &mut Lexer<'_>) -> Vec<Token> {
    let mut v = Vec::new();
    loop {
        let t = lx.next_token();
        match t.kind {
            TokKind::TripleSlash => continue,
            TokKind::Eof => {
                v.push(t);
                return v;
            }
            _ => v.push(t),
        }
    }
}

/// Matching delimiter indexes (`NONE` for other tokens), or `None` if
/// unbalanced.
fn mates(t: &[Token]) -> Option<Vec<u32>> {
    let mut mate = vec![NONE; t.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, tok) in t.iter().enumerate() {
        let open = match tok.kind {
            TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                stack.push(i);
                continue;
            }
            TokKind::RParen => TokKind::LParen,
            TokKind::RBracket => TokKind::LBracket,
            TokKind::RBrace => TokKind::LBrace,
            TokKind::TemplateTail => TokKind::TemplateHead,
            TokKind::TemplateMiddle => {
                if t[*stack.last()?].kind != TokKind::TemplateHead {
                    return None;
                }
                continue;
            }
            _ => continue,
        };
        let o = stack.pop()?;
        if t[o].kind != open {
            return None;
        }
        mate[o] = i as u32;
        mate[i] = o as u32;
    }
    stack.is_empty().then_some(mate)
}

struct Scanner<'s> {
    src: &'s [u8],
    ts: bool,
    t: Vec<Token>,
    mate: Vec<u32>,
    shape: Shape<'s>,
    /// `import * as ns` bindings: (use index, binding token index).
    namespaces: Vec<(usize, usize)>,
    has_jsx: bool,
    // Side-effect pass state.
    /// Top-level runtime bindings declared so far.
    locals: HashSet<&'s str>,
    /// Locals bound to primitive values.
    prims: HashSet<&'s str>,
    /// Top-level function declarations (hoisted).
    fns: HashSet<&'s str>,
    /// Names referenced before their declaration: must turn out to be function
    /// declarations.
    pending: Vec<&'s str>,
    /// Inside `export <declaration>`.
    exporting: bool,
    /// Names of exported `const` / `let` / `var` declarators, as the
    /// side-effect pass parsed them (cross-checked against
    /// [`Shape::exports`]).
    decl_exports: Vec<&'s str>,
    /// Current pure-expression nesting.
    nesting: u32,
}

/// Tokens that can continue an expression on the next line (no automatic
/// semicolon).
fn continues(t: &Token, text: &[u8]) -> bool {
    match t.kind {
        TokKind::LParen
        | TokKind::LBracket
        | TokKind::Dot
        | TokKind::OptChain
        | TokKind::Template
        | TokKind::TemplateHead
        | TokKind::Op
        | TokKind::Lt
        | TokKind::Gt
        | TokKind::Star
        | TokKind::Assign
        | TokKind::Question
        | TokKind::Arrow
        | TokKind::Comma
        | TokKind::Colon
        | TokKind::Bang => true,
        TokKind::Ident => matches!(text, b"in" | b"instanceof" | b"as" | b"satisfies"),
        _ => false,
    }
}

/// Tokens that end an expression (for the newline rule of
/// [`Scanner::skip_expr`]).
fn ends_expr(t: &Token) -> bool {
    match t.kind {
        TokKind::Ident => !t.kw.expr_follows(),
        TokKind::PrivateName
        | TokKind::Str
        | TokKind::Template
        | TokKind::TemplateTail
        | TokKind::Regex
        | TokKind::Number
        | TokKind::Jsx
        | TokKind::RParen
        | TokKind::RBracket
        | TokKind::RBrace
        | TokKind::IncDec => true,
        _ => false,
    }
}

impl<'s> Scanner<'s> {
    #[inline]
    fn tok(&self, i: usize) -> Token {
        // The last token is always Eof.
        self.t[i.min(self.t.len() - 1)]
    }

    #[inline]
    fn kind(&self, i: usize) -> TokKind {
        self.tok(i).kind
    }

    #[inline]
    fn text(&self, i: usize) -> &'s [u8] {
        let t = self.tok(i);
        self.src
            .get(t.start as usize..t.end as usize)
            .unwrap_or_default()
    }

    #[inline]
    fn is(&self, i: usize, word: &[u8]) -> bool {
        self.kind(i) == TokKind::Ident && self.text(i) == word
    }

    #[inline]
    fn nl(&self, i: usize) -> bool {
        self.tok(i).flags & NEWLINE_BEFORE != 0
    }

    /// Index after the group opened at `i` (its closer + 1).
    #[inline]
    fn after_group(&self, i: usize) -> Option<usize> {
        let m = *self.mate.get(i)?;
        (m != NONE && m as usize > i).then_some(m as usize + 1)
    }

    fn ident_str(&self, i: usize) -> Option<&'s str> {
        if self.kind(i) != TokKind::Ident {
            return None;
        }
        std::str::from_utf8(self.text(i)).ok()
    }

    /// An escape-free string literal or substitution-free template.
    fn literal(&self, i: usize) -> Option<&'s str> {
        let t = self.tok(i);
        if t.flags & ESCAPES != 0 {
            return None;
        }
        std::str::from_utf8(t.literal_content(self.src)?).ok()
    }

    fn is_literal_tok(&self, i: usize) -> bool {
        matches!(self.kind(i), TokKind::Str | TokKind::Template)
    }

    /// A module export name: identifier or (escape-free) string.
    fn name(&self, i: usize) -> Option<&'s str> {
        match self.kind(i) {
            TokKind::Ident => self.ident_str(i),
            TokKind::Str => self.literal(i),
            _ => None,
        }
    }

    // ---------------------------------------------------------------------------------
    // Pass 1: module syntax
    // ---------------------------------------------------------------------------------

    fn module_pass(&mut self) {
        let n = self.t.len();
        let mut i = 0;
        while i < n {
            let t = self.t[i];
            if t.kind != TokKind::Ident {
                i += 1;
                continue;
            }
            let after_dot = i > 0 && matches!(self.t[i - 1].kind, TokKind::Dot | TokKind::OptChain);
            match t.kw {
                Kw::Import if t.depth == 0 => {
                    i = self.import_decl(i);
                    continue;
                }
                Kw::Export if t.depth == 0 => {
                    i = self.export_decl(i);
                    continue;
                }
                Kw::Require
                    if !after_dot
                        && self.kind(i + 1) == TokKind::LParen
                        && self.is_literal_tok(i + 2)
                        && self.kind(i + 3) == TokKind::RParen =>
                {
                    self.require_call(i);
                }
                Kw::Module
                    if !after_dot
                        && self.kind(i + 1) == TokKind::Dot
                        && self.is(i + 2, b"exports") =>
                {
                    self.shape.exports_open = true;
                }
                _ => {}
            }
            if !after_dot {
                let w = self.text(i);
                if w == b"exports" {
                    self.shape.exports_open = true;
                } else if (w == b"jest" || w == b"vi") && self.kind(i + 1) == TokKind::Dot {
                    self.mock_call(i);
                }
            }
            i += 1;
        }
    }

    fn record_use(&mut self, start: u32, spec: &'s str, names: Names<'s>, type_only: bool) {
        self.shape.uses.push(Use {
            start,
            spec,
            names,
            type_only,
            namespace: false,
        });
    }

    /// `import ...` at the top level; returns the index to continue at.
    fn import_decl(&mut self, i: usize) -> usize {
        let start = self.t[i].start;
        let mut j = i + 1;
        match self.kind(j) {
            // Dynamic import / import.meta: not a declaration.
            TokKind::LParen | TokKind::Dot => return i + 1,
            _ => {}
        }
        self.shape.module_syntax = true;
        if self.is_literal_tok(j) {
            if let Some(spec) = self.literal(j) {
                self.record_use(start, spec, Names::Some(Vec::new()), false);
                self.shape.executes.push(spec);
            }
            return j + 1;
        }
        let mut type_only = false;
        if (self.is(j, b"type") || self.is(j, b"typeof"))
            && !(self.tok(j + 1).kw == Kw::From && self.is_literal_tok(j + 2))
            && self.kind(j + 1) != TokKind::Comma
            && self.kind(j + 1) != TokKind::Assign
        {
            type_only = true;
            j += 1;
        }
        let mut all = false;
        if (self.is(j, b"defer") || self.is(j, b"source"))
            && matches!(self.kind(j + 1), TokKind::Star | TokKind::Ident)
            && self.tok(j + 1).kw != Kw::From
        {
            all = true;
            j += 1;
        }
        let mut names: Vec<&'s str> = Vec::new();
        let mut ns: Option<usize> = None;
        if self.kind(j) == TokKind::Ident
            && !(self.tok(j).kw == Kw::From && self.is_literal_tok(j + 1))
        {
            // Default binding, or TS `import x = ...`.
            if self.kind(j + 1) == TokKind::Assign {
                return self.import_equals(i, j, type_only);
            }
            names.push("default");
            j += 1;
            if self.kind(j) == TokKind::Comma {
                j += 1;
            }
        }
        match self.kind(j) {
            TokKind::Star => {
                if self.tok(j + 1).kw != Kw::As || self.kind(j + 2) != TokKind::Ident {
                    return j;
                }
                ns = Some(j + 2);
                j += 3;
            }
            TokKind::LBrace => {
                let Some(end) = self.after_group(j) else {
                    return j + 1;
                };
                let close = end - 1;
                let mut k = j + 1;
                while k < close {
                    let first = k;
                    while k < close && self.kind(k) != TokKind::Comma {
                        k += 1;
                    }
                    // Specifier tokens first..k: [type] name [as local].
                    let len = k - first;
                    let mut a = first;
                    if len >= 2
                        && self.is(first, b"type")
                        && !(len == 3 && self.tok(first + 1).kw == Kw::As)
                    {
                        a += 1;
                    }
                    if a < k {
                        match self.name(a) {
                            Some(n) => names.push(n),
                            None => all = true,
                        }
                    }
                    k += 1;
                }
                j = end;
            }
            _ => {}
        }
        if self.tok(j).kw != Kw::From || !self.is_literal_tok(j + 1) {
            return j.max(i + 1);
        }
        let Some(spec) = self.literal(j + 1) else {
            return j + 2;
        };
        let names = if all {
            Names::All
        } else if let Some(b) = ns {
            self.namespaces.push((self.shape.uses.len(), b));
            Names::Some(names)
        } else {
            Names::Some(names)
        };
        self.record_use(start, spec, names, type_only);
        if ns.is_some()
            && let Some(u) = self.shape.uses.last_mut()
        {
            u.namespace = true;
        }
        if !type_only {
            self.shape.executes.push(spec);
        }
        j + 2
    }

    /// TS `import x = require('y')` (`j` is the index of `x`).
    fn import_equals(&mut self, i: usize, j: usize, type_only: bool) -> usize {
        if self.tok(j + 2).kw == Kw::Require
            && self.kind(j + 3) == TokKind::LParen
            && self.is_literal_tok(j + 4)
            && self.kind(j + 5) == TokKind::RParen
        {
            if let Some(spec) = self.literal(j + 4) {
                self.record_use(self.t[i].start, spec, Names::All, type_only);
                if !type_only {
                    self.shape.executes.push(spec);
                }
            }
            return j + 6;
        }
        j + 2
    }

    /// `require('x')` at `i` (`(` literal `)` follow).
    fn require_call(&mut self, i: usize) {
        let Some(spec) = self.literal(i + 2) else {
            return;
        };
        let after = i + 4;
        let names = if matches!(self.kind(after), TokKind::Dot | TokKind::OptChain)
            && self.kind(after + 1) == TokKind::Ident
        {
            self.ident_str(after + 1)
                .map(|n| Names::Some(vec![n]))
                .unwrap_or(Names::All)
        } else if i >= 2
            && self.kind(i - 1) == TokKind::Assign
            && self.kind(i - 2) == TokKind::RBrace
            && !matches!(
                self.kind(after),
                TokKind::Dot | TokKind::OptChain | TokKind::LBracket | TokKind::LParen
            )
        {
            let open = self.mate[i - 2];
            if open == NONE {
                Names::All
            } else {
                self.pattern_names(open as usize, i - 2)
            }
        } else {
            Names::All
        };
        self.record_use(self.t[i].start, spec, names, false);
    }

    /// Property names read by an object destructuring pattern `{ ... }`
    /// (`open..close`).
    fn pattern_names(&self, open: usize, close: usize) -> Names<'s> {
        let mut out = Vec::new();
        let mut k = open + 1;
        while k < close {
            match self.kind(k) {
                TokKind::Ellipsis | TokKind::LBracket => return Names::All,
                TokKind::Ident | TokKind::Str | TokKind::Number => match self.name(k) {
                    Some(n) => out.push(n),
                    None if self.kind(k) == TokKind::Number => {
                        match std::str::from_utf8(self.text(k)) {
                            Ok(n) => out.push(n),
                            Err(_) => return Names::All,
                        }
                    }
                    None => return Names::All,
                },
                _ => return Names::All,
            }
            // Skip `: value` / `= default` up to the next comma at this level.
            k += 1;
            while k < close && self.kind(k) != TokKind::Comma {
                k = match self.kind(k) {
                    TokKind::LParen
                    | TokKind::LBracket
                    | TokKind::LBrace
                    | TokKind::TemplateHead => match self.after_group(k) {
                        Some(e) => e,
                        None => return Names::All,
                    },
                    _ => k + 1,
                };
            }
            k += 1;
        }
        Names::Some(out)
    }

    /// `jest.x(` / `vi.x(` at `i`.
    fn mock_call(&mut self, i: usize) {
        let Some(f) = self.ident_str(i + 2) else {
            return;
        };
        if !MOCK_FNS.contains(&f) || self.kind(i + 3) != TokKind::LParen {
            return;
        }
        let a = i + 4;
        if self.is_literal_tok(a) && matches!(self.kind(a + 1), TokKind::Comma | TokKind::RParen) {
            match self.literal(a) {
                Some(s) => self.shape.mocked.push(s),
                None => self.shape.uses_all = true,
            }
        } else if self.tok(a).kw == Kw::Import
            && self.kind(a + 1) == TokKind::LParen
            && self.is_literal_tok(a + 2)
            && self.kind(a + 3) == TokKind::RParen
        {
            // `vi.mock(import('./a'))`.
            match self.literal(a + 2) {
                Some(s) => self.shape.mocked.push(s),
                None => self.shape.uses_all = true,
            }
        } else {
            self.shape.uses_all = true;
        }
    }

    /// `export ...` at the top level; returns the index to continue at.
    fn export_decl(&mut self, i: usize) -> usize {
        self.shape.module_syntax = true;
        let start = self.t[i].start;
        let mut j = i + 1;
        let mut type_only = false;
        if self.tok(j).kw == Kw::Type && matches!(self.kind(j + 1), TokKind::LBrace | TokKind::Star)
        {
            type_only = true;
            j += 1;
        }
        match self.kind(j) {
            TokKind::Star => {
                let mut kind = ReExportKind::Star;
                let mut k = j + 1;
                if self.tok(k).kw == Kw::As {
                    match self.name(k + 1) {
                        Some(n) => kind = ReExportKind::Namespace(n),
                        None => {
                            self.shape.exports_open = true;
                            return k + 1;
                        }
                    }
                    k += 2;
                }
                if self.tok(k).kw == Kw::From && self.is_literal_tok(k + 1) {
                    match self.literal(k + 1) {
                        Some(spec) => {
                            if matches!(kind, ReExportKind::Star) {
                                self.shape.exports_open = true;
                            }
                            self.shape.reexports.push(ReExport {
                                start,
                                spec,
                                kind,
                                type_only,
                            });
                            if !type_only {
                                self.shape.executes.push(spec);
                            }
                        }
                        None => self.shape.exports_open = true,
                    }
                    return k + 2;
                }
                self.shape.exports_open = true;
                k
            }
            TokKind::LBrace => {
                let Some(end) = self.after_group(j) else {
                    self.shape.exports_open = true;
                    return j + 1;
                };
                let close = end - 1;
                let mut pairs: Vec<(&'s str, &'s str)> = Vec::new();
                let mut ok = true;
                let mut k = j + 1;
                while k < close {
                    let first = k;
                    while k < close && self.kind(k) != TokKind::Comma {
                        k += 1;
                    }
                    let mut a = first;
                    let len = k - first;
                    if len >= 2
                        && self.is(first, b"type")
                        && !(len == 3 && self.tok(first + 1).kw == Kw::As)
                    {
                        a += 1;
                    }
                    let n = k - a;
                    match n {
                        0 => {}
                        1 => match self.name(a) {
                            Some(x) => pairs.push((x, x)),
                            None => ok = false,
                        },
                        3 if self.tok(a + 1).kw == Kw::As => match (self.name(a), self.name(a + 2))
                        {
                            (Some(x), Some(y)) => pairs.push((x, y)),
                            _ => ok = false,
                        },
                        _ => ok = false,
                    }
                    k += 1;
                }
                if self.tok(end).kw == Kw::From && self.is_literal_tok(end + 1) {
                    match (ok, self.literal(end + 1)) {
                        (true, Some(spec)) => {
                            self.shape.reexports.push(ReExport {
                                start,
                                spec,
                                kind: ReExportKind::Named(pairs),
                                type_only,
                            });
                            if !type_only {
                                self.shape.executes.push(spec);
                            }
                        }
                        _ => self.shape.exports_open = true,
                    }
                    return end + 2;
                }
                if !ok {
                    self.shape.exports_open = true;
                }
                for (_, to) in pairs {
                    self.shape.exports.push(to);
                }
                end
            }
            TokKind::Assign => {
                self.shape.exports_open = true;
                j + 1
            }
            _ => {
                self.export_declaration(j);
                j
            }
        }
    }

    /// `export <declaration>` with the declaration at `j`.
    fn export_declaration(&mut self, mut j: usize) {
        if self.tok(j).kw == Kw::Default {
            self.shape.exports.push("default");
            return;
        }
        if self.is(j, b"declare") {
            j += 1;
        }
        let t = self.tok(j);
        let name_at = match t.kw {
            Kw::Const | Kw::Let | Kw::Var => {
                if t.kw == Kw::Const && self.is(j + 1, b"enum") {
                    j + 2
                } else {
                    self.export_vars(j + 1);
                    return;
                }
            }
            Kw::Function => {
                if self.kind(j + 1) == TokKind::Star {
                    j + 2
                } else {
                    j + 1
                }
            }
            Kw::Class | Kw::Type | Kw::Module => j + 1,
            // `export import A = B.C`
            Kw::Import => j + 1,
            _ => {
                let w = self.text(j);
                if w == b"async" && self.tok(j + 1).kw == Kw::Function {
                    if self.kind(j + 2) == TokKind::Star {
                        j + 3
                    } else {
                        j + 2
                    }
                } else if w == b"abstract" && self.tok(j + 1).kw == Kw::Class {
                    j + 2
                } else if matches!(w, b"enum" | b"interface" | b"namespace") {
                    j + 1
                } else {
                    self.shape.exports_open = true;
                    return;
                }
            }
        };
        match self.ident_str(name_at) {
            Some(n) => self.shape.exports.push(n),
            None => self.shape.exports_open = true,
        }
    }

    /// Names of `export const a = ..., b = ...` (starting at the first
    /// binding).
    fn export_vars(&mut self, mut k: usize) {
        loop {
            match self.ident_str(k) {
                Some(n) if self.kind(k) == TokKind::Ident => self.shape.exports.push(n),
                _ => {
                    self.shape.exports_open = true;
                    return;
                }
            }
            k += 1;
            // Skip the type annotation and initializer up to a comma at this level or the
            // end of the statement.
            match self.skip_expr_until_decl_end(k) {
                Some(e) if self.kind(e) == TokKind::Comma => k = e + 1,
                Some(_) => return,
                None => {
                    self.shape.exports_open = true;
                    return;
                }
            }
        }
    }

    /// From inside a declarator, the index of the `,` separating it from the
    /// next one, or of the token ending the statement.
    fn skip_expr_until_decl_end(&self, mut k: usize) -> Option<usize> {
        let mut angle = 0u32;
        let start = k;
        loop {
            let t = self.tok(k);
            match t.kind {
                TokKind::Eof
                | TokKind::Semi
                | TokKind::RParen
                | TokKind::RBracket
                | TokKind::RBrace => {
                    return Some(k);
                }
                TokKind::Comma if angle == 0 => return Some(k),
                TokKind::Lt => angle += 1,
                TokKind::Gt => angle = angle.saturating_sub(1),
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    k = self.after_group(k)?;
                    continue;
                }
                _ => {}
            }
            if k > start
                && t.flags & NEWLINE_BEFORE != 0
                && angle == 0
                && ends_expr(&self.tok(k - 1))
                && !continues(&t, self.text(k))
            {
                return Some(k);
            }
            k += 1;
        }
    }

    // ---------------------------------------------------------------------------------
    // Namespace member analysis, mocks, export bookkeeping
    // ---------------------------------------------------------------------------------

    fn finish(&mut self) {
        if !self.namespaces.is_empty() {
            self.namespace_members();
        }
        if !self.shape.mocked.is_empty() {
            let mocked: HashSet<&str> = self.shape.mocked.iter().copied().collect();
            for u in &mut self.shape.uses {
                if mocked.contains(u.spec) {
                    u.names = Names::All;
                }
            }
        }
        for u in &mut self.shape.uses {
            if let Names::Some(v) = &mut u.names {
                v.sort_unstable();
                v.dedup();
            }
        }
        self.shape.exports.sort_unstable();
        self.shape.exports.dedup();
        // Exported declarators as the exact parse saw them must all be listed.
        if self
            .decl_exports
            .iter()
            .any(|n| self.shape.exports.binary_search(n).is_err())
        {
            self.shape.exports_open = true;
        }
        self.shape.mocked.sort_unstable();
        self.shape.mocked.dedup();
    }

    /// Resolves `import * as ns` uses to the static members accessed on `ns`.
    fn namespace_members(&mut self) {
        let mut by_name: HashMap<&'s [u8], Vec<(usize, usize)>> = HashMap::new();
        for &(u, b) in &self.namespaces {
            by_name.entry(self.text(b)).or_default().push((u, b));
        }
        let mut all: HashSet<usize> = HashSet::new();
        let mut members: HashMap<usize, Vec<&'s str>> = HashMap::new();
        // A binding declared twice is a syntax error; keep the scan linear.
        by_name.retain(|_, list| {
            if list.len() > 1 {
                all.extend(list.iter().map(|&(u, _)| u));
                false
            } else {
                true
            }
        });
        if self.has_jsx {
            // JSX tag names (`<ns.Comp />`) are not tokens.
            all.extend(self.namespaces.iter().map(|&(u, _)| u));
        } else {
            for i in 0..self.t.len() {
                if self.t[i].kind != TokKind::Ident {
                    continue;
                }
                let Some(list) = by_name.get(self.text(i)) else {
                    continue;
                };
                if list.iter().any(|&(_, b)| b == i) {
                    continue;
                }
                if i > 0 && matches!(self.t[i - 1].kind, TokKind::Dot | TokKind::OptChain) {
                    continue; // a property of something else
                }
                let member = (matches!(self.kind(i + 1), TokKind::Dot | TokKind::OptChain)
                    && self.kind(i + 2) == TokKind::Ident)
                    .then(|| self.ident_str(i + 2))
                    .flatten();
                for &(u, _) in list {
                    match member {
                        Some(m) => members.entry(u).or_default().push(m),
                        None => {
                            all.insert(u);
                        }
                    }
                }
            }
        }
        for &(u, _) in &self.namespaces {
            let use_ = &mut self.shape.uses[u];
            if all.contains(&u) {
                use_.names = Names::All;
            } else if let Names::Some(v) = &mut use_.names {
                v.extend(members.remove(&u).unwrap_or_default());
            }
        }
    }

    // ---------------------------------------------------------------------------------
    // Pass 2: side effects
    // ---------------------------------------------------------------------------------

    fn sef_pass(&mut self) -> bool {
        let mut i = 0;
        let n = self.t.len();
        while i < n && self.t[i].kind != TokKind::Eof {
            match self.statement(i) {
                Some(j) if j > i => i = j,
                _ => return false,
            }
        }
        let fns = std::mem::take(&mut self.fns);
        let ok = self.pending.iter().all(|p| fns.contains(p));
        self.fns = fns;
        ok
    }

    /// End of a statement whose last token is before `j`.
    fn stmt_end(&self, j: usize) -> Option<usize> {
        match self.kind(j) {
            TokKind::Semi => Some(j + 1),
            TokKind::Eof => Some(j),
            _ if self.nl(j) && !continues(&self.tok(j), self.text(j)) => Some(j),
            _ => None,
        }
    }

    fn declare_local(&mut self, name: &'s str, prim: bool) -> Option<()> {
        if !self.locals.insert(name) {
            return None; // redeclaration
        }
        if prim {
            self.prims.insert(name);
        }
        Some(())
    }

    fn statement(&mut self, i: usize) -> Option<usize> {
        let t = self.tok(i);
        match t.kind {
            TokKind::Semi => Some(i + 1),
            // Directives ('use strict', 'use client').
            TokKind::Str => self.stmt_end(i + 1),
            TokKind::Ident => match t.kw {
                Kw::Import => self.import_stmt(i),
                Kw::Export => self.export_stmt(i),
                Kw::Const if self.is(i + 1, b"enum") => self.enum_decl(i + 2, true),
                Kw::Const | Kw::Let | Kw::Var => self.var_decl(i, true),
                Kw::Function => self.function(i, false, true),
                Kw::Class => self.class(i, false, true),
                Kw::Type if self.kind(i + 1) == TokKind::Ident && !self.nl(i + 1) => {
                    self.type_alias(i)
                }
                _ => {
                    let w = self.text(i);
                    match w {
                        b"async" if self.tok(i + 1).kw == Kw::Function && !self.nl(i + 1) => {
                            self.function(i + 1, false, true)
                        }
                        b"abstract" if self.tok(i + 1).kw == Kw::Class && !self.nl(i + 1) => {
                            self.class(i + 1, false, true)
                        }
                        b"interface" if self.kind(i + 1) == TokKind::Ident && !self.nl(i + 1) => {
                            self.interface(i)
                        }
                        b"enum" if self.kind(i + 1) == TokKind::Ident && !self.nl(i + 1) => {
                            self.enum_decl(i + 1, true)
                        }
                        b"declare" if !self.nl(i + 1) => self.declare(i + 1),
                        _ => None,
                    }
                }
            },
            _ => None,
        }
    }

    fn import_stmt(&mut self, i: usize) -> Option<usize> {
        let mut j = i + 1;
        if matches!(self.kind(j), TokKind::LParen | TokKind::Dot) {
            return None;
        }
        // TS `import x = require('y')` / `import type x = require('y')`.
        let eq = if self.is(j, b"type") { j + 2 } else { j + 1 };
        if self.kind(j) == TokKind::Ident && self.kind(eq) == TokKind::Assign {
            if self.tok(eq + 1).kw == Kw::Require
                && self.kind(eq + 2) == TokKind::LParen
                && self.is_literal_tok(eq + 3)
                && self.kind(eq + 4) == TokKind::RParen
            {
                return self.stmt_end(eq + 5);
            }
            return None;
        }
        loop {
            match self.kind(j) {
                TokKind::Str | TokKind::Template => break,
                TokKind::Ident | TokKind::Comma | TokKind::Star => j += 1,
                TokKind::LBrace => j = self.after_group(j)?,
                _ => return None,
            }
        }
        j += 1;
        j = self.attributes(j)?;
        self.stmt_end(j)
    }

    /// Skips import attributes (`with { type: 'json' }`, `assert {...}`).
    fn attributes(&self, j: usize) -> Option<usize> {
        if (self.is(j, b"with") || self.is(j, b"assert"))
            && !self.nl(j)
            && self.kind(j + 1) == TokKind::LBrace
        {
            return self.after_group(j + 1);
        }
        Some(j)
    }

    fn export_stmt(&mut self, i: usize) -> Option<usize> {
        let mut j = i + 1;
        if self.tok(j).kw == Kw::Type && matches!(self.kind(j + 1), TokKind::LBrace | TokKind::Star)
        {
            j += 1;
        }
        match self.kind(j) {
            TokKind::Star => {
                j += 1;
                if self.tok(j).kw == Kw::As {
                    j += 2;
                }
                if self.tok(j).kw != Kw::From || !self.is_literal_tok(j + 1) {
                    return None;
                }
                let k = self.attributes(j + 2)?;
                self.stmt_end(k)
            }
            TokKind::LBrace => {
                let e = self.after_group(j)?;
                if self.tok(e).kw == Kw::From {
                    if !self.is_literal_tok(e + 1) {
                        return None;
                    }
                    let k = self.attributes(e + 2)?;
                    return self.stmt_end(k);
                }
                self.stmt_end(e)
            }
            TokKind::Ident => {
                let t = self.tok(j);
                match t.kw {
                    Kw::Default => self.export_default(j + 1),
                    Kw::Import => None,
                    _ => {
                        let w = self.text(j);
                        if w == b"as" {
                            // `export as namespace X;` (UMD declaration files).
                            if self.is(j + 1, b"namespace") && self.kind(j + 2) == TokKind::Ident {
                                return self.stmt_end(j + 3);
                            }
                            return None;
                        }
                        // A declaration.
                        self.exporting = true;
                        let r = self.statement(j);
                        self.exporting = false;
                        match r {
                            Some(e) if e > j => Some(e),
                            _ => None,
                        }
                    }
                }
            }
            _ => None,
        }
    }

    fn export_default(&mut self, j: usize) -> Option<usize> {
        let t = self.tok(j);
        match t.kw {
            Kw::Function => return self.function(j, true, true),
            Kw::Class => return self.class(j, true, true),
            _ => {}
        }
        let w = self.text(j);
        if w == b"async" && self.tok(j + 1).kw == Kw::Function && !self.nl(j + 1) {
            return self.function(j + 1, true, true);
        }
        if w == b"abstract" && self.tok(j + 1).kw == Kw::Class {
            return self.class(j + 1, true, true);
        }
        if w == b"interface" && self.kind(j + 1) == TokKind::Ident {
            return self.interface(j);
        }
        let (e, _) = self.pure_expr(j)?;
        self.stmt_end(e)
    }

    /// `const` / `let` / `var` at `i`.
    fn var_decl(&mut self, i: usize, runtime: bool) -> Option<usize> {
        let mut j = i + 1;
        let exported = std::mem::take(&mut self.exporting);
        loop {
            if self.kind(j) != TokKind::Ident {
                return None; // destructuring
            }
            let name = self.ident_str(j)?;
            if exported {
                self.decl_exports.push(name);
            }
            j += 1;
            if self.kind(j) == TokKind::Bang {
                j += 1;
            }
            if self.kind(j) == TokKind::Colon {
                j = self.skip_type(j + 1)?;
            }
            let mut prim = false;
            if self.kind(j) == TokKind::Assign {
                let (e, p) = self.pure_expr(j + 1)?;
                j = e;
                prim = p;
            }
            if runtime {
                self.declare_local(name, prim)?;
            }
            if self.kind(j) == TokKind::Comma {
                j += 1;
                continue;
            }
            return self.stmt_end(j);
        }
    }

    /// A function declaration or expression with the `function` keyword at `i`.
    /// Returns the index after it.
    fn function(&mut self, i: usize, name_optional: bool, declaration: bool) -> Option<usize> {
        let mut j = i + 1;
        if self.kind(j) == TokKind::Star {
            j += 1;
        }
        if self.kind(j) == TokKind::Ident {
            if declaration {
                // Overload signatures repeat the name; duplicate bodies are syntax errors,
                // which the parser validation callers run on candidates rejects.
                let name = self.ident_str(j)?;
                self.fns.insert(name);
                self.locals.insert(name);
            }
            j += 1;
        } else if !name_optional {
            return None;
        }
        if self.kind(j) == TokKind::Lt {
            j = self.skip_angle(j)?;
        }
        if self.kind(j) != TokKind::LParen {
            return None;
        }
        j = self.after_group(j)?;
        if self.kind(j) == TokKind::Colon {
            j = self.skip_type(j + 1)?;
        }
        if self.kind(j) == TokKind::LBrace {
            return self.after_group(j);
        }
        if declaration {
            // Overload signature or `declare function`.
            return self.stmt_end(j);
        }
        None
    }

    /// A class declaration or expression with the `class` keyword at `i`.
    fn class(&mut self, i: usize, name_optional: bool, declaration: bool) -> Option<usize> {
        let mut j = i + 1;
        let mut name = None;
        if self.kind(j) == TokKind::Ident && !matches!(self.text(j), b"extends" | b"implements") {
            name = Some(self.ident_str(j)?);
            j += 1;
        } else if !name_optional && declaration {
            return None;
        }
        if self.kind(j) == TokKind::Lt {
            j = self.skip_angle(j)?;
        }
        if self.tok(j).kw == Kw::Extends {
            let base = self.ident_str(j + 1)?;
            let known = SAFE_BASES.contains(&base)
                || self.locals.contains(base) && !self.prims.contains(base);
            if !known {
                return None;
            }
            j += 2;
            if self.kind(j) == TokKind::Lt {
                j = self.skip_angle(j)?;
            }
        }
        if self.is(j, b"implements") {
            j += 1;
            let mut angle = 0u32;
            loop {
                match self.kind(j) {
                    TokKind::LBrace if angle == 0 => break,
                    TokKind::Ident | TokKind::Dot | TokKind::Comma => j += 1,
                    TokKind::Lt => {
                        angle += 1;
                        j += 1;
                    }
                    TokKind::Gt if angle > 0 => {
                        angle -= 1;
                        j += 1;
                    }
                    TokKind::LBrace | TokKind::LBracket | TokKind::LParen if angle > 0 => {
                        j = self.after_group(j)?;
                    }
                    _ => return None,
                }
            }
        }
        if self.kind(j) != TokKind::LBrace {
            return None;
        }
        let end = self.after_group(j)?;
        self.class_body(j + 1, end - 1)?;
        if declaration && let Some(n) = name {
            self.declare_local(n, false)?;
        }
        Some(end)
    }

    fn class_body(&mut self, mut k: usize, close: usize) -> Option<()> {
        while k < close {
            if self.kind(k) == TokKind::Semi {
                k += 1;
                continue;
            }
            let mut is_static = false;
            // Modifiers: a modifier word followed by another member token.
            while self.kind(k) == TokKind::Ident {
                let w = self.text(k);
                if !MODIFIERS.iter().any(|m| m.as_bytes() == w) {
                    break;
                }
                let next = self.kind(k + 1);
                if w == b"static" && next == TokKind::LBrace {
                    return None; // static block
                }
                if !matches!(
                    next,
                    TokKind::Ident
                        | TokKind::Str
                        | TokKind::Number
                        | TokKind::PrivateName
                        | TokKind::LBracket
                        | TokKind::Star
                ) || self.nl(k + 1)
                {
                    break;
                }
                if w == b"static" {
                    is_static = true;
                }
                k += 1;
            }
            if self.kind(k) == TokKind::Star {
                k += 1;
            }
            match self.kind(k) {
                TokKind::Ident | TokKind::PrivateName | TokKind::Str | TokKind::Number => k += 1,
                _ => return None, // computed keys, index signatures, decorators
            }
            if matches!(self.kind(k), TokKind::Question | TokKind::Bang) {
                k += 1;
            }
            match self.kind(k) {
                TokKind::Lt | TokKind::LParen => {
                    if self.kind(k) == TokKind::Lt {
                        k = self.skip_angle(k)?;
                    }
                    if self.kind(k) != TokKind::LParen {
                        return None;
                    }
                    k = self.after_group(k)?;
                    if self.kind(k) == TokKind::Colon {
                        k = self.skip_type(k + 1)?;
                    }
                    if self.kind(k) == TokKind::LBrace {
                        k = self.after_group(k)?;
                        continue;
                    }
                }
                _ => {
                    if self.kind(k) == TokKind::Colon {
                        k = self.skip_type(k + 1)?;
                    }
                    if self.kind(k) == TokKind::Assign {
                        k = if is_static {
                            self.pure_expr(k + 1)?.0
                        } else {
                            self.skip_expr(k + 1)?
                        };
                    }
                }
            }
            // Member end.
            match self.kind(k) {
                TokKind::Semi => k += 1,
                _ if k == close => {}
                _ if self.nl(k) => {}
                _ => return None,
            }
        }
        Some(())
    }

    /// `enum Name { ... }` with the name at `i`.
    fn enum_decl(&mut self, i: usize, runtime: bool) -> Option<usize> {
        let name = self.ident_str(i)?;
        if self.kind(i + 1) != TokKind::LBrace {
            return None;
        }
        let end = self.after_group(i + 1)?;
        let close = end - 1;
        let mut k = i + 2;
        while k < close {
            match self.kind(k) {
                TokKind::Ident | TokKind::Str => k += 1,
                _ => return None,
            }
            if self.kind(k) == TokKind::Assign {
                k += 1;
                if self.kind(k) == TokKind::Op && self.text(k) == b"-" {
                    k += 1;
                    if self.kind(k) != TokKind::Number {
                        return None;
                    }
                }
                match self.kind(k) {
                    TokKind::Number | TokKind::Str | TokKind::Template => k += 1,
                    _ => return None,
                }
            }
            match self.kind(k) {
                TokKind::Comma => k += 1,
                _ if k == close => {}
                _ => return None,
            }
        }
        if runtime {
            self.declare_local(name, false)?;
        }
        Some(end)
    }

    /// `type Name<...> = <type>` at `i`.
    fn type_alias(&mut self, i: usize) -> Option<usize> {
        let mut j = i + 2;
        if self.kind(j) == TokKind::Lt {
            j = self.skip_angle(j)?;
        }
        if self.kind(j) != TokKind::Assign {
            return None;
        }
        let e = self.skip_type(j + 1)?;
        self.stmt_end(e)
    }

    /// `interface Name ... { ... }` at `i`.
    fn interface(&mut self, i: usize) -> Option<usize> {
        let mut j = i + 2;
        let mut angle = 0u32;
        loop {
            match self.kind(j) {
                TokKind::LBrace if angle == 0 => return self.after_group(j),
                TokKind::LBrace | TokKind::LParen | TokKind::LBracket => j = self.after_group(j)?,
                TokKind::Lt => {
                    angle += 1;
                    j += 1;
                }
                TokKind::Gt if angle > 0 => {
                    angle -= 1;
                    j += 1;
                }
                TokKind::Ident
                | TokKind::Dot
                | TokKind::Comma
                | TokKind::Str
                | TokKind::Number
                | TokKind::Question
                | TokKind::Colon => j += 1,
                TokKind::Assign | TokKind::Op | TokKind::Arrow if angle > 0 => j += 1,
                _ => return None,
            }
        }
    }

    /// `declare ...` with the declared construct at `j`. Ambient declarations
    /// emit no code; global and module augmentations and namespaces are not
    /// modelled.
    fn declare(&mut self, j: usize) -> Option<usize> {
        let t = self.tok(j);
        match t.kw {
            Kw::Const if self.is(j + 1, b"enum") => self.enum_decl(j + 2, false),
            Kw::Const | Kw::Let | Kw::Var => self.var_decl(j, false),
            Kw::Function => {
                let mut k = j + 1;
                if self.kind(k) == TokKind::Star {
                    k += 1;
                }
                if self.kind(k) != TokKind::Ident {
                    return None;
                }
                k += 1;
                if self.kind(k) == TokKind::Lt {
                    k = self.skip_angle(k)?;
                }
                if self.kind(k) != TokKind::LParen {
                    return None;
                }
                k = self.after_group(k)?;
                if self.kind(k) == TokKind::Colon {
                    k = self.skip_type(k + 1)?;
                }
                self.stmt_end(k)
            }
            Kw::Class => self.declare_class(j),
            Kw::Type if self.kind(j + 1) == TokKind::Ident => self.type_alias(j),
            _ => match self.text(j) {
                b"abstract" if self.tok(j + 1).kw == Kw::Class => self.declare_class(j + 1),
                b"interface" => self.interface(j),
                b"enum" => self.enum_decl(j + 1, false),
                _ => None,
            },
        }
    }

    fn declare_class(&mut self, j: usize) -> Option<usize> {
        // Ambient classes: skip to the body and over it.
        let mut k = j + 1;
        let mut angle = 0u32;
        loop {
            match self.kind(k) {
                TokKind::LBrace if angle == 0 => return self.after_group(k),
                TokKind::LBrace | TokKind::LParen | TokKind::LBracket => k = self.after_group(k)?,
                TokKind::Lt => {
                    angle += 1;
                    k += 1;
                }
                TokKind::Gt if angle > 0 => {
                    angle -= 1;
                    k += 1;
                }
                TokKind::Ident | TokKind::Dot | TokKind::Comma => k += 1,
                _ => return None,
            }
        }
    }

    // ---------------------------------------------------------------------------------
    // Types
    // ---------------------------------------------------------------------------------

    /// Skips `<...>` starting at the `<` at `j`; returns the index after the
    /// `>`.
    fn skip_angle(&self, mut j: usize) -> Option<usize> {
        let mut angle = 0u32;
        loop {
            match self.kind(j) {
                TokKind::Lt => angle += 1,
                TokKind::Gt => {
                    angle = angle.checked_sub(1)?;
                    if angle == 0 {
                        return Some(j + 1);
                    }
                }
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    j = self.after_group(j)?;
                    continue;
                }
                TokKind::Eof
                | TokKind::Semi
                | TokKind::RParen
                | TokKind::RBracket
                | TokKind::RBrace => return None,
                _ => {}
            }
            j += 1;
        }
    }

    /// Skips a TypeScript type starting at `j`; returns the index of the first
    /// token after it. A `{` after a complete type ends it (a function
    /// body).
    fn skip_type(&self, mut j: usize) -> Option<usize> {
        let mut expect = true;
        let mut angle = 0u32;
        let mut cond = 0u32;
        let mut prev_ident = false;
        let mut paren = false;
        let start = j;
        loop {
            let t = self.tok(j);
            if expect {
                prev_ident = false;
                paren = false;
                match t.kind {
                    TokKind::Ident => {
                        let w = self.text(j);
                        if matches!(
                            w,
                            b"typeof"
                                | b"keyof"
                                | b"infer"
                                | b"unique"
                                | b"readonly"
                                | b"asserts"
                                | b"new"
                                | b"abstract"
                        ) && matches!(
                            self.kind(j + 1),
                            TokKind::Ident
                                | TokKind::LParen
                                | TokKind::LBracket
                                | TokKind::Lt
                                | TokKind::LBrace
                        ) {
                            j += 1;
                            continue;
                        }
                        if t.kw == Kw::Import && self.kind(j + 1) == TokKind::LParen {
                            j = self.after_group(j + 1)?;
                        } else {
                            j += 1;
                            prev_ident = true;
                        }
                        expect = false;
                    }
                    TokKind::Str | TokKind::Template | TokKind::Number => {
                        j += 1;
                        expect = false;
                    }
                    TokKind::TemplateHead | TokKind::LBrace | TokKind::LBracket => {
                        j = self.after_group(j)?;
                        expect = false;
                    }
                    TokKind::LParen => {
                        j = self.after_group(j)?;
                        expect = false;
                        paren = true;
                        if self.kind(j) == TokKind::Arrow {
                            j += 1;
                            expect = true;
                        }
                        continue;
                    }
                    // Type parameters of a function type: `<T>(x: T) => T`.
                    TokKind::Lt => j = self.skip_angle(j)?,
                    TokKind::Op if matches!(self.text(j), b"|" | b"&" | b"-") => j += 1,
                    _ => return None,
                }
                continue;
            }
            // A complete type so far.
            let nl = t.flags & NEWLINE_BEFORE != 0;
            let w = self.text(j);
            let cont = match t.kind {
                TokKind::Op if matches!(w, b"|" | b"&") => {
                    expect = true;
                    true
                }
                TokKind::Dot if prev_ident => {
                    expect = true;
                    true
                }
                TokKind::LBracket if !nl => {
                    j = self.after_group(j)?;
                    prev_ident = false;
                    continue;
                }
                TokKind::Lt if !nl => {
                    angle += 1;
                    expect = true;
                    true
                }
                TokKind::Gt if angle > 0 => {
                    angle -= 1;
                    prev_ident = false;
                    true
                }
                TokKind::Comma | TokKind::Assign if angle > 0 => {
                    expect = true;
                    true
                }
                TokKind::Arrow if paren => {
                    expect = true;
                    true
                }
                TokKind::Ident if t.kw == Kw::Extends || w == b"is" => {
                    expect = true;
                    true
                }
                TokKind::Question if !nl || cond > 0 || angle > 0 => {
                    cond += 1;
                    expect = true;
                    true
                }
                TokKind::Colon if cond > 0 => {
                    cond -= 1;
                    expect = true;
                    true
                }
                _ => false,
            };
            if !cont {
                if angle > 0 || cond > 0 {
                    return None;
                }
                return (j > start).then_some(j);
            }
            j += 1;
            paren = false;
        }
    }

    // ---------------------------------------------------------------------------------
    // Expressions
    // ---------------------------------------------------------------------------------

    /// Skips an expression that is not evaluated now (arrow bodies, instance
    /// field initializers); returns the index of the token after it.
    fn skip_expr(&self, mut k: usize) -> Option<usize> {
        let start = k;
        loop {
            let t = self.tok(k);
            match t.kind {
                TokKind::Eof
                | TokKind::Semi
                | TokKind::Comma
                | TokKind::RParen
                | TokKind::RBracket
                | TokKind::RBrace => break,
                _ => {}
            }
            if k > start
                && t.flags & NEWLINE_BEFORE != 0
                && ends_expr(&self.tok(k - 1))
                && !continues(&t, self.text(k))
            {
                break;
            }
            k = match t.kind {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace | TokKind::TemplateHead => {
                    self.after_group(k)?
                }
                _ => k + 1,
            };
        }
        (k > start).then_some(k)
    }

    /// A pure expression at `j`: returns the index after it and whether its
    /// value is a primitive.
    fn pure_expr(&mut self, j: usize) -> Option<(usize, bool)> {
        if self.nesting >= MAX_NESTING {
            return None;
        }
        self.nesting += 1;
        let r = self.pure_expr_inner(j);
        self.nesting -= 1;
        r
    }

    fn pure_expr_inner(&mut self, j: usize) -> Option<(usize, bool)> {
        let (mut j, mut prim) = self.pure_unary(j)?;
        loop {
            let t = self.tok(j);
            let glued_prev = |s: &Self, k: usize| s.tok(k).start == s.tok(k - 1).end;
            match t.kind {
                TokKind::Ident if !self.nl(j) && matches!(self.text(j), b"as" | b"satisfies") => {
                    j = self.skip_type(j + 1)?;
                }
                TokKind::Bang if self.ts && glued_prev(self, j) => j += 1,
                TokKind::Op if matches!(self.text(j), b"&&" | b"||" | b"??") => {
                    let (k, p) = self.pure_unary(j + 1)?;
                    j = k;
                    prim = prim && p;
                }
                TokKind::Op | TokKind::Star | TokKind::Lt | TokKind::Gt => {
                    // Operators on primitives run no user code (`valueOf`, `toString`).
                    if !prim {
                        return None;
                    }
                    let w = self.text(j);
                    if t.kind == TokKind::Op
                        && !matches!(
                            w,
                            b"+" | b"-"
                                | b"*"
                                | b"/"
                                | b"%"
                                | b"**"
                                | b"<<"
                                | b"&"
                                | b"|"
                                | b"^"
                                | b"=="
                                | b"!="
                                | b"==="
                                | b"!=="
                                | b"<="
                        )
                    {
                        return None;
                    }
                    let mut k = j + 1;
                    // `>>`, `>>>`, `>=` are split into `>` and `=` tokens.
                    if t.kind == TokKind::Gt {
                        while matches!(self.kind(k), TokKind::Gt) && glued_prev(self, k) {
                            k += 1;
                        }
                        if self.kind(k) == TokKind::Assign && glued_prev(self, k) {
                            k += 1;
                        }
                    }
                    let (e, p) = self.pure_unary(k)?;
                    if !p {
                        return None;
                    }
                    j = e;
                }
                TokKind::Question => {
                    let (k, p1) = self.pure_expr(j + 1)?;
                    if self.kind(k) != TokKind::Colon {
                        return None;
                    }
                    let (e, p2) = self.pure_expr(k + 1)?;
                    return Some((e, prim && p1 && p2));
                }
                _ => return Some((j, prim)),
            }
        }
    }

    fn pure_unary(&mut self, j: usize) -> Option<(usize, bool)> {
        if self.nesting >= MAX_NESTING {
            return None;
        }
        self.nesting += 1;
        let r = self.pure_unary_inner(j);
        self.nesting -= 1;
        r
    }

    fn pure_unary_inner(&mut self, j: usize) -> Option<(usize, bool)> {
        let t = self.tok(j);
        match t.kind {
            TokKind::Op if matches!(self.text(j), b"-" | b"+" | b"~") => {
                let (k, p) = self.pure_unary(j + 1)?;
                p.then_some((k, true))
            }
            TokKind::Bang => {
                let (k, _) = self.pure_unary(j + 1)?;
                Some((k, true))
            }
            TokKind::Ident if matches!(t.kw, Kw::Void | Kw::Typeof) => {
                let (k, _) = self.pure_unary(j + 1)?;
                Some((k, true))
            }
            _ => self.pure_primary(j),
        }
    }

    fn pure_primary(&mut self, j: usize) -> Option<(usize, bool)> {
        let t = self.tok(j);
        match t.kind {
            TokKind::Str | TokKind::Template => Some((j + 1, true)),
            TokKind::Number => Some((j + 1, !self.text(j).ends_with(b"n"))),
            TokKind::Regex => Some((j + 1, false)),
            TokKind::TemplateHead => {
                let end = self.after_group(j)?;
                let mut k = j + 1;
                loop {
                    let (e, p) = self.pure_expr(k)?;
                    if !p {
                        return None;
                    }
                    match self.kind(e) {
                        TokKind::TemplateMiddle => k = e + 1,
                        TokKind::TemplateTail if e + 1 == end => return Some((end, true)),
                        _ => return None,
                    }
                }
            }
            TokKind::Ident => {
                let w = self.text(j);
                if t.kw == Kw::Function {
                    return Some((self.function(j, true, false)?, false));
                }
                if t.kw == Kw::Class {
                    return Some((self.class(j, true, false)?, false));
                }
                if w == b"async" && !self.nl(j + 1) {
                    if self.tok(j + 1).kw == Kw::Function {
                        return Some((self.function(j + 1, true, false)?, false));
                    }
                    if self.kind(j + 1) == TokKind::Ident && self.kind(j + 2) == TokKind::Arrow {
                        return self.arrow_body(j + 3);
                    }
                    if self.kind(j + 1) == TokKind::LParen {
                        return self.paren_arrow(j + 1);
                    }
                }
                if self.kind(j + 1) == TokKind::Arrow && !self.nl(j + 1) {
                    return self.arrow_body(j + 2);
                }
                if matches!(
                    w,
                    b"true" | b"false" | b"null" | b"undefined" | b"NaN" | b"Infinity"
                ) {
                    return Some((j + 1, true));
                }
                if t.kw != Kw::None {
                    return None;
                }
                let name = self.ident_str(j)?;
                if self.locals.contains(name) {
                    return Some((j + 1, self.prims.contains(name)));
                }
                // Possibly a hoisted function declared later.
                self.pending.push(name);
                Some((j + 1, false))
            }
            TokKind::LParen => {
                let end = self.after_group(j)?;
                if matches!(self.kind(end), TokKind::Arrow | TokKind::Colon) {
                    return self.paren_arrow(j);
                }
                let (k, p) = self.pure_expr(j + 1)?;
                (k + 1 == end).then_some((end, p))
            }
            TokKind::Lt if self.ts => {
                // `<T>(x: T) => x` or the type assertion `<T>expr`.
                let k = self.skip_angle(j)?;
                if self.kind(k) == TokKind::LParen
                    && matches!(
                        self.kind(self.after_group(k)?),
                        TokKind::Arrow | TokKind::Colon
                    )
                {
                    return self.paren_arrow(k);
                }
                self.pure_unary(k)
            }
            TokKind::LBracket => {
                let end = self.after_group(j)?;
                let mut k = j + 1;
                while k + 1 < end {
                    if self.kind(k) == TokKind::Comma {
                        k += 1;
                        continue;
                    }
                    let (e, _) = self.pure_expr(k)?;
                    k = e;
                    match self.kind(k) {
                        TokKind::Comma => k += 1,
                        _ if k + 1 == end => {}
                        _ => return None,
                    }
                }
                Some((end, false))
            }
            TokKind::LBrace => Some((self.object(j)?, false)),
            _ => None,
        }
    }

    /// An arrow function whose parameter list starts at the `(` at `j`.
    fn paren_arrow(&mut self, j: usize) -> Option<(usize, bool)> {
        let mut k = self.after_group(j)?;
        if self.kind(k) == TokKind::Colon {
            k = self.skip_type(k + 1)?;
        }
        if self.kind(k) != TokKind::Arrow {
            return None;
        }
        self.arrow_body(k + 1)
    }

    fn arrow_body(&self, k: usize) -> Option<(usize, bool)> {
        // A block body opens at the arrow's depth; a JSX child container (`=>
        // <a>{x}</a>`) is deeper.
        if self.kind(k) == TokKind::LBrace && self.tok(k).depth == self.tok(k - 1).depth {
            return Some((self.after_group(k)?, false));
        }
        Some((self.skip_expr(k)?, false))
    }

    /// An object literal at the `{` at `j`; returns the index after it.
    fn object(&mut self, j: usize) -> Option<usize> {
        let end = self.after_group(j)?;
        let close = end - 1;
        let mut k = j + 1;
        while k < close {
            let mut method_mods = false;
            if matches!(self.text(k), b"get" | b"set" | b"async")
                && self.kind(k) == TokKind::Ident
                && matches!(
                    self.kind(k + 1),
                    TokKind::Ident | TokKind::Str | TokKind::Number | TokKind::Star
                )
            {
                k += 1;
                method_mods = true;
            }
            if self.kind(k) == TokKind::Star {
                k += 1;
                method_mods = true;
            }
            let key = k;
            match self.kind(k) {
                TokKind::Ident | TokKind::Str | TokKind::Number => k += 1,
                _ => return None, // spreads, computed keys
            }
            match self.kind(k) {
                TokKind::Colon if !method_mods => {
                    k = self.pure_expr(k + 1)?.0;
                }
                TokKind::LParen | TokKind::Lt => {
                    if self.kind(k) == TokKind::Lt {
                        k = self.skip_angle(k)?;
                    }
                    if self.kind(k) != TokKind::LParen {
                        return None;
                    }
                    k = self.after_group(k)?;
                    if self.kind(k) == TokKind::Colon {
                        k = self.skip_type(k + 1)?;
                    }
                    if self.kind(k) != TokKind::LBrace {
                        return None;
                    }
                    k = self.after_group(k)?;
                }
                TokKind::Comma | TokKind::RBrace if !method_mods => {
                    // Shorthand: a reference.
                    if self.kind(key) != TokKind::Ident {
                        return None;
                    }
                    self.pure_primary(key)?;
                }
                _ => return None,
            }
            match self.kind(k) {
                TokKind::Comma => k += 1,
                _ if k == close => {}
                _ => return None,
            }
        }
        Some(end)
    }
}

#[cfg(test)]
mod tests;
