//! The recognizer: finds module references in the token stream.
//!
//! Recognized forms (`'x'` is a string literal or a substitution-free template
//! literal):
//!
//! | source | kind | flags |
//! |---|---|---|
//! | `import 'x'`, `import a, {b as c} from 'x'`, `import * as ns from 'x'` | `Import` | |
//! | `import type ... from 'x'`, Flow `import typeof ...` | `Import` | `TYPE_ONLY` |
//! | `export * from 'x'`, `export * as ns from 'x'`, `export {a} from 'x'` | `ReExport` | |
//! | `export type {A} from 'x'`, `export type * from 'x'` | `ReExport` | `TYPE_ONLY` |
//! | `import a = require('x')` (TS), `import type a = require('x')` | `Require` | (`TYPE_ONLY`) |
//! | `/// <reference path="x" />`, `/// <reference types="x" />` | `Import` | `TYPE_ONLY` |
//! | `import('x')` | `DynamicImport` | `LAZY` (+ `OPTIONAL` in `try`) |
//! | `require('x')`, `require.resolve('x')`, `module.require('x')` | `Require` | (`OPTIONAL` in `try`) |
//! | `import.meta.resolve('x')` | `DynamicImport` | `LAZY` (+ `OPTIONAL` in `try`) |
//!
//! Import attributes (`with { type: 'json' }`, `assert {...}`) are accepted and
//! ignored. In declaration files (`.d.ts`) every reference is `TYPE_ONLY`.
//! `typeof import('x')` (a TypeScript type query) is `TYPE_ONLY`. An import
//! whose named specifiers are all `type`-qualified (`import {type A} from 'x'`)
//! is *not* `TYPE_ONLY`: under `verbatimModuleSyntax` it is kept as a
//! side-effect import, and over-approximating is the safe choice.
//!
//! Not references: `obj.require(...)`, `obj.import`, `import.meta` (other than
//! `import.meta.resolve(...)`, which finds what `import()` would load),
//! property keys and methods named `import`/`require` (`{ require(x) {} }`),
//! `function require(...)`, TypeScript signatures (`require(id: string): any`),
//! AMD `require([...], cb)`, and anything inside comments, strings, regexes,
//! template text and JSX text.
//!
//! # Dynamic references
//!
//! Non-literal arguments go through the partial evaluator ([`evaluator`]):
//! concatenation, templates, ternaries and `||`/`??` (alternatives), top-level
//! `const` values, `__dirname`, `import.meta.url`, `path.*`, `fileURLToPath`,
//! `new URL(x, import.meta.url)`, `process.cwd()`, `process.env.X`. The result
//! is a literal, a pattern ([`Specifier::Pattern`], never with a leading
//! wildcard) or alternatives; anything else is [`Specifier::Unknown`] with a
//! [`UncertaintyKind::ComputedSpecifier`] record holding the argument's source
//! text. Paths anchored at the file (`__dirname`, `import.meta.url`) are
//! emitted relative to the file's directory with [`Base::FileDir`]; relative
//! file system paths that are not anchored are relative to [`Base::Cwd`]
//! (`CwdDependent`).
//!
//! A `require` whose specifier cannot be said but ends with `.json` in every
//! alternative (`require(join(dir, 'auth.json'))`) reads a data file: Node
//! parses a `.json` file, runs nothing in it and loads nothing further. Its
//! unknown reference is a [`RefKind::FsRead`], so the record is that of a read
//! with a computed path (bounded by names in a change query), not of a module
//! load. This goes by the extension, as Node does, and assumes that what Node's
//! fallbacks make possible is not done: a module `auth.json.js` or a
//! directory `auth.json/` with an index where no file `auth.json` exists, a
//! replaced `require.extensions['.json']`. Like every read with a computed path
//! it is then held to its package by the names bound, which is wrong where the
//! unknown start of the specifier is a package name (`require(name +
//! '/package.json')` reads the manifest of another package through
//! `node_modules`): the assumption of that bound, not checked here. A specifier
//! that can be said (a literal, a known directory prefix) stays a
//! `require` reference; `require.resolve`, `import()` and aliases of `require`
//! are not concerned, and neither is a site the value-flow engine did not
//! evaluate.
//!
//! A module load can name **a file next to a resolved module**, or a module
//! **searched from another place** than the file:
//!
//! - `require.resolve('pkg')` and `import.meta.resolve('pkg')` find a file only
//!   the resolver knows ([`JsVal::Resolved`]: the module and a path joined onto
//!   its file). The path and URL functions keep it: `path.dirname` / `join` /
//!   `resolve`, `pathToFileURL`, `fileURLToPath`, `new URL(rel, base)` (in the
//!   directory of a file, inside a directory URL), `.href`, `.pathname`, and `+
//!   '/x'`. `import(new URL('./x.js', import.meta.resolve('pkg')).href)` is the
//!   reference `import.meta.resolve('pkg')/../x.js` with the base
//!   [`evaluator::RESOLVED_BASE`]; the resolver finds `pkg` with the conditions
//!   of the call that found it and joins the rest.
//! - `require.resolve(x, { paths: [p, ...] })` searches from each `p` instead
//!   of the file, and `createRequire(p)` gives a `require` function that
//!   resolves as a module at `p` would ([`evaluator::Located`] with a
//!   [`evaluator::Start`]; a relative request is a path joined onto the place).
//!   The callee decides here, so the recognizer hands the **whole call** to the
//!   value-flow engine (`load_site`, [`crate::flow::Ask::Load`]): `req(x)` and
//!   `req.resolve(x)` of a name bound to `createRequire(a)` unless `a` is
//!   literally the file (`import.meta.url`, `__filename`) and the name is a
//!   `const`, `createRequire(a)(x)`, `createRequire(a).resolve(x)`, and any
//!   `resolve` call with a second argument. The token pass records such a call
//!   as an unknown load, and the engine replaces it with what the call
//!   resolves. A place that cannot be evaluated (a parameter, the working
//!   directory) keeps the record: **a load is never resolved from the file
//!   because its real place is not known**. A request from the environment
//!   (`req(process.env.X)`) keeps its template, which says where the function
//!   searches from ([`turborepo_trace_core::env::EnvFrom`]): the environment
//!   layer resolves a relative value from that place, and not at all when the
//!   place is not known; an absolute value names the same module from anywhere.
//!
//! Runtime file APIs come from a data table ([`api`]): `fs` / `fs/promises` /
//! `fs-extra` / `graceful-fs` reads (`FsRead`, existence probes `OPTIONAL`),
//! directory reads (`FsReadDir`), `new Worker(x)` (`Worker`),
//! `child_process.fork(x)` (`ChildProcess`), and `new URL(x, import.meta.url)`
//! anywhere (`FsRead`). Calls match through import/require bindings at any
//! depth (`const {readFileSync: r} = require('fs')`, `import * as fs from
//! 'node:fs'`, `const fsp = require('fs').promises`,
//! `require('fs').readFileSync(x)`); bindings declared inside a function end
//! with it.
//!
//! Loaders: `const r = require; r('x')` and a `require` that is shadowed by a
//! parameter, variable or function declaration (bundle wrappers such as
//! `function (require, module, exports) {...}`) still produce edges for
//! evaluable arguments, like nft does, but carry
//! [`UncertaintyKind::AliasedLoader`]. A `const` bound to
//! `createRequire(import.meta.url)` or `createRequire(__filename)` has plain
//! `require` semantics; any other `createRequire(...)` is a value (above).
//! `eval(...)` and `new Function(...)` produce no edge and an
//! [`UncertaintyKind::EvalLike`] file diagnostic. So do the functions of `vm`
//! that run source text (`vm.runInNewContext(code)`, `runInContext`,
//! `runInThisContext`, `compileFunction`, `new vm.Script(code)`, `new
//! vm.SourceTextModule(code)`, through any binding of the module), and `new
//! Worker(code, { eval: true })` with a literal `eval: true` in the options
//! object: the first argument is source text, and no file is loaded.
//!
//! TypeScript type positions: `import('x')` used as a type (`type T =
//! import('x').A`, `let a: import('x').T`, `Foo<import('x').T>`, `typeof
//! import('x')`, anything inside a `type X = ...` alias) is `TYPE_ONLY`; `await
//! import('x')`, `import('x').then(...)` and `import()` in object literals,
//! ternaries and `case` labels stay runtime.
//!
//! Top-level `let`/`var` initial values are used like constants, with a
//! `PartialSpecifier` record (later assignments are not tracked). A call
//! `x.join(a, ...)` or `x.resolve(a, ...)` on an unknown `x` whose first
//! argument is an anchored path is taken as `path.join` / `path.resolve`
//! (bundler namespace objects), also with a record.
//!
//! # Deep pass
//!
//! Files that mention `__dirname`, `__filename` or `process.cwd`, call
//! `require` with a computed argument, look like a browserify bundle or import
//! `pino`/`fastify` are re-lexed once more for nft's context-dependent rules:
//! path values emitted as assets wherever they occur, require wrappers,
//! browserify module tables and pino transport targets. See [`deep`].
//!
//! Top-level bindings (`import fs from 'fs'`, `const {readFile: rf} =
//! require('fs')`, ...) are reported in [`Extraction::bindings`] when
//! [`ScanOptions::bindings`] is set.

pub mod api;
pub mod args;
pub mod config;
mod deep;
pub use deep::{MODULE_STRING_BASE, module_string_shape};
pub mod evaluator;
pub mod exports;
pub(crate) mod origin;
mod patharg;
pub mod prefilter;
mod statuse;
#[cfg(test)]
mod tests;
mod trycatch;

use std::{borrow::Cow, collections::HashMap};

use api::{ApiBinding, ApiEntry, BaseRule, CallStyle, Loader};
use evaluator::{JsVal, Outcome, Scope};
use turborepo_trace_core::{
    Base, Origin, RawRef, RefFlags, RefKind, Span, Specifier, Uncertainty, UncertaintyKind,
    eval::Value,
};

use crate::lexer::{Dialect, Jsx, Kw, Lexer, NEWLINE_BEFORE, TYPE_POS, TokKind, Token};

/// Options for [`scan`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// Collect top-level import/require bindings ([`Extraction::bindings`]).
    pub bindings: bool,
    /// Collect the file's environment assignment and helper facts
    /// ([`Extraction::facts`], [`crate::flow::env`]).
    pub env_facts: bool,
}

/// What a binding refers to inside its module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Imported<'s> {
    /// The whole module: `import * as ns`, `const m = require('m')`, `import m
    /// = require()`, `const m = await import('m')`.
    Namespace,
    /// `import d from 'm'`.
    Default,
    /// A named export: `import {a as b}`, `const {a: b} = require()`,
    /// `const b = require('m').a`.
    Named(Cow<'s, str>),
}

/// How a binding was introduced.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BindingSource {
    /// A static `import` declaration.
    Import,
    /// `require(...)` or TS `import x = require(...)`.
    Require,
    /// `await import(...)`.
    DynamicImport,
}

/// A top-level local name bound to a module (or one of its exports).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding<'s> {
    /// The local name.
    pub local: &'s str,
    /// Where the local name is declared.
    pub local_span: Span,
    /// What it refers to.
    pub imported: Imported<'s>,
    /// The module specifier.
    pub module: Cow<'s, str>,
    /// How it was introduced.
    pub source: BindingSource,
    /// Type-only (`import type`, `import {type A}`).
    pub type_only: bool,
}

/// The result of [`scan`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extraction<'s> {
    /// References, in source order.
    pub refs: Vec<RawRef<'s>>,
    /// File-level diagnostics: at most one `ParseIncomplete`, plus one
    /// `EvalLike` per `eval(...)` / `new Function(...)`.
    pub diagnostics: Vec<Uncertainty>,
    /// Top-level bindings (empty unless [`ScanOptions::bindings`]).
    pub bindings: Vec<Binding<'s>>,
    /// Whether the prefilter skipped lexing.
    pub skipped: bool,
    /// Environment facts ([`turborepo_trace_core::env`]): uses always,
    /// assignments and helpers with [`ScanOptions::env_facts`].
    pub facts: Vec<turborepo_trace_core::env::EnvFact>,
    /// The argument expressions of the sites that were evaluated (file APIs,
    /// module loads with a computed argument): what
    /// [`crate::flow::anchors`] takes as consumed by a site.
    pub site_args: Vec<Span>,
    /// The sites the value-flow pass evaluated, for cross-file facts
    /// ([`crate::xfile`]).
    pub(crate) flow_sites: Vec<crate::flow::facts::SiteIn>,
}

/// Extracts references from `src`. Linear time; never panics.
pub fn scan<'s>(src: &'s [u8], dialect: Dialect, opts: ScanOptions) -> Extraction<'s> {
    scan_named(src, dialect, opts, None)
}

/// [`scan`] for a file named `file_name` (its last path component), which lets
/// the evaluator express `__filename` (`path.basename(__filename)`,
/// `readFileSync(__filename)`).
pub fn scan_named<'s>(
    src: &'s [u8],
    dialect: Dialect,
    opts: ScanOptions,
    file_name: Option<&str>,
) -> Extraction<'s> {
    // No trigger word: only module strings (deep pass) can be references.
    if !prefilter::may_contain_refs(src) && (dialect.dts || !deep::may_have_module_strings(src)) {
        // No reference, but possibly assignments of environment variables.
        let facts = if env_facts_wanted(src, opts) {
            crate::flow::evaluate(src, dialect, file_name, &[], true)
                .map(|o| o.env)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        return Extraction {
            skipped: true,
            facts,
            ..Extraction::default()
        };
    }
    let mut ex = Extraction::default();
    if !run(src, dialect, opts, file_name, &mut ex) {
        // Heuristic JSX failed: re-run once with JSX off.
        ex = Extraction::default();
        run(
            src,
            Dialect {
                jsx: Jsx::Off,
                ..dialect
            },
            opts,
            file_name,
            &mut ex,
        );
    }
    ex
}

/// Whether the value-flow pass should collect environment facts for `src`.
/// `TURBO_TRACE_DEBUG_ENV=0` turns them off (for A/B measurements; graphs built
/// either way are not interchangeable).
fn env_facts_wanted(src: &[u8], opts: ScanOptions) -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("ENV").is_none_or(|v| v != "0")
    });
    opts.env_facts && *ENABLED && crate::flow::env::may_have_facts(src)
}

/// One pass. Returns false if heuristic JSX failed (and `ex` must be
/// discarded).
fn run<'s>(
    src: &'s [u8],
    dialect: Dialect,
    opts: ScanOptions,
    file_name: Option<&str>,
    ex: &mut Extraction<'s>,
) -> bool {
    let mut r = Recognizer {
        src,
        dialect,
        ts: Stream {
            lx: Lexer::new(src, dialect),
            buf: Vec::new(),
            head: 0,
            low: 0,
            cur: 0,
        },
        bindings_on: opts.bindings,
        env_facts: env_facts_wanted(src, opts),
        ex,
        file_name,
        consts: HashMap::new(),
        vars: HashMap::new(),
        apis: Vec::new(),
        loaders: Vec::new(),
        shadows: Vec::new(),
        max_scope: 0,
        type_region_end: 0,
        url_suppress_until: 0,
        sites: Vec::new(),
    };
    r.run();
    if r.ts.lx.jsx_failed() {
        return false;
    }
    r.flow_pass();
    r.deep_pass();
    trycatch::clear_unswallowed(src, dialect, &mut r.ex.refs);
    if let Some(u) = r.ts.lx.take_error() {
        r.ex.diagnostics.push(u);
    }
    true
}

/// The lexer plus a lookahead queue. Peeked tokens are later returned by
/// [`Stream::next`], so every token is seen by the main loop exactly once.
///
/// The stream also tracks the lowest context depth among the tokens consumed
/// since the last [`Stream::reset_low`] (including tokens the lexer skipped),
/// so the recognizer can tell when a scope (a function body declaring a binding
/// or shadowing `require`) has closed.
struct Stream<'s> {
    lx: Lexer<'s>,
    buf: Vec<Token>,
    head: usize,
    /// Lowest depth since the last reset.
    low: u32,
    /// Depth after the last consumed token.
    cur: u32,
}

/// The context depth after `t`: openers push.
#[inline]
fn after_depth(t: &Token) -> u32 {
    t.depth
        + u32::from(matches!(
            t.kind,
            TokKind::LParen
                | TokKind::LBracket
                | TokKind::LBrace
                | TokKind::TemplateHead
                | TokKind::TemplateMiddle
        ))
}

impl Stream<'_> {
    #[inline]
    fn note(&mut self, t: &Token) {
        let d = after_depth(t);
        self.cur = d;
        self.low = self.low.min(d);
    }

    fn reset_low(&mut self) {
        self.low = self.cur;
    }

    #[inline]
    fn next(&mut self) -> Token {
        let t = if self.head < self.buf.len() {
            let t = self.buf[self.head];
            self.head += 1;
            if self.head == self.buf.len() {
                self.buf.clear();
                self.head = 0;
            }
            t
        } else {
            self.lx.next_token()
        };
        self.note(&t);
        t
    }

    /// The token `n` positions ahead (`0` = the one [`Stream::next`] returns
    /// next).
    #[inline]
    fn peek(&mut self, n: usize) -> Token {
        while self.buf.len() - self.head <= n {
            let t = self.lx.next_token();
            self.buf.push(t);
        }
        self.buf[self.head + n]
    }

    /// The next token that can start a reference (see
    /// [`Lexer::next_matching`]): buffered tokens are checked first, then
    /// the lexer skips ahead.
    #[inline]
    fn next_trigger(&mut self, mask: u64, top_mask: u64) -> Token {
        while self.head < self.buf.len() {
            let t = self.buf[self.head];
            self.head += 1;
            self.note(&t);
            if is_trigger(&t, mask, top_mask, &self.lx) {
                return t;
            }
        }
        self.buf.clear();
        self.head = 0;
        self.lx.reset_low_water();
        let t = self.lx.next_matching_top(mask, top_mask);
        self.low = self.low.min(self.lx.low_water() as u32);
        self.note(&t);
        t
    }

    /// Peeked tokens `a..b` (call [`Stream::peek`] up to `b - 1` first).
    fn window(&self, a: usize, b: usize) -> &[Token] {
        &self.buf[self.head + a..self.head + b]
    }
}

#[inline]
fn is_trigger(t: &Token, mask: u64, top_mask: u64, lx: &Lexer<'_>) -> bool {
    match t.kind {
        TokKind::Ident => {
            let b = 1u64 << t.kw as u8;
            mask & b != 0 || (top_mask & b != 0 && t.depth == 0) || lx.is_dyn_trigger(t)
        }
        TokKind::Eof | TokKind::TripleSlash => true,
        _ => false,
    }
}

const fn bit(k: Kw) -> u64 {
    1u64 << k as u8
}

/// Keywords that can start a reference anywhere.
const TRIGGERS: u64 = bit(Kw::Import)
    | bit(Kw::Export)
    | bit(Kw::Require)
    | bit(Kw::Module)
    | bit(Kw::New)
    | bit(Kw::Eval);
/// Keywords that matter only at depth 0: declarations (bindings and constants).
const TOP_TRIGGERS: u64 = bit(Kw::Const) | bit(Kw::Let) | bit(Kw::Var);

/// Maximum tokens scanned for a `type X = ...` alias.
const MAX_REGION_TOKENS: usize = 2048;
/// Maximum live bindings to tracked modules and loader names. Lookups are
/// linear scans, so the caps keep pathological inputs linear; real files have a
/// handful.
const MAX_BINDINGS: usize = 256;
const MAX_LOADERS: usize = 32;
/// Maximum remembered top-level constants (and, separately, `let`/`var`
/// values).
const MAX_CONSTS: usize = 4096;

/// The result of capturing the first argument of a call.
struct ArgCapture {
    /// Peek index of the first argument token.
    first: usize,
    /// Peek index just past the first argument.
    end: usize,
    /// Peek index of the closing `)`, if it was reached before a top-level `,`.
    close: Option<usize>,
    /// Peek index of the token ending the call expression (the `)`, or the last
    /// captured token).
    last: usize,
    /// The capture hit the token cap or the end of input.
    truncated: bool,
}

struct Recognizer<'s, 'e> {
    src: &'s [u8],
    dialect: Dialect,
    ts: Stream<'s>,
    bindings_on: bool,
    /// Collect environment assignment and helper facts in the value-flow pass.
    env_facts: bool,
    ex: &'e mut Extraction<'s>,
    /// The file's own name (for `__filename`).
    file_name: Option<&'e str>,
    /// Top-level `const NAME = <evaluable>` values (and whether they read
    /// `process.env`).
    consts: HashMap<&'s str, (JsVal, bool)>,
    /// Top-level `let NAME = ...` / `var NAME = ...` initial values (see
    /// [`Scope::vars`]).
    vars: HashMap<&'s str, (JsVal, bool)>,
    /// Bindings to tracked modules, at any depth.
    apis: Vec<ApiBinding<'s>>,
    /// Names with `require` semantics.
    loaders: Vec<Loader<'s>>,
    /// Depths of scopes where `require` is shadowed.
    shadows: Vec<u32>,
    /// Maximum depth among `apis`, `loaders` and `shadows` (0 when none is
    /// scoped).
    max_scope: u32,
    /// `import(...)` before this offset is inside a TypeScript `type X = ...`
    /// alias.
    type_region_end: u32,
    /// `new URL(...)` before this offset was already part of a tracked call.
    url_suppress_until: u32,
    /// Evaluated sites, for the value-flow pass.
    sites: Vec<FlowSite>,
}

impl<'s> Recognizer<'s, '_> {
    fn run(&mut self) {
        let mut mask = TRIGGERS;
        if self.dialect.ts && !self.dialect.dts {
            mask |= bit(Kw::Type);
        }
        loop {
            let t = self.ts.next_trigger(mask, TOP_TRIGGERS);
            self.prune();
            match t.kind {
                TokKind::Eof => break,
                TokKind::Ident => match t.kw {
                    Kw::Import => self.on_import(t),
                    Kw::Export => self.on_export(t),
                    Kw::Require => self.on_require(t),
                    Kw::Module => self.on_module(t),
                    Kw::Const | Kw::Let | Kw::Var if t.depth == 0 => self.on_decl(t),
                    Kw::New => self.on_new(t),
                    Kw::Eval => self.on_eval(t),
                    Kw::Type => self.on_type(t),
                    Kw::None => self.on_dyn(t),
                    _ => {}
                },
                TokKind::TripleSlash => self.on_triple_slash(t),
                _ => {}
            }
        }
    }

    /// Ends bindings, loaders and `require` shadows whose scope closed since
    /// the last trigger.
    fn prune(&mut self) {
        let low = self.ts.low;
        if self.max_scope > low {
            self.apis.retain(|b| b.depth <= low);
            self.loaders.retain(|l| l.depth <= low);
            self.shadows.retain(|&d| d <= low);
            self.max_scope = self
                .apis
                .iter()
                .map(|b| b.depth)
                .chain(self.loaders.iter().map(|l| l.depth))
                .chain(self.shadows.iter().copied())
                .max()
                .unwrap_or(0);
        }
        self.ts.reset_low();
    }

    fn scope(&self) -> Scope<'_, 's> {
        Scope {
            src: self.src,
            consts: &self.consts,
            apis: &self.apis,
            loaders: &self.loaders,
            file_name: self.file_name,
        }
    }

    /// Evaluates `toks` with the file's scope, including top-level `let`/`var`
    /// values.
    fn eval(&self, toks: &[Token]) -> Outcome {
        evaluator::evaluate_with(&self.scope(), Some(&self.vars), toks)
    }

    /// Consumes `n` peeked tokens (known to contain no references).
    fn consume(&mut self, n: usize) {
        for _ in 0..n {
            self.ts.next();
        }
    }

    #[inline]
    fn text(&self, t: &Token) -> &'s [u8] {
        t.text(self.src)
    }

    fn is_literal(&self, t: &Token) -> bool {
        t.literal_content(self.src).is_some()
    }

    fn base_flags(&self) -> RefFlags {
        if self.dialect.dts {
            RefFlags::TYPE_ONLY
        } else {
            RefFlags::empty()
        }
    }

    /// Pushes a reference whose specifier is the literal token `lit`.
    fn emit_literal(&mut self, kind: RefKind, lit: &Token, span: Span, flags: RefFlags) {
        let flags = flags | self.base_flags();
        let r = match args::literal(self.src, lit) {
            Some(Ok(s)) => RawRef::new(kind, Specifier::Literal(s), span),
            Some(Err(u)) => RawRef::new(kind, Specifier::Unknown, span).with_uncertainty(u),
            None => return,
        };
        self.ex.refs.push(r.with_flags(flags));
    }

    fn literal_str(&self, lit: &Token) -> Option<Cow<'s, str>> {
        args::literal(self.src, lit)?.ok()
    }

    fn ident_str(&self, t: &Token) -> Option<&'s str> {
        std::str::from_utf8(self.text(t)).ok()
    }

    // `&Cow` (not `&str`): cloning keeps a borrowed specifier borrowed.
    #[allow(clippy::ptr_arg)]
    fn bind(
        &mut self,
        local: &Token,
        imported: Imported<'s>,
        module: &Cow<'s, str>,
        source: BindingSource,
        type_only: bool,
    ) {
        let Some(name) = self.ident_str(local) else {
            return;
        };
        if !type_only {
            let member = match &imported {
                Imported::Namespace | Imported::Default => None,
                Imported::Named(n) => Some(n.clone()),
            };
            self.add_api(name, module.clone(), member, 0);
        }
        if self.bindings_on {
            self.ex.bindings.push(Binding {
                local: name,
                local_span: local.span(),
                imported,
                module: module.clone(),
                source,
                type_only,
            });
        }
    }

    /// Records that `local` refers to `module` (or its export `member`) in the
    /// scope at `depth`, if the module is tracked.
    fn add_api(
        &mut self,
        local: &'s str,
        module: Cow<'s, str>,
        member: Option<Cow<'s, str>>,
        depth: u32,
    ) {
        let module = match module {
            Cow::Borrowed(s) => Cow::Borrowed(api::normalize_module(s)),
            Cow::Owned(s) => Cow::Owned(api::normalize_module(&s).to_owned()),
        };
        if !api::is_tracked_module(&module) {
            return;
        }
        let member = member.filter(|m| m != "default");
        if self.apis.len() >= MAX_BINDINGS {
            return;
        }
        if self.apis.iter().rev().take(8).any(|b| {
            b.local == local && b.depth == depth && b.module == module && b.member == member
        }) {
            return;
        }
        self.ts.lx.add_trigger_word(local.as_bytes());
        self.max_scope = self.max_scope.max(depth);
        self.apis.push(ApiBinding {
            local,
            module,
            member,
            depth,
        });
    }

    // ---- import ----

    fn on_import(&mut self, t: Token) {
        let n = self.ts.peek(0);
        match n.kind {
            _ if self.is_literal(&n) => {
                self.emit_literal(
                    RefKind::Import,
                    &n,
                    Span::new(t.start, n.end),
                    RefFlags::empty(),
                );
                self.consume(1);
            }
            TokKind::LParen => {
                let mut flags = RefFlags::LAZY;
                if t.prev_kw == Kw::Typeof {
                    flags |= RefFlags::TYPE_ONLY;
                }
                self.call(t, RefKind::DynamicImport, 0, flags, None);
            }
            // `import.meta.resolve(x)`: finds what `import(x)` would load (ESM resolution).
            TokKind::Dot => {
                let (m, d, r) = (self.ts.peek(1), self.ts.peek(2), self.ts.peek(3));
                if self.text(&m) == b"meta"
                    && d.kind == TokKind::Dot
                    && self.text(&r) == b"resolve"
                    && self.ts.peek(4).kind == TokKind::LParen
                {
                    if self.more_args(4) {
                        // A parent URL: where the search starts.
                        self.load_site(t, 4, RefKind::DynamicImport, RefFlags::LAZY, false);
                    } else {
                        self.call(t, RefKind::DynamicImport, 4, RefFlags::LAZY, None);
                    }
                }
            }
            _ => self.import_decl(t),
        }
    }

    fn import_decl(&mut self, t: Token) {
        let mut i = 0;
        let mut type_only = false;
        let first = self.ts.peek(0);
        if first.kind == TokKind::Ident {
            let modifier_type = first.kw == Kw::Type || self.text(&first) == b"typeof";
            let modifier_phase = matches!(first.kw, Kw::Defer | Kw::Source);
            if modifier_type || modifier_phase {
                let second = self.ts.peek(1);
                let is_modifier = match second.kind {
                    TokKind::LBrace | TokKind::Star => true,
                    TokKind::Ident if second.kw == Kw::From => {
                        // `import type from 'x'` (default import named `type`) vs
                        // `import type from from 'x'`.
                        let third = self.ts.peek(2);
                        third.kind == TokKind::Ident && third.kw == Kw::From
                    }
                    TokKind::Ident => true,
                    _ => false,
                };
                if is_modifier {
                    type_only = modifier_type;
                    i = 1;
                }
            }
        }
        let collect = t.depth == 0;
        let mut pending: Vec<(Token, Imported<'s>, bool)> = Vec::new();

        let mut tk = self.ts.peek(i);
        let mut need_from = false;
        if tk.kind == TokKind::Ident {
            let after = self.ts.peek(i + 1);
            if after.kind == TokKind::Assign {
                self.import_equals(t, i, type_only);
                return;
            }
            if collect {
                pending.push((tk, Imported::Default, type_only));
            }
            i += 1;
            need_from = true;
            if after.kind == TokKind::Comma {
                i += 1;
                tk = self.ts.peek(i);
            } else {
                tk = Token::default();
            }
        }
        match tk.kind {
            TokKind::Star => {
                let a = self.ts.peek(i + 1);
                let name = self.ts.peek(i + 2);
                if a.kw != Kw::As || name.kind != TokKind::Ident {
                    return;
                }
                if collect {
                    pending.push((name, Imported::Namespace, type_only));
                }
                i += 3;
            }
            TokKind::LBrace => {
                i += 1;
                loop {
                    let e = self.ts.peek(i);
                    match e.kind {
                        TokKind::RBrace => {
                            i += 1;
                            break;
                        }
                        TokKind::Comma => i += 1,
                        TokKind::Ident | TokKind::Str => {
                            let mut name = e;
                            let mut entry_type = type_only;
                            let next = self.ts.peek(i + 1);
                            if e.kind == TokKind::Ident
                                && e.kw == Kw::Type
                                && matches!(next.kind, TokKind::Ident | TokKind::Str)
                                && !(next.kw == Kw::As
                                    && matches!(
                                        self.ts.peek(i + 2).kind,
                                        TokKind::Comma | TokKind::RBrace
                                    ))
                            {
                                entry_type = true;
                                i += 1;
                                name = next;
                            }
                            i += 1;
                            let mut local = name;
                            if self.ts.peek(i).kw == Kw::As
                                && self.ts.peek(i + 1).kind == TokKind::Ident
                            {
                                local = self.ts.peek(i + 1);
                                i += 2;
                            }
                            if collect && local.kind == TokKind::Ident {
                                let imported = if name.kind == TokKind::Str {
                                    self.literal_str(&name).map(Imported::Named)
                                } else {
                                    self.ident_str(&name)
                                        .map(|s| Imported::Named(Cow::Borrowed(s)))
                                };
                                if let Some(imported) = imported {
                                    pending.push((local, imported, entry_type));
                                }
                            }
                        }
                        _ => return,
                    }
                }
            }
            _ if need_from => {}
            _ => return,
        }
        let from = self.ts.peek(i);
        let lit = self.ts.peek(i + 1);
        if from.kind != TokKind::Ident || from.kw != Kw::From || !self.is_literal(&lit) {
            return;
        }
        let flags = if type_only {
            RefFlags::TYPE_ONLY
        } else {
            RefFlags::empty()
        };
        self.emit_literal(RefKind::Import, &lit, Span::new(t.start, lit.end), flags);
        if collect && let Some(module) = self.literal_str(&lit) {
            for (local, imported, ty) in pending {
                self.bind(&local, imported, &module, BindingSource::Import, ty);
            }
        }
        self.consume(i + 2);
    }

    /// `import a = require('x')`; `i` is the peek index of `a`.
    fn import_equals(&mut self, t: Token, i: usize, type_only: bool) {
        let name = self.ts.peek(i);
        let req = self.ts.peek(i + 2);
        let lp = self.ts.peek(i + 3);
        let lit = self.ts.peek(i + 4);
        let rp = self.ts.peek(i + 5);
        if self.text(&req) != b"require"
            || lp.kind != TokKind::LParen
            || !self.is_literal(&lit)
            || rp.kind != TokKind::RParen
        {
            return;
        }
        let flags = if type_only {
            RefFlags::TYPE_ONLY
        } else {
            RefFlags::empty()
        };
        self.emit_literal(RefKind::Require, &lit, Span::new(t.start, rp.end), flags);
        if t.depth == 0
            && let Some(module) = self.literal_str(&lit)
        {
            self.bind(
                &name,
                Imported::Namespace,
                &module,
                BindingSource::Require,
                type_only,
            );
        }
        self.consume(i + 6);
    }

    // ---- export ----

    fn on_export(&mut self, t: Token) {
        let mut i = 0;
        let mut type_only = false;
        let first = self.ts.peek(0);
        if first.kind == TokKind::Ident
            && first.kw == Kw::Type
            && matches!(self.ts.peek(1).kind, TokKind::LBrace | TokKind::Star)
        {
            type_only = true;
            i = 1;
        }
        match self.ts.peek(i).kind {
            TokKind::Star => {
                i += 1;
                if self.ts.peek(i).kw == Kw::As {
                    i += 2;
                }
            }
            TokKind::LBrace => {
                i += 1;
                loop {
                    match self.ts.peek(i).kind {
                        TokKind::RBrace => {
                            i += 1;
                            break;
                        }
                        TokKind::Comma | TokKind::Ident | TokKind::Str => i += 1,
                        _ => return,
                    }
                }
            }
            _ => return,
        }
        let from = self.ts.peek(i);
        let lit = self.ts.peek(i + 1);
        if from.kind != TokKind::Ident || from.kw != Kw::From || !self.is_literal(&lit) {
            return;
        }
        let flags = if type_only {
            RefFlags::TYPE_ONLY
        } else {
            RefFlags::empty()
        };
        self.emit_literal(RefKind::ReExport, &lit, Span::new(t.start, lit.end), flags);
        self.consume(i + 2);
    }

    // ---- require ----

    fn on_require(&mut self, t: Token) {
        if matches!(
            t.prev_kw,
            Kw::Function | Kw::Const | Kw::Let | Kw::Var | Kw::Class
        ) {
            // `function require(...)`, `var require = ...`: shadows `require` in the
            // enclosing scope, unless it is `const require = createRequire(...)`.
            if t.prev_kw != Kw::Class {
                match self.create_require_init() {
                    None => self.push_shadow(t.depth),
                    // The file's own `require` under its usual name, for good.
                    Some(true) if t.prev_kw == Kw::Const => {}
                    // A `require` that may search from another place.
                    Some(_) => self.add_loader("require", false, true, t.depth),
                }
            }
            return;
        }
        let aliased = self.shadow_note();
        // `const require = createRequire(elsewhere)`: the calls are resolved from
        // there.
        let elsewhere = self
            .loaders
            .iter()
            .rev()
            .find(|l| l.local == "require")
            .is_some_and(|l| l.elsewhere);
        let n = self.ts.peek(0);
        match n.kind {
            TokKind::LParen if elsewhere => {
                // `require('fs')`: a builtin module is the same for every `require`
                // function, and what is bound to it is tracked as usual.
                let cap = self.capture_arg(0, 0);
                let lit = match cap.close {
                    Some(c) if c == cap.first + 1 => {
                        let a = self.ts.peek(cap.first);
                        self.literal_str(&a)
                    }
                    _ => None,
                };
                match lit {
                    Some(m) if api::is_builtin_module(&m) => {
                        if let Some(module) =
                            self.call(t, RefKind::Require, 0, RefFlags::empty(), None)
                        {
                            self.after_require(t, module);
                        }
                    }
                    // A tracked package (`fs-extra`): found from the other place, bound
                    // as usual.
                    Some(m) if api::is_tracked_module(api::normalize_module(&m)) => {
                        self.load_site(t, 0, RefKind::Require, RefFlags::empty(), true);
                        if let Some(c) = cap.close {
                            self.consume(c + 1);
                            self.after_require(t, m);
                        }
                    }
                    _ => self.load_site(t, 0, RefKind::Require, RefFlags::empty(), true),
                }
            }
            TokKind::LParen => {
                if let Some(module) = self.call(t, RefKind::Require, 0, RefFlags::empty(), aliased)
                {
                    self.after_require(t, module);
                }
            }
            TokKind::Dot => {
                let p = self.ts.peek(1);
                if self.text(&p) == b"resolve" && self.ts.peek(2).kind == TokKind::LParen {
                    self.resolve_call(t, 2, elsewhere, aliased);
                }
            }
            TokKind::Comma | TokKind::RParen
                if matches!(self.prev_byte(t.start), Some(b'(' | b',')) =>
            {
                self.require_param(t);
            }
            _ => {
                // `const r = require`: an aliased loader.
                if let Some(Target::Name(name)) = self.assigned_target(t.start) {
                    self.add_loader(name, true, elsewhere, t.depth);
                }
            }
        }
    }

    fn on_module(&mut self, t: Token) {
        let p1 = self.ts.peek(1);
        if self.ts.peek(0).kind == TokKind::Dot && self.ts.peek(2).kind == TokKind::LParen {
            match self.text(&p1) {
                b"require" => {
                    self.call(t, RefKind::Require, 2, RefFlags::empty(), None);
                }
                b"createRequire" => self.on_create_require(t, 2),
                _ => {}
            }
        }
    }

    /// `callee.resolve(x, ...)` of a `require` function; `lp` is the peek index
    /// of `(`. With a second argument (`{ paths }`: where the search
    /// starts), or for a `require` that searches from another place, the
    /// whole call goes to the value-flow engine.
    fn resolve_call(&mut self, t: Token, lp: usize, elsewhere: bool, aliased: Option<String>) {
        if elsewhere || self.more_args(lp) {
            self.load_site(t, lp, RefKind::Require, RefFlags::empty(), false);
        } else {
            self.call(t, RefKind::Require, lp, RefFlags::empty(), aliased);
        }
    }

    /// Whether the call whose `(` is at peek index `lp` has more than one
    /// argument.
    fn more_args(&mut self, lp: usize) -> bool {
        let cap = self.capture_arg(lp, 0);
        cap.close.is_none() && !cap.truncated && cap.end > cap.first
    }

    /// The peek index of the `)` closing the call whose `(` is at peek index
    /// `lp`.
    fn call_close(&mut self, lp: usize) -> Option<usize> {
        let d = self.ts.peek(lp).depth;
        let mut i = lp + 1;
        loop {
            if i - lp > args::MAX_ARG_TOKENS {
                return None;
            }
            let t = self.ts.peek(i);
            match t.kind {
                TokKind::RParen if t.depth == d => return Some(i),
                TokKind::Eof => return None,
                _ if t.depth <= d => return None,
                _ => {}
            }
            i += 1;
        }
    }

    /// A call whose callee decides where its first argument is resolved from
    /// ([`crate::flow::Ask::Load`]): `req(x)` / `req.resolve(x)` of a `require`
    /// function that is not known to be the file's own,
    /// `createRequire(a)(x)`, `require.resolve(x, { paths })`. `t` starts
    /// the callee and `lp` is the peek index of the `(` of the call. The
    /// token pass does not know from where such a call resolves,
    /// so it records an unknown load; the value-flow engine evaluates the
    /// callee and the arguments together and replaces it with the path the
    /// call resolves. Nothing is consumed: the arguments may hold
    /// references of their own.
    fn load_site(
        &mut self,
        t: Token,
        lp: usize,
        kind: RefKind,
        mut flags: RefFlags,
        requires: bool,
    ) {
        if t.in_try() {
            flags |= RefFlags::OPTIONAL;
        }
        let close = self.call_close(lp);
        let last = self.ts.peek(close.unwrap_or(lp));
        let span = Span::new(t.start, last.end.max(t.end));
        let mut detail = evaluator::reason_text(self.src, &[t, last]).into_string();
        if close.is_none() {
            detail.push('…');
        }
        let site = Site {
            kind,
            rule: BaseRule::Module,
            span,
            arg_span: span,
            flags,
            requires,
        };
        let start = self.ex.refs.len();
        let out = Outcome {
            val: JsVal::Unknown(detail.clone().into()),
            env: false,
            complete: false,
            guess: None,
            named: false,
            flow: false,
        };
        self.emit_outcome(site, out, &detail, None);
        if close.is_some() {
            self.note_site(start, site, span, Wants::Load, detail, None);
        }
    }

    /// After `require('<module>')` (consumed):
    /// `require('fs').readFileSync(...)`, `require('module').
    /// createRequire(...)`, or a binding `x = require('fs')[.member]`.
    fn after_require(&mut self, t: Token, module: Cow<'s, str>) {
        let m = api::normalize_module(&module);
        if !api::is_tracked_module(m) {
            return;
        }
        let (chain, i) = self.member_chain(0);
        if !chain.is_empty() && self.ts.peek(i).kind == TokKind::LParen {
            let path = chain.join(".");
            if m == "module" && path == "createRequire" {
                self.on_create_require(t, i);
            } else if m == "vm" && api::vm_evaluates(&path, CallStyle::Call) {
                self.eval_like(t, i, &format!("vm.{path}"), "");
            } else if let Some(e) = api::lookup(Some(m), &path, CallStyle::Call) {
                self.api_call(t, e, i);
            }
            return;
        }
        if chain.len() > 1 {
            return;
        }
        let member = chain.first().map(|s| Cow::Borrowed(*s));
        match self.assigned_target(t.start) {
            Some(Target::Name(name)) => self.add_api(name, module, member, t.depth),
            Some(Target::Destructure(entries)) if member.is_none() => {
                for (key, local) in entries {
                    self.add_api(local, module.clone(), Some(Cow::Borrowed(key)), t.depth);
                }
            }
            _ => {}
        }
    }

    /// `createRequire(a)` / `module.createRequire(a)`; `t` starts the callee
    /// and `lp` is the peek index of its `(`. Node gives a `require`
    /// function that resolves as a module at `a` would. When `a` is
    /// literally the file (`import.meta.url`, `__filename`) that
    /// is the file's own `require`: `const X = createRequire(a)` gives `X`
    /// `require` semantics and `createRequire(a)(x)` /
    /// `createRequire(a).resolve(x)` are calls of `require`. Any other `a`
    /// is a value, and a name that is not a `const` can be bound again: the
    /// calls go to the value-flow engine as a whole
    /// ([`Recognizer::load_site`]), which knows the bindings and from where
    /// they resolve.
    fn on_create_require(&mut self, t: Token, lp: usize) {
        let own = self.names_this_file(lp);
        // (An argument too long to capture: no inline call is looked for, and a name
        // bound to the result is a value.)
        if let Some(close) = self.call_close(lp) {
            // The argument says where the function searches from: a
            // `new URL(x, import.meta.url)` there locates no file that is read.
            let end = self.ts.peek(close).end;
            self.url_suppress_until = self.url_suppress_until.max(end);
            let next = self.ts.peek(close + 1);
            if next.kind == TokKind::LParen {
                // `createRequire(a)(x)`
                if own {
                    self.call(t, RefKind::Require, close + 1, RefFlags::empty(), None);
                } else {
                    self.load_site(t, close + 1, RefKind::Require, RefFlags::empty(), true);
                }
                return;
            }
            if next.kind == TokKind::Dot && self.ts.peek(close + 3).kind == TokKind::LParen && {
                let p = self.ts.peek(close + 2);
                self.text(&p) == b"resolve"
            } {
                // `createRequire(a).resolve(x)`
                self.resolve_call(t, close + 3, !own, None);
                return;
            }
        }
        if let Some(Target::Name(name)) = self.assigned_target(t.start) {
            if name == "require" {
                // Registered at its declaration ([`Recognizer::on_require`]); an
                // assignment to the name binds it again.
                if !self.declares(t.start, name) {
                    self.add_loader(name, false, true, t.depth);
                }
                return;
            }
            let fixed = own && self.declares_const(t.start, name);
            self.add_loader(name, false, !fixed, t.depth);
        }
    }

    /// Whether `<keyword> name =` precedes the expression starting at `start`,
    /// for one of `keywords`.
    fn declared_with(&self, start: u32, name: &str, keywords: &[&[u8]]) -> bool {
        let src = self.src;
        let mut i = skip_ws_back(src, start as usize);
        if i == 0 || src[i - 1] != b'=' {
            return false;
        }
        i = skip_ws_back(src, i - 1);
        if i < name.len() || &src[i - name.len()..i] != name.as_bytes() {
            return false;
        }
        keywords.contains(&ident_before(src, i - name.len()))
    }

    /// Whether the expression starting at `start` initializes `const name`.
    fn declares_const(&self, start: u32, name: &str) -> bool {
        self.declared_with(start, name, &[b"const"])
    }

    /// Whether the expression starting at `start` initializes a declaration of
    /// `name`.
    fn declares(&self, start: u32, name: &str) -> bool {
        self.declared_with(start, name, &[b"const", b"let", b"var"])
    }

    /// Whether the first argument of the call whose `(` is at peek index `lp`
    /// is literally the file itself: `import.meta.url`,
    /// `import.meta.filename` or `__filename`.
    fn names_this_file(&mut self, lp: usize) -> bool {
        let cap = self.capture_arg(lp, 0);
        if cap.truncated || cap.close.is_none() {
            return false;
        }
        let src = self.src;
        match self.ts.window(cap.first, cap.end) {
            [f] => f.text(src) == b"__filename",
            [i, d1, m, d2, u] => {
                i.kw == Kw::Import
                    && d1.kind == TokKind::Dot
                    && m.text(src) == b"meta"
                    && d2.kind == TokKind::Dot
                    && matches!(u.text(src), b"url" | b"filename")
            }
            _ => false,
        }
    }

    /// Whether the `require` just declared is initialized with
    /// `createRequire(...)`, and if so, whether with the file itself
    /// ([`Recognizer::names_this_file`]).
    fn create_require_init(&mut self) -> Option<bool> {
        if self.ts.peek(0).kind != TokKind::Assign {
            return None;
        }
        let at = (1..8).find(|&i| {
            let k = self.ts.peek(i);
            self.text(&k) == b"createRequire"
        })?;
        Some(self.ts.peek(at + 1).kind == TokKind::LParen && self.names_this_file(at + 1))
    }

    /// Registers `local` as a name with `require` semantics. `elsewhere`: it
    /// was made by `createRequire(a)` for an `a` that is not literally the
    /// file ([`Loader::elsewhere`]).
    fn add_loader(&mut self, local: &'s str, aliased: bool, elsewhere: bool, depth: u32) {
        if self.loaders.len() >= MAX_LOADERS {
            return;
        }
        self.ts.lx.add_trigger_word(local.as_bytes());
        self.max_scope = self.max_scope.max(depth);
        self.loaders.push(Loader {
            local,
            aliased,
            elsewhere,
            depth,
        });
    }

    fn push_shadow(&mut self, depth: u32) {
        self.max_scope = self.max_scope.max(depth);
        if self.shadows.last() != Some(&depth) {
            self.shadows.push(depth);
        }
    }

    /// The `AliasedLoader` detail for a `require` call in a shadowing scope.
    fn shadow_note(&self) -> Option<String> {
        (!self.shadows.is_empty())
            .then(|| "`require` is shadowed by a local declaration (bundle wrapper?)".to_owned())
    }

    /// `require` as a parameter: `function (module, exports, require) {`.
    /// Registers a shadow for the function body.
    fn require_param(&mut self, t: Token) {
        // `if (require) {...}` is a condition, not a parameter list.
        if self.prev_byte(t.start) == Some(b'(') {
            let open = skip_ws_back(self.src, t.start as usize) - 1;
            let kw = ident_before(self.src, open);
            if matches!(kw, b"if" | b"while" | b"for" | b"with" | b"switch") {
                return;
            }
        }
        let d = t.depth;
        let mut i = 0;
        loop {
            if i > 128 {
                return;
            }
            let k = self.ts.peek(i);
            match k.kind {
                TokKind::Eof => return,
                TokKind::RParen if k.depth + 1 == d => break,
                _ if k.depth < d => return,
                _ => {}
            }
            i += 1;
        }
        let mut j = i + 1;
        if self.ts.peek(j).kind == TokKind::Arrow {
            j += 1;
        }
        if self.ts.peek(j).kind != TokKind::LBrace {
            return;
        }
        self.consume(j + 1);
        self.push_shadow(d);
        self.ts.reset_low();
    }

    /// `.a.b` chain starting at peek index `i`: the names and the index after
    /// the chain.
    fn member_chain(&mut self, mut i: usize) -> (Vec<&'s str>, usize) {
        let mut chain = Vec::new();
        while chain.len() < 4
            && self.ts.peek(i).kind == TokKind::Dot
            && self.ts.peek(i + 1).kind == TokKind::Ident
        {
            let n = self.ts.peek(i + 1);
            let Some(s) = self.ident_str(&n) else { break };
            chain.push(s);
            i += 2;
        }
        (chain, i)
    }

    /// Whether the token at `start` begins a statement or an object or class
    /// member, after optional member modifiers (`async`, `static`,
    /// `public`, ...).
    fn at_member_start(&self, start: u32) -> bool {
        let mut at = start as usize;
        for _ in 0..4 {
            let i = skip_ws_back(self.src, at);
            match i.checked_sub(1).map(|j| self.src[j]) {
                None | Some(b'{' | b',' | b';' | b'}') => return true,
                Some(b'*') => at = i - 1,
                Some(_) => {
                    let w = ident_before(self.src, i);
                    if !matches!(
                        w,
                        b"async"
                            | b"static"
                            | b"public"
                            | b"private"
                            | b"protected"
                            | b"override"
                            | b"abstract"
                    ) {
                        return false;
                    }
                    at = i - w.len();
                }
            }
        }
        false
    }

    /// The last non-whitespace byte before `start`.
    fn prev_byte(&self, start: u32) -> Option<u8> {
        let i = skip_ws_back(self.src, start as usize);
        i.checked_sub(1).map(|j| self.src[j])
    }

    /// What `<expr starting at start>` is assigned to: `name = ` or `{a, b: c}
    /// = `, scanning the source bytes backwards (comments in between are
    /// not supported).
    fn assigned_target(&self, start: u32) -> Option<Target<'s>> {
        let src = self.src;
        let mut i = skip_ws_back(src, start as usize);
        if i == 0 || src[i - 1] != b'=' {
            return None;
        }
        i -= 1;
        if i > 0 && b"=!<>+-*/%&|^?".contains(&src[i - 1]) {
            return None;
        }
        i = skip_ws_back(src, i);
        if i > 0 && src[i - 1] == b'}' {
            return destructure_back(src, i - 1).map(Target::Destructure);
        }
        let end = i;
        while i > 0 && is_ident_byte(src[i - 1]) {
            i -= 1;
        }
        if i == end || src[i].is_ascii_digit() {
            return None;
        }
        if self.prev_byte(i as u32) == Some(b'.') {
            return None; // `a.b = require(...)`
        }
        std::str::from_utf8(&src[i..end]).ok().map(Target::Name)
    }

    /// Captures argument number `n` of the call whose `(` is at peek index
    /// `lp`.
    fn capture_arg(&mut self, lp: usize, n: usize) -> ArgCapture {
        let d = self.ts.peek(lp).depth;
        let mut first = lp + 1;
        let mut index = 0;
        let mut i = first;
        loop {
            if i - first > args::MAX_ARG_TOKENS {
                return ArgCapture {
                    first,
                    end: i,
                    close: None,
                    last: i - 1,
                    truncated: true,
                };
            }
            let t = self.ts.peek(i);
            match t.kind {
                TokKind::RParen if t.depth == d => {
                    if index < n {
                        first = i;
                    }
                    return ArgCapture {
                        first,
                        end: i,
                        close: Some(i),
                        last: i,
                        truncated: false,
                    };
                }
                TokKind::Comma if t.depth == d + 1 && index < n => {
                    index += 1;
                    first = i + 1;
                }
                TokKind::Comma if t.depth == d + 1 => {
                    // Later arguments do not matter (`import('x', {with: ...})`).
                    return ArgCapture {
                        first,
                        end: i,
                        close: None,
                        last: i.saturating_sub(1).max(first),
                        truncated: false,
                    };
                }
                TokKind::Eof => {
                    return ArgCapture {
                        first,
                        end: i,
                        close: None,
                        last: i.saturating_sub(1),
                        truncated: true,
                    };
                }
                _ if t.depth <= d => {
                    // Unbalanced input: the call never closes.
                    return ArgCapture {
                        first,
                        end: i,
                        close: None,
                        last: i.saturating_sub(1),
                        truncated: true,
                    };
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// A call `callee(arg)`; `t` is the first callee token and `lp` the peek
    /// index of `(`. `aliased`: the `AliasedLoader` detail when the callee
    /// is not the real `require`. Returns the specifier when the argument
    /// was a plain string literal (the call is then consumed).
    fn call(
        &mut self,
        t: Token,
        kind: RefKind,
        lp: usize,
        mut flags: RefFlags,
        aliased: Option<String>,
    ) -> Option<Cow<'s, str>> {
        let cap = self.capture_arg(lp, 0);
        if cap.end == cap.first {
            return None; // no argument
        }
        let a0 = self.ts.peek(cap.first);
        // `{ require(x) { ... } }`: a method definition. A parameter is never a
        // literal, and `require(x)\n{` is a call followed by a block (ASI).
        if let Some(c) = cap.close
            && !(c == cap.first + 1 && self.is_literal(&a0))
        {
            let next = self.ts.peek(c + 1);
            if next.kind == TokKind::LBrace && next.flags & NEWLINE_BEFORE == 0 {
                return None;
            }
            // `{ require({ fallback = true } = {}): T { ... } }`: a TypeScript method with
            // a return type. A call is never followed by `:` when it starts a
            // statement or an object or class member (`c ? require(x) : y`
            // follows a `?`).
            if self.dialect.ts && next.kind == TokKind::Colon && self.at_member_start(t.start) {
                return None;
            }
        }
        // `require(id: string): any`: a TypeScript signature.
        if a0.kind == TokKind::Ident {
            let a1 = self.ts.peek(cap.first + 1);
            if a1.kind == TokKind::Colon
                || (a1.kind == TokKind::Question
                    && self.ts.peek(cap.first + 2).kind == TokKind::Colon)
            {
                return None;
            }
        }
        // AMD `require(['a', 'b'], function (a, b) {})`.
        if kind == RefKind::Require && a0.kind == TokKind::LBracket {
            return None;
        }
        if t.in_try() {
            flags |= RefFlags::OPTIONAL;
        }
        if kind == RefKind::DynamicImport && self.dialect.ts && self.ts_type_import(t, &cap) {
            flags |= RefFlags::TYPE_ONLY;
        }
        let last = self.ts.peek(cap.last);
        let span = Span::new(t.start, last.end.max(t.end));
        let lit = (!cap.truncated)
            .then(|| single_literal(self.ts.window(cap.first, cap.end)))
            .flatten();
        let mut literal = None;
        if let Some(lt) = lit {
            let flags = flags | self.base_flags();
            let mut r = match args::literal(self.src, &lt) {
                Some(Ok(s)) => {
                    literal = Some(s.clone());
                    RawRef::new(kind, Specifier::Literal(s), span)
                }
                Some(Err(u)) => RawRef::new(kind, Specifier::Unknown, span).with_uncertainty(u),
                None => return None,
            };
            if let Some(a) = aliased {
                r.uncertainty = Some(Uncertainty::new(
                    UncertaintyKind::AliasedLoader,
                    lt.span(),
                    a,
                ));
            }
            self.ex.refs.push(r.with_flags(flags));
        } else {
            // `require.resolve(x)` and `req.resolve(x)` share the kind and load nothing.
            let requires = kind == RefKind::Require
                && !(lp >= 1 && {
                    let callee = self.ts.peek(lp - 1);
                    self.text(&callee) == b"resolve"
                });
            self.emit_arg(kind, BaseRule::Module, &cap, span, flags, aliased, requires);
        }
        if literal.is_some()
            && let Some(c) = cap.close
        {
            // `callee ( 'x' )`: nothing else to see.
            self.consume(c + 1);
        } else {
            // Leave the argument tokens to the main loop (they may contain references).
            self.consume(lp);
        }
        literal
    }

    /// Whether the TypeScript `import(...)` at `t` is a type (see the module
    /// docs).
    fn ts_type_import(&mut self, t: Token, cap: &ArgCapture) -> bool {
        if t.prev_kw == Kw::Typeof || t.start < self.type_region_end {
            return true;
        }
        if t.prev_kw == Kw::Await {
            return false;
        }
        let type_pos = t.flags & TYPE_POS != 0;
        let Some(c) = cap.close else {
            return type_pos;
        };
        let (chain, i) = self.member_chain(c + 1);
        let next = self.ts.peek(i);
        let called = matches!(
            next.kind,
            TokKind::LParen
                | TokKind::OptChain
                | TokKind::Template
                | TokKind::TemplateHead
                | TokKind::Assign
        );
        if !chain.is_empty() {
            // `import('x').A.B`: a qualified type name; `.then(...)` is a call.
            return !called;
        }
        next.kind == TokKind::Lt || (type_pos && !called)
    }

    /// Evaluates the captured argument and pushes the references. `requires`:
    /// see [`Site::requires`].
    #[allow(clippy::too_many_arguments)]
    fn emit_arg(
        &mut self,
        kind: RefKind,
        rule: BaseRule,
        cap: &ArgCapture,
        span: Span,
        flags: RefFlags,
        aliased: Option<String>,
        requires: bool,
    ) {
        let toks = self.ts.window(cap.first, cap.end);
        let (arg_span, detail) = args::arg_detail(self.src, toks, cap.truncated);
        let out = if cap.truncated {
            Outcome {
                val: JsVal::Unknown(detail.clone().into()),
                env: false,
                complete: false,
                guess: None,
                named: false,
                flow: false,
            }
        } else {
            self.eval(toks)
        };
        let site = Site {
            kind,
            rule,
            span,
            arg_span,
            flags,
            requires,
        };
        // The value-flow engine evaluates every argument that is not one string
        // literal: the token evaluator reads names without knowing scopes (a
        // parameter called `path` or `dir` shadows what it thinks the name is),
        // so even a complete token result is only checked there. A truncated
        // argument still starts at the right place: the engine sees the whole
        // expression.
        let literal = !cap.truncated
            && matches!(toks, [t] if matches!(t.kind, TokKind::Str | TokKind::Template));
        let wants = match (literal, cap.truncated) {
            (true, _) => Wants::Nothing,
            (false, false) => Wants::Value,
            (false, true) => Wants::ValueFrom,
        };
        let start = self.ex.refs.len();
        self.emit_outcome(site, out, &detail, aliased.clone());
        self.note_site(start, site, arg_span, wants, detail, aliased);
    }

    /// Remembers a site for the value-flow pass ([`Recognizer::flow_pass`]).
    fn note_site(
        &mut self,
        start: usize,
        site: Site,
        expr: Span,
        wants: Wants,
        detail: String,
        aliased: Option<String>,
    ) {
        self.ex.site_args.push(expr);
        if self.sites.len() >= MAX_FLOW_SITES {
            // Too many sites to hand to the engine: this one stays unchecked.
            if wants != Wants::Nothing {
                for r in &mut self.ex.refs[start..] {
                    unchecked(r, site.arg_span, &detail);
                }
            }
            return;
        }
        self.sites.push(FlowSite {
            refs: start..self.ex.refs.len(),
            site,
            expr,
            wants,
            detail,
            aliased,
        });
    }

    /// The value-flow pass (tier 2, [`crate::flow`]): when the token evaluator
    /// left a site unresolved, guessed, or relied on a top-level binding by
    /// name (it does not know scopes), the file is parsed into a real AST
    /// and those sites are evaluated again with scoped bindings,
    /// assignments, destructuring, loops, local functions and same-file
    /// call sites. The site's references are replaced by the engine's result.
    /// Files the parser rejects keep the token results.
    fn flow_pass(&mut self) {
        /// `TURBO_TRACE_DEBUG_FLOW=0` turns the pass off (for A/B measurements,
        /// like `TURBO_TRACE_DEBUG_BARRELS`; graphs built either way
        /// are not interchangeable).
        static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            turborepo_trace_core::debug::var_os("FLOW").is_none_or(|v| v != "0")
        });
        if !(self.sites.iter().any(|s| s.wants.ask().is_some()) || (self.env_facts && *ENABLED)) {
            return;
        }
        let wanted: Vec<(Span, crate::flow::Ask)> = self
            .sites
            .iter()
            .filter_map(|s| Some((s.expr, s.wants.ask()?)))
            .collect();
        static FLOW: turborepo_trace_core::timing::Span =
            turborepo_trace_core::timing::Span::new("trace: value flow (inside extract)");
        let t = FLOW.enter();
        // The engine is off or declines the file (it does not parse, is too large or
        // too deep, uses `eval` or `with`): no site has a value, and the file
        // has no environment facts.
        let flow = if *ENABLED {
            crate::flow::evaluate(
                self.src,
                self.dialect,
                self.file_name,
                &wanted,
                self.env_facts,
            )
        } else {
            None
        };
        let vals = match flow {
            Some(f) => {
                self.ex.facts = f.env;
                f.sites
            }
            None => vec![None; wanted.len()],
        };
        drop(t);
        let sites = std::mem::take(&mut self.sites);
        let old = std::mem::take(&mut self.ex.refs);
        let mut old = old.into_iter().enumerate().peekable();
        let mut vals = vals.into_iter();
        for s in sites {
            let asked = s.wants != Wants::Nothing;
            let v = if asked { vals.next().flatten() } else { None };
            // References before this site (and this site's own, if it keeps them).
            let upto = if v.is_some() {
                s.refs.start
            } else {
                s.refs.end
            };
            while let Some((i, mut r)) = old.next_if(|(i, _)| *i < upto) {
                // A site the engine was asked about and did not answer keeps the token
                // result, which nothing has checked: the references stay, with an open
                // record.
                if v.is_none() && asked && i >= s.refs.start {
                    unchecked(&mut r, s.site.arg_span, &s.detail);
                }
                self.ex.refs.push(r);
            }
            let Some(v) = v else { continue };
            while old.next_if(|(i, _)| *i < s.refs.end).is_some() {}
            let asked_load = s.wants == Wants::Load;
            let (uses, asked_from) = (v.uses, v.from);
            let (site, from) = (s.site, self.ex.refs.len());
            self.emit_outcome(
                s.site,
                Outcome {
                    val: v.val,
                    env: false,
                    complete: true,
                    guess: v.guess,
                    named: false,
                    flow: true,
                },
                &s.detail,
                s.aliased,
            );
            // A load asked of its callee that stays unknown: what is not known is where the
            // `require` function searches (or what it is asked for there), and no other
            // file's value can be put in its place: the value-flow facts make such a site
            // no sink (`SiteIn::load`). So its record does not say `param`, `call` or
            // `imported`, the origins of a value that callers or a callee decide: a file
            // would be a binding site for a record that binding cannot close, and its
            // facts would be computed for nothing on every graph without a facts cache
            // (after a restore). The detail keeps the text of the call.
            //
            // A template of origin `call` (the request is the result of an imported
            // function, which may be an environment helper) is found by the origin of its
            // record, so it says the origin the record ends with. When another alternative
            // of the load has a record of the same kind at the same place (a parameter, a
            // value nothing knows), the two cannot be told apart, and what resolves the
            // one must not close the other: the template says that its record is shared,
            // and the environment layer adds what it names and leaves the records alone.
            let mut shared = false;
            if asked_load {
                shared = self.ex.refs[from..].iter().any(|r| {
                    matches!(&r.uncertainty, Some(u) if u.kind == UncertaintyKind::ComputedSpecifier && u.origin != Origin::Call)
                });
                for r in &mut self.ex.refs[from..] {
                    if let Some(u) = &mut r.uncertainty
                        && matches!(u.origin, Origin::Param | Origin::Call | Origin::Imported)
                    {
                        u.origin = Origin::Unknown;
                    }
                }
            }
            // The alternatives that stay open with origin `env` or `call`, as templates
            // the graph store resolves against the repository's assignments. A load asked
            // of its callee says where it searches from: a relative value is resolved
            // from there, and with an unknown place it resolves nothing. Such a load has
            // no template that could not be evaluated anyway (the place itself is the
            // unknown): nothing would look for it.
            for (origin, value) in uses {
                if asked_load && !evaluable(&value) {
                    continue;
                }
                let (origin, shared) = match origin {
                    Origin::Call if asked_load => (Origin::Unknown, shared),
                    o => (o, false),
                };
                let at = site.arg_span;
                self.ex.facts.push(turborepo_trace_core::env::EnvFact::Use {
                    span: [at.start, at.end],
                    line: 1 + memchr::memchr_iter(
                        b'\n',
                        &self.src[..(at.start as usize).min(self.src.len())],
                    )
                    .count() as u32,
                    kind: RefKind::ALL
                        .iter()
                        .position(|k| *k == site.kind)
                        .unwrap_or(0) as u8,
                    origin: Origin::ALL.iter().position(|o| *o == origin).unwrap_or(0) as u8,
                    value,
                    from: if asked_load {
                        asked_from.clone()
                    } else {
                        turborepo_trace_core::env::EnvFrom::File
                    },
                    shared,
                });
            }
            // A record whose value other files decide: callers (a parameter), a called
            // function or an imported binding.
            let param = self.ex.refs[from..]
                .iter()
                .any(|r| matches!(&r.uncertainty, Some(u) if matches!(u.origin, Origin::Param | Origin::Call | Origin::Imported)));
            self.ex.flow_sites.push(crate::flow::facts::SiteIn {
                kind: site.kind,
                rule: site.rule,
                expr: s.expr,
                open_end: s.wants == Wants::ValueFrom,
                load: s.wants == Wants::Load,
                arg_span: site.arg_span,
                span: site.span,
                param,
            });
        }
        self.ex.refs.extend(old.map(|(_, r)| r));
    }

    /// Pushes the references for an evaluated value: one per distinct base with
    /// its alternatives merged, plus one unknown reference per origin of
    /// the alternatives that cannot be lowered (unknown values, leading
    /// wildcards), each with an uncertainty record carrying that origin
    /// ([`turborepo_trace_core::Origin`]). Alternatives that can never be a
    /// file (the empty string) or lie outside the repository (temporary
    /// directories) produce nothing.
    fn emit_outcome(&mut self, site: Site, out: Outcome, detail: &str, aliased: Option<String>) {
        // A module loaded from the temporary directory was generated at runtime (a
        // bundle, a compiled addon, a script written by the test) from files
        // this site cannot name: the load keeps a record. A file read from
        // there is just the program's own data.
        let generated = matches!(
            site.kind,
            RefKind::Require | RefKind::DynamicImport | RefKind::Worker | RefKind::ChildProcess
        ) && evaluator::in_tmp(&out.val);
        // A `require` whose specifier ends with `.json` in every alternative reads a
        // data file: Node parses a `.json` file, nothing in it runs and it
        // loads nothing further. Where such a specifier cannot be said, the
        // record is that of a read with a computed path, not of a module load
        // (which would bring dependencies of its own). A specifier that can be
        // said (a literal, a known directory prefix) is a `require` reference
        // as before, and so is one through an alias of `require` or
        // of `require.resolve`, which reads nothing. Only a value of the value-flow
        // engine counts: the token evaluator reads names without scopes, so the
        // `.json` it sees may belong to another binding of that name.
        let kind = site.kind;
        let open_kind = if site.requires
            && out.flow
            && aliased.is_none()
            && !generated
            && evaluator::ends_with_json(&out.val)
        {
            RefKind::FsRead
        } else {
            kind
        };
        let mut lowered = evaluator::lower(out.val, site.rule);
        if generated {
            lowered.origins.push((
                Origin::Data,
                "a module generated in the temporary directory".to_owned(),
            ));
        }
        let mut flags = site.flags | self.base_flags();
        if lowered.count > 1 && !lowered.leaves.is_empty() {
            flags |= RefFlags::CONDITIONAL;
        }
        let mut groups: Vec<(Base, Vec<Value>, bool)> = Vec::new();
        for leaf in lowered.leaves {
            let spec = if out.flow {
                strip_marks(leaf.spec)
            } else {
                leaf.spec
            };
            match groups.iter_mut().find(|g| g.0 == leaf.base) {
                Some(g) => {
                    g.1.push(spec);
                    g.2 |= leaf.cwd;
                }
                None => groups.push((leaf.base, vec![spec], leaf.cwd)),
            }
        }
        let uncertain = |kind, d: &str, o: Origin| {
            Some(Uncertainty::new(kind, site.arg_span, d).with_origin(o))
        };
        // The record of an unknown alternative with origin `o`.
        let open = |o: &(Origin, String)| -> Option<Uncertainty> {
            if let Some(a) = &aliased {
                return uncertain(UncertaintyKind::AliasedLoader, a, o.0);
            }
            let d = if o.1.is_empty() {
                Cow::Borrowed(detail)
            } else {
                Cow::Owned(format!("{detail}; {}", o.1))
            };
            if o.0 == Origin::Env || (out.env && !out.flow) {
                return uncertain(UncertaintyKind::EnvDependent, &d, Origin::Env);
            }
            uncertain(UncertaintyKind::ComputedSpecifier, &d, o.0)
        };
        let before = self.ex.refs.len();
        let mut collapsed = false;
        for (base, specs, cwd) in groups {
            let spec = merge_specs(specs);
            // Collapsing many alternatives can produce a leading wildcard.
            if matches!(&spec, Specifier::Unknown)
                || matches!(&spec, Specifier::Pattern(p) if matches!(p.first(), Some(turborepo_trace_core::Part::Wild { .. })))
            {
                collapsed = true;
                continue;
            }
            let u = if let Some(a) = &aliased {
                uncertain(UncertaintyKind::AliasedLoader, a, Origin::Unknown)
            } else if out.env && !out.flow {
                uncertain(UncertaintyKind::EnvDependent, detail, Origin::Env)
            } else {
                let mut reasons: Vec<String> = Vec::new();
                spec_reasons(&spec, &mut reasons);
                if let Some(g) = &out.guess {
                    reasons.push(g.to_string());
                }
                if cwd {
                    match &out.guess {
                        Some(g) => uncertain(
                            UncertaintyKind::CwdDependent,
                            &format!("{detail}; {g}"),
                            Origin::Cwd,
                        ),
                        None => uncertain(UncertaintyKind::CwdDependent, detail, Origin::Cwd),
                    }
                } else if !reasons.is_empty() {
                    uncertain(
                        UncertaintyKind::PartialSpecifier,
                        &reasons.join(", "),
                        Origin::Unknown,
                    )
                } else {
                    None
                }
            };
            let mut r = RawRef::new(site.kind, spec, site.span)
                .with_base(base)
                .with_flags(flags);
            r.uncertainty = u;
            self.ex.refs.push(r);
        }
        // One unknown reference per origin of the alternatives that could not be
        // lowered.
        let mut origins: Vec<(Origin, String)> = Vec::new();
        for o in lowered.origins {
            if !origins.iter().any(|x| x.0 == o.0) {
                origins.push(o);
            }
        }
        if collapsed && !origins.iter().any(|x| x.0 == Origin::Unknown) {
            origins.push((Origin::Unknown, String::new()));
        }
        for o in &origins {
            let mut r = RawRef::new(open_kind, Specifier::Unknown, site.span).with_flags(flags);
            r.uncertainty = open(o);
            self.ex.refs.push(r);
        }
        if self.ex.refs.len() == before && !out.flow {
            // Nothing at all (`require('')`): keep the site visible.
            let mut r = RawRef::new(open_kind, Specifier::Unknown, site.span).with_flags(flags);
            r.uncertainty = open(&(Origin::Unknown, String::new()));
            self.ex.refs.push(r);
        }
    }

    // ---- bindings ----

    /// `const|let|var <pattern> = require('x') | await import('x') [.name] [,
    /// ...]` at the top level. Only peeks; the main loop still sees every
    /// token.
    fn on_decl(&mut self, decl: Token) {
        const MAX_PROPS: usize = 64;
        let mut i = 0;
        loop {
            let mut names: Vec<(Token, Option<Token>)> = Vec::new();
            let pat = self.ts.peek(i);
            let destructured = match pat.kind {
                TokKind::Ident => {
                    i += 1;
                    false
                }
                TokKind::LBrace => {
                    i += 1;
                    loop {
                        let e = self.ts.peek(i);
                        match e.kind {
                            TokKind::RBrace => {
                                i += 1;
                                break;
                            }
                            TokKind::Comma => i += 1,
                            TokKind::Ident | TokKind::Str if names.len() < MAX_PROPS => {
                                if self.ts.peek(i + 1).kind == TokKind::Colon {
                                    let local = self.ts.peek(i + 2);
                                    if local.kind != TokKind::Ident {
                                        return;
                                    }
                                    names.push((local, Some(e)));
                                    i += 3;
                                } else if e.kind == TokKind::Ident {
                                    names.push((e, None));
                                    i += 1;
                                } else {
                                    return;
                                }
                            }
                            _ => return,
                        }
                    }
                    true
                }
                _ => return,
            };
            if self.dialect.ts && self.ts.peek(i).kind == TokKind::Colon {
                // `const A: T = ...`: skip the annotation.
                let mut j = i + 1;
                loop {
                    let k = self.ts.peek(j);
                    if k.kind == TokKind::Assign && k.depth == decl.depth {
                        break;
                    }
                    if j - i > 64
                        || k.kind == TokKind::Eof
                        || (k.depth == decl.depth
                            && matches!(k.kind, TokKind::Semi | TokKind::Comma))
                    {
                        return;
                    }
                    j += 1;
                }
                i = j;
            }
            if self.ts.peek(i).kind != TokKind::Assign {
                return;
            }
            i += 1;
            let init = i;
            if self.ts.peek(i).kw == Kw::Await {
                i += 1;
            }
            let callee = self.ts.peek(i);
            let source = match callee.kw {
                Kw::Require => Some(BindingSource::Require),
                Kw::Import => Some(BindingSource::DynamicImport),
                _ => None,
            };
            let lit = self.ts.peek(i + 2);
            let Some(source) = source.filter(|_| {
                self.ts.peek(i + 1).kind == TokKind::LParen
                    && self.is_literal(&lit)
                    && self.ts.peek(i + 3).kind == TokKind::RParen
            }) else {
                // Not a binding: maybe a constant (or a `let`/`var` initial value).
                if !destructured
                    && let Some(end) = self.const_init(pat, init, decl.kw != Kw::Const)
                    && self.ts.peek(end).kind == TokKind::Comma
                {
                    i = end + 1;
                    continue;
                }
                return;
            };
            let Some(module) = self.literal_str(&lit) else {
                return;
            };
            let mut j = i + 4;
            let mut member = None;
            if self.ts.peek(j).kind == TokKind::Dot && self.ts.peek(j + 1).kind == TokKind::Ident {
                let m = self.ts.peek(j + 1);
                member = self.ident_str(&m);
                j += 2;
            }
            // `const x = require('url').pathToFileURL(__filename).href`: a value computed
            // from the module, not a binding to it.
            if matches!(
                self.ts.peek(j).kind,
                TokKind::Dot
                    | TokKind::OptChain
                    | TokKind::LParen
                    | TokKind::LBracket
                    | TokKind::Template
                    | TokKind::TemplateHead
            ) {
                if !destructured
                    && let Some(end) = self.const_init(pat, init, decl.kw != Kw::Const)
                    && self.ts.peek(end).kind == TokKind::Comma
                {
                    i = end + 1;
                    continue;
                }
                return;
            }
            i = j;
            if destructured {
                if member.is_some() {
                    return;
                }
                for (local, key) in names {
                    let key = key.unwrap_or(local);
                    let imported = if key.kind == TokKind::Str {
                        self.literal_str(&key).map(Imported::Named)
                    } else {
                        self.ident_str(&key)
                            .map(|s| Imported::Named(Cow::Borrowed(s)))
                    };
                    if let Some(imported) = imported {
                        self.bind(&local, imported, &module, source, false);
                    }
                }
            } else {
                let imported =
                    member.map_or(Imported::Namespace, |m| Imported::Named(Cow::Borrowed(m)));
                self.bind(&pat, imported, &module, source, false);
            }
            if self.ts.peek(i).kind != TokKind::Comma {
                return;
            }
            i += 1;
        }
    }

    /// `const NAME = <expr>` at the top level, `start` the peek index of the
    /// initializer: evaluates and remembers the value. Returns the peek
    /// index after the initializer. `mutable`: a `let`/`var` declaration,
    /// remembered in [`Recognizer::vars`].
    fn const_init(&mut self, name_tok: Token, start: usize, mutable: bool) -> Option<usize> {
        let first = self.ts.peek(start);
        let plausible = match first.kind {
            TokKind::Str
            | TokKind::Template
            | TokKind::TemplateHead
            | TokKind::LParen
            | TokKind::LBracket => true,
            TokKind::Ident => {
                !matches!(first.kw, Kw::Function | Kw::Class)
                    && self.ts.peek(start + 1).kind != TokKind::Arrow
            }
            _ => false,
        };
        if !plausible {
            return None;
        }
        let d = name_tok.depth;
        let mut i = start;
        let mut prev = TokKind::Assign;
        loop {
            if i - start > args::MAX_ARG_TOKENS {
                return None;
            }
            let k = self.ts.peek(i);
            if k.kind == TokKind::Eof
                || k.depth < d
                || (k.depth == d && matches!(k.kind, TokKind::Semi | TokKind::Comma))
            {
                break;
            }
            // ASI: a new line starting with a name after a complete value.
            if i > start
                && k.depth == d
                && k.flags & NEWLINE_BEFORE != 0
                && k.kind == TokKind::Ident
                && matches!(
                    prev,
                    TokKind::Ident
                        | TokKind::Str
                        | TokKind::Template
                        | TokKind::TemplateTail
                        | TokKind::Number
                        | TokKind::RParen
                        | TokKind::RBracket
                        | TokKind::RBrace
                )
            {
                break;
            }
            prev = k.kind;
            i += 1;
        }
        let name = self.ident_str(&name_tok)?;
        let out = self.eval(self.ts.window(start, i));
        if out.complete && out.val.is_useful() {
            let map = if mutable {
                &mut self.vars
            } else {
                &mut self.consts
            };
            if map.len() < MAX_CONSTS {
                map.entry(name).or_insert((out.val, out.env));
            }
        }
        Some(i)
    }

    // ---- runtime APIs ----

    /// `new X(...)`: `new Worker`, `new URL`, `new Function`, or a bound
    /// constructor.
    fn on_new(&mut self, t: Token) {
        let n0 = self.ts.peek(0);
        if n0.kind != TokKind::Ident {
            return;
        }
        let Some(root) = self.ident_str(&n0) else {
            return;
        };
        let (chain, lp) = self.member_chain(1);
        if self.ts.peek(lp).kind != TokKind::LParen {
            return;
        }
        if let Some(b) = self.apis.iter().rev().find(|b| b.local == root) {
            let path = b.export_path(&chain);
            let module = b.module.clone();
            if module == "url" && path == "URL" {
                self.url_asset(t, lp);
            } else if module == "vm" && api::vm_evaluates(&path, CallStyle::New) {
                self.eval_like(t, lp, &format!("new vm.{path}"), "");
            } else if let Some(e) = api::lookup(Some(module.as_ref()), &path, CallStyle::New) {
                self.api_call(t, e, lp);
            }
            return;
        }
        if !chain.is_empty() {
            return;
        }
        match root {
            "URL" => self.url_asset(t, lp),
            "Function" => self.eval_like(t, lp, "new Function", ""),
            _ => {
                if let Some(e) = api::lookup(None, root, CallStyle::New) {
                    self.api_call(t, e, lp);
                }
            }
        }
    }

    /// A call of a tracked API whose `(` is at peek index `lp`; `t` starts the
    /// callee.
    fn api_call(&mut self, t: Token, e: &'static ApiEntry, lp: usize) {
        let cap = self.capture_arg(lp, e.arg as usize);
        if cap.end == cap.first {
            return; // missing argument
        }
        // `new Worker(code, { eval: true })`: the first argument is source text that
        // the worker evaluates. No file is loaded, so the site is code
        // evaluation like `eval` and `new Function`, not a load of an unknown
        // file.
        if e.kind == RefKind::Worker && self.worker_evals(lp) {
            self.eval_like(t, lp, "new Worker", ", {eval: true}");
            return;
        }
        let mut flags = e.flags;
        if t.in_try() {
            flags |= RefFlags::OPTIONAL;
        }
        // A `stat` that only asks for the kind of the entry observes names, not
        // contents.
        if api::is_stat(e)
            && let Some(close) = cap.close
            && statuse::kind_only(self.src, t.start as usize, self.ts.peek(close).end as usize)
        {
            flags |= RefFlags::NAMES;
        }
        let last = self.ts.peek(cap.last);
        let span = Span::new(t.start, last.end.max(t.end));
        self.emit_arg(e.kind, e.base, &cap, span, flags, None, false);
        self.url_suppress_until = self.url_suppress_until.max(span.end);
        self.consume(lp);
    }

    /// `new URL(x, import.meta.url)`: an asset next to the file.
    fn url_asset(&mut self, t: Token, lp: usize) {
        if t.start < self.url_suppress_until {
            return;
        }
        let c0 = self.capture_arg(lp, 0);
        let c1 = self.capture_arg(lp, 1);
        if c0.end == c0.first || c1.end == c1.first || c0.truncated || c1.truncated {
            return;
        }
        let sc = self.scope();
        let base =
            evaluator::evaluate_with(&sc, Some(&self.vars), self.ts.window(c1.first, c1.end));
        if !matches!(base.val, JsVal::FileUrl | JsVal::Url(..)) {
            return;
        }
        let input =
            evaluator::evaluate_with(&sc, Some(&self.vars), self.ts.window(c0.first, c0.end));
        let (arg_span, detail) =
            args::arg_detail(self.src, self.ts.window(c0.first, c0.end), false);
        let known_input = matches!(input.val, JsVal::Str(Value::Str(_)));
        let env = input.env;
        let named = input.named || base.named;
        let val = match evaluator::new_url(vec![input.val, base.val], &sc) {
            Some(v) => v,
            None if known_input => return, // `https:`, `data:`, ...
            None => JsVal::Unknown(detail.clone().into()),
        };
        let last = self.ts.peek(c1.close.unwrap_or(c1.last));
        let span = Span::new(t.start, last.end.max(t.end));
        // A located path: what reads or runs it is not seen here.
        let mut flags = RefFlags::LOCATED;
        if t.in_try() {
            flags |= RefFlags::OPTIONAL;
        }
        let site = Site {
            kind: RefKind::FsRead,
            rule: BaseRule::Path,
            span,
            arg_span,
            flags,
            requires: false,
        };
        let out = Outcome {
            val,
            env,
            complete: true,
            guess: input.guess,
            named,
            flow: false,
        };
        let wants = if c1.close.is_some() && flow_candidate(&out) {
            Wants::Value
        } else {
            Wants::Nothing
        };
        let start = self.ex.refs.len();
        self.emit_outcome(site, out, &detail, None);
        // The value-flow engine evaluates the whole `new URL(...)` expression.
        self.note_site(start, site, span, wants, detail, None);
        self.consume(lp);
    }

    /// `module.register(specifier, parentURL)` / `register(specifier, {
    /// parentURL })`: an ESM loader hooks module, resolved against
    /// `parentURL` (the file for `import.meta.url` /
    /// `pathToFileURL(__filename)`, the cwd for `pathToFileURL('./')`,
    /// the file when absent).
    fn module_register(&mut self, t: Token, lp: usize) {
        let c0 = self.capture_arg(lp, 0);
        if c0.end == c0.first || c0.truncated {
            return;
        }
        let c1 = self.capture_arg(lp, 1);
        let mut cwd = false;
        let mut unknown_parent = None;
        if c1.end > c1.first && !c1.truncated {
            let mut toks = self.ts.window(c1.first, c1.end);
            if toks[0].kind == TokKind::LBrace {
                // `{ parentURL: <expr>, ... }`
                let d = toks[0].depth + 1;
                let key = toks
                    .iter()
                    .position(|k| k.depth == d && k.text(self.src) == b"parentURL");
                toks = match key {
                    Some(i) if toks.get(i + 1).map(|k| k.kind) == Some(TokKind::Colon) => {
                        let rest = &toks[i + 2..];
                        let end = rest
                            .iter()
                            .position(|k| k.depth < d || (k.depth == d && k.kind == TokKind::Comma))
                            .unwrap_or(rest.len());
                        &rest[..end]
                    }
                    _ => &[],
                };
            }
            if !toks.is_empty() {
                match self.eval(toks).val {
                    JsVal::FileUrl | JsVal::Url(evaluator::Anchor::FileDir, _) => {}
                    JsVal::Url(evaluator::Anchor::Cwd, _) => cwd = true,
                    _ => unknown_parent = Some(evaluator::reason_text(self.src, toks)),
                }
            }
        }
        let last = self.ts.peek(c0.close.unwrap_or(c0.last));
        let span = Span::new(t.start, last.end.max(t.end));
        let before = self.ex.refs.len();
        let noted = self.sites.len();
        self.emit_arg(
            RefKind::Import,
            BaseRule::Module,
            &c0,
            span,
            RefFlags::empty(),
            None,
            false,
        );
        // The references are adjusted below: not a value-flow site.
        self.sites.truncate(noted);
        for r in &mut self.ex.refs[before..] {
            if cwd {
                r.base = Base::Cwd;
                if r.uncertainty.is_none() {
                    r.uncertainty = Some(Uncertainty::new(
                        UncertaintyKind::CwdDependent,
                        r.span,
                        "module.register parentURL is the working directory",
                    ));
                }
            } else if let Some(p) = &unknown_parent
                && r.uncertainty.is_none()
            {
                r.uncertainty = Some(Uncertainty::new(
                    UncertaintyKind::PartialSpecifier,
                    r.span,
                    format!("module.register parentURL `{p}` not known; resolved from the file"),
                ));
            }
        }
        self.consume(lp);
    }

    /// `eval(...)`.
    fn on_eval(&mut self, t: Token) {
        if t.prev_kw == Kw::Function
            || self.prev_byte(t.start) == Some(b'.')
            || self.ts.peek(0).kind != TokKind::LParen
        {
            return;
        }
        self.eval_like(t, 0, "eval", "");
    }

    /// Records an `EvalLike` diagnostic for the call whose `(` is at peek index
    /// `lp`; `rest` is what the detail says after the first argument.
    fn eval_like(&mut self, t: Token, lp: usize, what: &str, rest: &str) {
        let cap = self.capture_arg(lp, 0);
        let last = self.ts.peek(cap.last);
        let span = Span::new(t.start, last.end.max(t.end));
        let (_, detail) =
            args::arg_detail(self.src, self.ts.window(cap.first, cap.end), cap.truncated);
        self.ex.diagnostics.push(Uncertainty::new(
            UncertaintyKind::EvalLike,
            span,
            format!("{what}({detail}{rest})"),
        ));
        self.consume(lp);
    }

    /// Whether the second argument of the `new Worker(...)` whose `(` is at
    /// peek index `lp` is an object literal that sets `eval: true`: the
    /// last `eval` property is the literal `true`, and nothing written
    /// after it can replace it (a spread, a computed key). Anything else (a
    /// variable, `eval: flag`, the shorthand `eval`) is not known to
    /// evaluate, and the site stays a load.
    fn worker_evals(&mut self, lp: usize) -> bool {
        let cap = self.capture_arg(lp, 1);
        if cap.truncated || cap.end == cap.first {
            return false;
        }
        let src = self.src;
        let toks = self.ts.window(cap.first, cap.end);
        if toks[0].kind != TokKind::LBrace {
            return false;
        }
        // Tokens of the object's own level; nested values are deeper.
        let d = toks[0].depth + 1;
        let mut on = false;
        let mut i = 1;
        while i < toks.len() {
            // `i` is at the start of a property.
            let key = toks[i];
            let named = match key.kind {
                TokKind::Ident => key.text(src) == b"eval",
                TokKind::Str => matches!(key.text(src), b"'eval'" | b"\"eval\""),
                // `...rest` and `[key]: v` can set `eval` to anything.
                TokKind::Ellipsis | TokKind::LBracket => {
                    on = false;
                    false
                }
                _ => false,
            };
            let end = toks[i..]
                .iter()
                .position(|k| k.depth < d || (k.depth == d && k.kind == TokKind::Comma))
                .map_or(toks.len(), |n| i + n);
            if named {
                on = matches!(&toks[i + 1..end], [c, v]
                    if c.kind == TokKind::Colon && v.text(src) == b"true");
            }
            i = end + 1;
        }
        on
    }

    /// An identifier registered as a dynamic trigger: a loader alias or a
    /// binding of a tracked module.
    fn on_dyn(&mut self, t: Token) {
        let Some(name) = self.ident_str(&t) else {
            return;
        };
        if self.prev_byte(t.start) == Some(b'.') {
            return; // `obj.fs`
        }
        if let Some(l) = self.loaders.iter().rev().find(|l| l.local == name) {
            let aliased = l
                .aliased
                .then(|| format!("`{name}` is an alias of `require`"));
            let elsewhere = l.elsewhere;
            let n = self.ts.peek(0);
            if n.kind == TokKind::LParen {
                if elsewhere {
                    self.load_site(t, 0, RefKind::Require, RefFlags::empty(), true);
                } else {
                    self.call(t, RefKind::Require, 0, RefFlags::empty(), aliased);
                }
            } else if n.kind == TokKind::Dot && self.ts.peek(2).kind == TokKind::LParen && {
                let p = self.ts.peek(1);
                self.text(&p) == b"resolve"
            } {
                self.resolve_call(t, 2, elsewhere, aliased);
            }
            return;
        }
        let Some(b) = self.apis.iter().rev().find(|b| b.local == name) else {
            return;
        };
        let (module, member) = (b.module.clone(), b.member.clone());
        let (chain, lp) = self.member_chain(0);
        if self.ts.peek(lp).kind != TokKind::LParen {
            return;
        }
        let b = ApiBinding {
            local: name,
            module,
            member,
            depth: 0,
        };
        let path = b.export_path(&chain);
        if b.module == "module" && path == "createRequire" {
            self.on_create_require(t, lp);
        } else if b.module == "vm" && api::vm_evaluates(&path, CallStyle::Call) {
            self.eval_like(t, lp, &format!("vm.{path}"), "");
        } else if b.module == "module" && path == "register" {
            self.module_register(t, lp);
        } else if let Some(e) = api::lookup(Some(b.module.as_ref()), &path, CallStyle::Call) {
            self.api_call(t, e, lp);
        }
    }

    // ---- TypeScript type aliases ----

    /// `type Name<...> = ...`: every `import(...)` up to the end of the alias
    /// is a type.
    fn on_type(&mut self, t: Token) {
        if self.ts.peek(0).kind != TokKind::Ident {
            return;
        }
        let mut i = 1;
        if self.ts.peek(1).kind == TokKind::Lt {
            let mut nest = 0usize;
            loop {
                if i > 256 {
                    return;
                }
                match self.ts.peek(i).kind {
                    TokKind::Lt => nest += 1,
                    TokKind::Gt => {
                        nest = nest.saturating_sub(1);
                        if nest == 0 {
                            i += 1;
                            break;
                        }
                    }
                    TokKind::Eof | TokKind::Semi => return,
                    _ => {}
                }
                i += 1;
            }
        }
        if self.ts.peek(i).kind != TokKind::Assign {
            return;
        }
        i += 1;
        let d = t.depth;
        let mut prev = TokKind::Assign;
        let end = loop {
            let k = self.ts.peek(i);
            if i > MAX_REGION_TOKENS {
                break k.start;
            }
            if k.kind == TokKind::Eof || k.depth < d || (k.depth == d && k.kind == TokKind::Semi) {
                break k.start;
            }
            if k.depth == d
                && k.flags & NEWLINE_BEFORE != 0
                && k.kind == TokKind::Ident
                && matches!(
                    prev,
                    TokKind::Ident
                        | TokKind::RParen
                        | TokKind::RBracket
                        | TokKind::RBrace
                        | TokKind::Gt
                        | TokKind::Str
                        | TokKind::Template
                        | TokKind::TemplateTail
                        | TokKind::Number
                )
            {
                break k.start;
            }
            prev = k.kind;
            i += 1;
        };
        self.type_region_end = self.type_region_end.max(end);
    }

    // ---- triple-slash directives ----

    fn on_triple_slash(&mut self, t: Token) {
        let text = self.text(&t);
        let base = t.start as usize;
        let Some(rest) = text.strip_prefix(b"///") else {
            return;
        };
        let mut i = 3 + rest.iter().take_while(|c| c.is_ascii_whitespace()).count();
        let Some(tail) = text.get(i..) else { return };
        if !tail.starts_with(b"<reference") {
            return;
        }
        i += b"<reference".len();
        // Attributes: name = "value" | 'value'.
        loop {
            while text.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            let name_start = i;
            while text
                .get(i)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
            {
                i += 1;
            }
            if i == name_start {
                return;
            }
            let name = &text[name_start..i];
            while text.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            if text.get(i) != Some(&b'=') {
                return;
            }
            i += 1;
            while text.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                i += 1;
            }
            let Some(&q @ (b'"' | b'\'')) = text.get(i) else {
                return;
            };
            let v_start = i + 1;
            let Some(len) = memchr::memchr(q, &text[v_start..]) else {
                return;
            };
            i = v_start + len + 1;
            let value = &text[v_start..v_start + len];
            let is_path = name == b"path";
            if !is_path && name != b"types" {
                continue;
            }
            let Ok(v) = std::str::from_utf8(value) else {
                return;
            };
            if v.is_empty() {
                return;
            }
            // `path` is always relative to the file, even without `./`.
            let spec: Cow<'s, str> = if is_path
                && !(v.starts_with("./") || v.starts_with("../") || v.starts_with('/'))
            {
                Cow::Owned(format!("./{v}"))
            } else {
                Cow::Borrowed(v)
            };
            let span = Span::from_usize(base + v_start - 1, base + i);
            self.ex.refs.push(
                RawRef::new(RefKind::Import, Specifier::Literal(spec), span)
                    .with_flags(RefFlags::TYPE_ONLY),
            );
            return;
        }
    }
}

/// Where a value is used: the reference kind, base rule and spans.
#[derive(Copy, Clone)]
struct Site {
    kind: RefKind,
    rule: BaseRule,
    span: Span,
    arg_span: Span,
    flags: RefFlags,
    /// The site is a call of `require` itself (`require(x)`,
    /// `module.require(x)`), which loads what `x` names.
    /// `require.resolve(x)` only finds it.
    requires: bool,
}

/// A site the token pass evaluated, remembered for the value-flow pass.
struct FlowSite {
    /// The references the token pass emitted for it (indices into
    /// `Extraction::refs`).
    refs: std::ops::Range<usize>,
    site: Site,
    /// The expression whose value the site uses.
    expr: Span,
    /// What the value-flow engine is asked about `expr`.
    wants: Wants,
    detail: String,
    aliased: Option<String>,
}

/// What a site asks the value-flow engine.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Wants {
    /// Nothing: the token result stands (one string literal).
    Nothing,
    /// The value of the expression (the token result is unresolved, guessed, or
    /// relies on a binding by name).
    Value,
    /// The same, where the span only marks the start of the expression (the
    /// capture was truncated).
    ValueFrom,
    /// The path the call at the span resolves ([`Recognizer::load_site`]).
    Load,
}

impl Wants {
    fn ask(self) -> Option<crate::flow::Ask> {
        match self {
            Wants::Nothing => None,
            Wants::Value => Some(crate::flow::Ask::Value),
            Wants::ValueFrom => Some(crate::flow::Ask::ValueFrom),
            Wants::Load => Some(crate::flow::Ask::Load),
        }
    }
}

/// Maximum sites per file remembered for the value-flow pass.
const MAX_FLOW_SITES: usize = 4096;
/// Maximum alternatives of one reference; larger sets collapse into a pattern.
const MAX_SITE_ALTERNATIVES: usize = 64;

/// Whether the template `value` has an alternative the environment layer can
/// evaluate (one that is not just the text of an unknown expression).
fn evaluable(value: &turborepo_trace_core::env::EnvValue) -> bool {
    value
        .alts
        .iter()
        .any(|a| matches!(a, turborepo_trace_core::env::EnvAlt::Term(_)))
}

/// Marks a reference of a site whose token result the value-flow engine did not
/// check: it keeps its target and gets an open record, unless it has one. The
/// token evaluator reads names without scopes and top-level `let` / `var`
/// bindings by their initial value, so its result alone is a guess.
fn unchecked(r: &mut RawRef<'_>, at: Span, detail: &str) {
    // An informational record is replaced; any other (also `CwdDependent`, which
    // the working directory of a test can resolve) stays.
    let open = r
        .uncertainty
        .as_ref()
        .is_some_and(|u| u.kind != UncertaintyKind::PartialSpecifier);
    if open {
        return;
    }
    let why = match &r.uncertainty {
        Some(u) if !u.detail.is_empty() => {
            format!("{} (not checked by the value-flow engine)", u.detail)
        }
        _ => format!("{detail} (not checked by the value-flow engine)"),
    };
    r.uncertainty = Some(Uncertainty::new(
        UncertaintyKind::ComputedSpecifier,
        at,
        why,
    ));
}

/// Whether the value-flow engine can improve on, or must verify, a token
/// result.
fn flow_candidate(out: &Outcome) -> bool {
    fn value_known(v: &Value) -> bool {
        match v {
            Value::Str(_) => true,
            Value::Alternatives(a) => a.iter().all(value_known),
            _ => false,
        }
    }
    fn known(v: &JsVal) -> bool {
        match v {
            JsVal::Str(x) | JsVal::Path(_, x) | JsVal::Url(_, x) | JsVal::Resolved(_, x) => {
                value_known(x)
            }
            JsVal::Alts(vs) => vs.iter().all(known),
            _ => false,
        }
    }
    out.named || out.env || out.guess.is_some() || !out.complete || !known(&out.val)
}

/// The specifier for the alternatives of one base: merged like
/// [`Value::alternatives`] up to its cap, kept as separate alternatives up to
/// [`MAX_SITE_ALTERNATIVES`] (registries of known paths), collapsed into a
/// pattern beyond.
fn merge_specs(specs: Vec<Value>) -> Specifier<'static> {
    let mut flat: Vec<Value> = Vec::new();
    for v in specs {
        match v {
            Value::Alternatives(a) => {
                for x in a {
                    if !flat.contains(&x) {
                        flat.push(x);
                    }
                }
            }
            v => {
                if !flat.contains(&v) {
                    flat.push(v);
                }
            }
        }
    }
    if flat.len() <= turborepo_trace_core::eval::MAX_ALTERNATIVES
        || flat.len() > MAX_SITE_ALTERNATIVES
    {
        return Value::alternatives(flat).to_specifier();
    }
    Specifier::Alternatives(flat.iter().map(Value::to_specifier).collect())
}

/// The wildcard reasons of a specifier.
fn spec_reasons(spec: &Specifier<'_>, out: &mut Vec<String>) {
    match spec {
        Specifier::Pattern(parts) => {
            for p in parts {
                if let turborepo_trace_core::Part::Wild { reason } = p {
                    out.push(reason.to_string());
                }
            }
        }
        Specifier::Alternatives(alts) => alts.iter().for_each(|a| spec_reasons(a, out)),
        Specifier::Literal(_) | Specifier::Unknown => {}
    }
}

/// `v` with the origin marks removed from its wildcard reasons.
fn strip_marks(v: Value) -> Value {
    use turborepo_trace_core::eval::PatPart;
    match v {
        Value::Str(_) => v,
        Value::Unknown { reason } => Value::unknown(origin::strip(&reason)),
        Value::Pattern(parts) => Value::Pattern(
            parts
                .into_iter()
                .map(|p| match p {
                    PatPart::Wild { reason } => PatPart::Wild {
                        reason: origin::strip(&reason).into(),
                    },
                    lit => lit,
                })
                .collect(),
        ),
        Value::Alternatives(a) => Value::Alternatives(a.into_iter().map(strip_marks).collect()),
    }
}

/// The target of an assignment found by [`Recognizer::assigned_target`].
enum Target<'s> {
    /// `name = ...`.
    Name(&'s str),
    /// `{ key, key: local } = ...`: `(key, local)` pairs.
    Destructure(Vec<(&'s str, &'s str)>),
}

/// The literal token of a (possibly parenthesized) single-literal argument.
fn single_literal(toks: &[Token]) -> Option<Token> {
    let mut inner = toks;
    while inner.len() >= 3
        && inner[0].kind == TokKind::LParen
        && inner[inner.len() - 1].kind == TokKind::RParen
        && inner[inner.len() - 1].depth == inner[0].depth
        && inner[1..inner.len() - 1]
            .iter()
            .all(|t| t.depth > inner[0].depth)
    {
        inner = &inner[1..inner.len() - 1];
    }
    match inner {
        [t] if matches!(t.kind, TokKind::Str | TokKind::Template) => Some(*t),
        _ => None,
    }
}

fn skip_ws_back(src: &[u8], mut i: usize) -> usize {
    while i > 0 && src[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The identifier ending right before `end` (whitespace skipped), or empty.
fn ident_before(src: &[u8], end: usize) -> &[u8] {
    let e = skip_ws_back(src, end);
    let mut s = e;
    while s > 0 && is_ident_byte(src[s - 1]) {
        s -= 1;
    }
    &src[s..e]
}

/// Parses `{ a, b: c, d = 1, ...rest }` backwards from the `}` at `close`.
/// Gives up on nesting, strings and comments.
fn destructure_back(src: &[u8], close: usize) -> Option<Vec<(&str, &str)>> {
    let lo = close.saturating_sub(1024);
    let mut open = None;
    for j in (lo..close).rev() {
        match src[j] {
            b'{' => {
                open = Some(j);
                break;
            }
            b'}' | b'\'' | b'"' | b'`' | b'/' | b'(' | b')' | b'[' | b']' => return None,
            _ => {}
        }
    }
    let inner = std::str::from_utf8(&src[open? + 1..close]).ok()?;
    let mut out = Vec::new();
    for entry in inner.split(',') {
        let entry = entry.split('=').next().unwrap_or("").trim();
        if entry.is_empty() || entry.starts_with("...") {
            continue;
        }
        let (key, local) = match entry.split_once(':') {
            Some((k, l)) => (k.trim(), l.trim()),
            None => (entry, entry),
        };
        let ok = |s: &str| {
            !s.is_empty() && s.bytes().all(is_ident_byte) && !s.as_bytes()[0].is_ascii_digit()
        };
        if !ok(key) || !ok(local) {
            return None;
        }
        out.push((key, local));
    }
    Some(out)
}
