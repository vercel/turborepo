//! The deep pass: nft's analyses that need the tokens around a site, not just
//! the call arguments the main recognizer captures. It re-lexes the file into a
//! token vector (every token, no skimming) and runs only when the file can
//! contain one of its forms:
//!
//! - Path values as assets (nft's "static child" emission). An expression that
//!   evaluates to a path anchored at `__dirname` / `__filename` /
//!   `process.cwd()` (or at the file `require.resolve('pkg')` returns) is
//!   emitted as a [`RefKind::Asset`] even when it never reaches a tracked fs
//!   call: assigned (`const p = path.join(__dirname, 'x')`), returned,
//!   exported, passed to `spawn` or an untracked function, or read through an
//!   undeclared global `fs`. Starting at each anchor (and at each top-level
//!   constant or `let`/`var` holding an anchored path), the pass grows to the
//!   largest enclosing expression that still evaluates to a path: the comma-,
//!   statement- and operator-delimited element around the anchor, then the
//!   element around its enclosing parenthesis or template substitution, and so
//!   on. The file's own directory, the cwd, ancestors of the file's directory
//!   and the file itself are never emitted (nft's `validWildcard`); nor are
//!   expressions already inside a reference the main pass resolved, nor the
//!   argument of `createRequire(...)` (the place a `require` function searches
//!   from: Node does not read it). Missing paths resolve quietly (assets are
//!   probes), patterns glob, and a directory expands to every file below it
//!   with a `PartialSpecifier` record added by the resolver.
//! - Require wrappers: `const load = name => require(name)` (also `function
//!   load(name)`): calls `load('./x')` are `require` references with
//!   [`UncertaintyKind::AliasedLoader`].
//! - Browserify bundles: in a module table entry `[function (r, module,
//!   exports) {...}, {deps}]`, `r('x')` is a `require` of `x` unless the
//!   dependency map points `x` at a bundled module id
//!   ([`UncertaintyKind::AliasedLoader`]).
//! - pino transports: in a file importing `pino` or `fastify`, an object
//!   property `target: 'module'` names a module pino loads in a worker thread
//!   (`OPTIONAL`, with an [`UncertaintyKind::AliasedLoader`] record).
//!
//! - Module strings: a string literal in a registry position (an object
//!   property value or an array element) that looks like a module path
//!   (`'services/a/src/handlers/get'`, `'./lib/stub.ts'`), such as the handler
//!   paths of a route table that a framework imports later. Emitted as an
//!   optional lazy [`RefKind::DynamicImport`] with the base
//!   [`MODULE_STRING_BASE`]; the resolver turns it into an edge only when it
//!   names an existing first-party source file, relative to the repository
//!   root, the file's package or (for `./` and `../`) the file, and otherwise
//!   drops it without an uncertainty.
//!
//! Every guess carries an uncertainty record except a literal anchored path,
//! which is nft's documented behavior and resolves only if the file exists, and
//! module strings, which only ever add edges to files that exist.

use std::{collections::HashSet, sync::LazyLock};

use memchr::memmem::Finder;
use turborepo_trace_core::{
    RawRef, RefFlags, RefKind, Span, Specifier, Uncertainty, UncertaintyKind,
    eval::{Value, anchor, loader::Wrappers},
};

use super::{
    Recognizer, Site,
    api::BaseRule,
    args,
    evaluator::{self, Anchor, JsVal, Outcome},
};
use crate::lexer::{Kw, Lexer, NEWLINE_BEFORE, TokKind, Token};

/// Files larger than this are not re-lexed.
const MAX_DEEP_BYTES: usize = 32 << 20;
/// Maximum tokens scanned to each side of an element.
const MAX_SIDE: usize = 1024;
/// Maximum growth steps from an anchor to its largest path expression.
const MAX_GROW: usize = 16;
/// Maximum assets emitted per file.
const MAX_ASSETS: usize = 512;
/// Maximum references emitted per wrapper or bundle.
const MAX_CALLS: usize = 256;
/// Maximum module strings emitted per file (a large route table has under a
/// thousand).
const MAX_MODULE_STRINGS: usize = 4096;
/// Maximum length of a module string.
const MAX_MODULE_STRING: usize = 200;

/// The [`turborepo_trace_core::Base::Language`] anchor of a module string: the
/// specifier is resolved as a path relative to the repository (workspace) root,
/// the file's package root or, for `./` and `../`, the file's directory.
pub const MODULE_STRING_BASE: &str = "module_string";

/// Whether `s` has the shape of a module string: a relative path of at least
/// two segments made of name characters, whose last segment has a letter. `./`
/// and `../` segments may only lead.
pub fn module_string_shape(s: &[u8]) -> bool {
    if s.len() < 3 || s.len() > MAX_MODULE_STRING || !s.contains(&b'/') {
        return false;
    }
    if !(s[0].is_ascii_alphanumeric() || s[0] == b'_' || s[0] == b'.') {
        return false;
    }
    let mut leading = true;
    let mut last: &[u8] = b"";
    for seg in s.split(|&c| c == b'/') {
        match seg {
            b"" => return false,
            b"." | b".." => {
                if !leading {
                    return false;
                }
            }
            _ => {
                if !seg
                    .iter()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b'@'))
                    || seg[0] == b'@'
                {
                    return false;
                }
                leading = false;
            }
        }
        last = seg;
    }
    !leading && last.iter().any(u8::is_ascii_alphabetic)
}

/// Whether `src` may contain a module string: a quoted literal of that shape
/// right after `:`, `[` or `,` (a cheap scan; the deep pass then checks
/// tokens).
pub(super) fn may_have_module_strings(src: &[u8]) -> bool {
    for q in memchr::memchr3_iter(b'\'', b'"', b'`', src) {
        let mut i = q;
        while i > 0 && src[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        if i == 0 || !matches!(src[i - 1], b':' | b'[' | b',') {
            continue;
        }
        let rest = &src[q + 1..];
        let window = &rest[..rest.len().min(MAX_MODULE_STRING + 1)];
        if let Some(end) = memchr::memchr(src[q], window)
            && module_string_shape(&window[..end])
        {
            return true;
        }
    }
    false
}

pub(super) const NONE: u32 = u32::MAX;

/// A function-local anchored binding: the token index where its scope ends, its
/// name, and the top-level `let`/`var` value it shadows.
type Local<'s> = (usize, &'s str, Option<(JsVal, bool)>);

static ANCHOR_FINDERS: LazyLock<[Finder<'static>; 3]> = LazyLock::new(|| {
    [
        Finder::new(b"__dirname"),
        Finder::new(b"__filename"),
        Finder::new(b"process.cwd"),
    ]
});
static BROWSERIFY_FINDER: LazyLock<Finder<'static>> =
    LazyLock::new(|| Finder::new(b"MODULE_NOT_FOUND"));

/// Which analyses a file needs.
#[derive(Default)]
struct Want {
    anchors: bool,
    wrappers: bool,
    browserify: bool,
    pino: bool,
    strings: bool,
    /// Path arguments ([`super::patharg`]).
    path_args: bool,
}

impl Want {
    fn any(&self) -> bool {
        self.anchors
            || self.wrappers
            || self.browserify
            || self.pino
            || self.strings
            || self.path_args
    }
}

/// Bracket structure of a token vector.
pub(super) struct Tree {
    /// The innermost opener enclosing each token (`NONE` at the top level).
    pub(super) parent: Vec<u32>,
    /// For an opener, its closer (`NONE` if unclosed).
    close: Vec<u32>,
    /// For a closer, its opener.
    open: Vec<u32>,
}

fn is_opener(k: TokKind) -> bool {
    matches!(
        k,
        TokKind::LParen
            | TokKind::LBracket
            | TokKind::LBrace
            | TokKind::TemplateHead
            | TokKind::TemplateMiddle
    )
}

impl Tree {
    fn new(toks: &[Token]) -> Tree {
        let n = toks.len();
        let mut t = Tree {
            parent: vec![NONE; n],
            close: vec![NONE; n],
            open: vec![NONE; n],
        };
        let mut stack: Vec<u32> = Vec::new();
        for (i, tok) in toks.iter().enumerate() {
            let d = tok.depth as usize;
            while stack.len() > d {
                if let Some(o) = stack.pop() {
                    t.close[o as usize] = i as u32;
                    t.open[i] = o;
                }
            }
            t.parent[i] = stack.last().copied().unwrap_or(NONE);
            if is_opener(tok.kind) {
                stack.push(i as u32);
            }
        }
        t
    }
}

/// Lexes every token of `src` (without the final `Eof`). `None` if the JSX
/// heuristic failed.
fn lex_all(src: &[u8], dialect: crate::lexer::Dialect) -> Option<Vec<Token>> {
    let mut lx = Lexer::new(src, dialect);
    let mut out = Vec::with_capacity(src.len() / 5);
    loop {
        let t = lx.next_token();
        if t.kind == TokKind::Eof {
            break;
        }
        out.push(t);
    }
    (!lx.jsx_failed()).then_some(out)
}

/// Whether `t` ends an element at its own depth.
fn is_sep(src: &[u8], t: &Token) -> bool {
    match t.kind {
        TokKind::Comma
        | TokKind::Semi
        | TokKind::Colon
        | TokKind::Question
        | TokKind::Assign
        | TokKind::Arrow
        | TokKind::Eof => true,
        TokKind::Op => {
            let s = t.text(src);
            s.len() >= 2
                && s.ends_with(b"=")
                && !matches!(s, b"==" | b"===" | b"!=" | b"!==" | b"<=" | b">=")
        }
        TokKind::Ident => matches!(
            t.kw,
            Kw::Return
                | Kw::Throw
                | Kw::Case
                | Kw::Default
                | Kw::Else
                | Kw::Do
                | Kw::Yield
                | Kw::Const
                | Kw::Let
                | Kw::Var
                | Kw::Export
                | Kw::In
                | Kw::Of
                | Kw::Extends
                | Kw::Typeof
                | Kw::Void
                | Kw::Delete
                | Kw::Instanceof
                | Kw::If
                | Kw::While
                | Kw::For
                | Kw::With
                | Kw::Try
                | Kw::Function
                | Kw::Class
                | Kw::From
                | Kw::As
        ),
        _ => false,
    }
}

/// Automatic semicolon insertion between `prev` and `cur` (a new line starting
/// with a value after a complete value).
fn asi_break(prev: &Token, cur: &Token) -> bool {
    cur.flags & NEWLINE_BEFORE != 0
        && matches!(
            cur.kind,
            TokKind::Ident | TokKind::Str | TokKind::Template | TokKind::TemplateHead
        )
        && matches!(
            prev.kind,
            TokKind::Ident
                | TokKind::Str
                | TokKind::Number
                | TokKind::Template
                | TokKind::TemplateTail
                | TokKind::RParen
                | TokKind::RBracket
                | TokKind::RBrace
                | TokKind::Regex
                | TokKind::Jsx
        )
}

/// Whether a value contains an anchored path.
fn anchored(v: &JsVal) -> bool {
    match v {
        JsVal::Path(..) | JsVal::Url(..) | JsVal::Resolved(..) => true,
        JsVal::Alts(vs) => vs.iter().any(anchored),
        _ => false,
    }
}

/// A path nft never emits: the anchor directory itself, an ancestor, the file
/// itself, or a pattern without any literal name (the shared rule,
/// [`anchor::is_trivial`]; the file name only counts for the file's own
/// directory).
fn trivial(v: &Value, anchor: Anchor, file_name: Option<&str>) -> bool {
    anchor::is_trivial(v, file_name.filter(|_| anchor == Anchor::FileDir))
}

fn useful_leaf(l: &JsVal, file_name: Option<&str>) -> bool {
    match l {
        JsVal::Path(an, v) | JsVal::Url(an, v) => {
            !trivial(&evaluator::normalize_value(v.clone(), true), *an, file_name)
        }
        // The resolved module itself is the main pass's `require.resolve` reference.
        JsVal::Resolved(_, v) => !trivial(
            &evaluator::normalize_value(v.clone(), true),
            Anchor::Cwd,
            None,
        ),
        _ => false,
    }
}

impl<'s> Recognizer<'s, '_> {
    fn deep_wanted(&self) -> Want {
        let src = self.src;
        let mut w = Want {
            anchors: ANCHOR_FINDERS.iter().any(|f| f.find(src).is_some()),
            browserify: BROWSERIFY_FINDER.find(src).is_some(),
            strings: !self.dialect.dts && may_have_module_strings(src),
            path_args: !self.dialect.dts && super::patharg::may_have(src),
            ..Want::default()
        };
        for r in &self.ex.refs {
            match (&r.kind, &r.spec) {
                (RefKind::Require, Specifier::Unknown) => w.wrappers = true,
                (
                    RefKind::Require | RefKind::Import | RefKind::DynamicImport,
                    Specifier::Literal(s),
                ) if super::api::registry_module(s) => w.pino = true,
                _ => {}
            }
        }
        w
    }

    /// Runs the deep pass (see the module docs) after the main pass.
    pub(super) fn deep_pass(&mut self) {
        let want = self.deep_wanted();
        if !want.any() || self.src.len() > MAX_DEEP_BYTES {
            return;
        }
        let Some(toks) = lex_all(self.src, self.dialect) else {
            return;
        };
        let tree = Tree::new(&toks);
        let before = self.ex.refs.len();
        if want.anchors {
            // Bindings declared inside functions ended with their scope in the main pass;
            // the evaluator modules (`path`, `url`, ...) are rebound for the whole file.
            let saved = self.apis.len();
            self.local_path_bindings(&toks);
            self.deep_assets(&toks, &tree, before);
            self.apis.truncate(saved);
        }
        if want.wrappers {
            self.deep_wrappers(&toks, &tree);
        }
        if want.browserify {
            self.deep_browserify(&toks, &tree);
        }
        if want.pino {
            self.deep_pino(&toks);
        }
        if want.strings {
            self.deep_module_strings(&toks, &tree);
        }
        if want.path_args {
            self.deep_path_args(&toks, &tree);
        }
        if self.ex.refs.len() > before {
            // Keep source order (stable: same-start references keep their order).
            self.ex.refs.sort_by_key(|r| r.span.start);
        }
    }

    /// `NAME = require('path')` (also `url`, `process`, `resolve-from`, with
    /// `node:`) at any depth, as file-wide bindings. Shadowing is ignored:
    /// these names are conventional.
    fn local_path_bindings(&mut self, toks: &[Token]) {
        let src = self.src;
        for k in 2..toks.len().saturating_sub(3) {
            if toks[k].kw != Kw::Require
                || toks[k - 1].kind != TokKind::Assign
                || toks[k - 2].kind != TokKind::Ident
                || toks[k - 2].kw != Kw::None
                || toks[k + 1].kind != TokKind::LParen
                || toks[k + 2].kind != TokKind::Str
                || toks[k + 3].kind != TokKind::RParen
                || toks[k - 2].depth == 0
                // `x = require('url').pathToFileURL(...)` is not the module.
                || toks.get(k + 4).is_some_and(|n| {
                    matches!(
                        n.kind,
                        TokKind::Dot
                            | TokKind::OptChain
                            | TokKind::LParen
                            | TokKind::LBracket
                            | TokKind::Template
                            | TokKind::TemplateHead
                    )
                })
            {
                continue;
            }
            let Some(Ok(m)) = args::literal(src, &toks[k + 2]) else {
                continue;
            };
            let m = super::api::normalize_module(&m);
            if !matches!(
                m,
                "path" | "path/posix" | "url" | "process" | "resolve-from"
            ) {
                continue;
            }
            let Ok(local) = std::str::from_utf8(toks[k - 2].text(src)) else {
                continue;
            };
            if self.apis.len() >= 2 * super::MAX_BINDINGS
                || self.apis.iter().any(|b| b.local == local)
            {
                continue;
            }
            self.apis.push(super::api::ApiBinding {
                local,
                module: std::borrow::Cow::Owned(m.to_owned()),
                member: None,
                depth: 0,
            });
        }
    }

    /// The element around token `i` (see the module docs): token indices
    /// `a..=b`.
    fn element(&self, toks: &[Token], tree: &Tree, i: usize) -> Option<(usize, usize)> {
        let src = self.src;
        let d = toks[i].depth;
        let p = tree.parent[i];
        let mut a = i;
        let mut n = 0;
        while a > 0 {
            let k = a - 1;
            if k as u32 == p {
                break;
            }
            let tk = &toks[k];
            if tk.depth < d {
                break;
            }
            if tk.depth == d && (is_sep(src, tk) || asi_break(tk, &toks[a])) {
                break;
            }
            a = k;
            n += 1;
            if n > MAX_SIDE {
                return None;
            }
        }
        let mut b = i;
        n = 0;
        while b + 1 < toks.len() {
            let tk = &toks[b + 1];
            if tk.depth < d {
                break;
            }
            if tk.depth == d && (is_sep(src, tk) || asi_break(&toks[b], tk)) {
                break;
            }
            b += 1;
            n += 1;
            if n > MAX_SIDE {
                return None;
            }
        }
        Some((a, b))
    }

    /// The largest expression around anchor token `k` that evaluates to an
    /// anchored path.
    fn grow(&self, toks: &[Token], tree: &Tree, k: usize) -> Option<(usize, usize, Outcome)> {
        let (mut a, mut b) = self.element(toks, tree, k)?;
        let mut best = None;
        for _ in 0..MAX_GROW {
            if b - a >= args::MAX_ARG_TOKENS {
                break;
            }
            let out = self.eval(&toks[a..=b]);
            if !(out.complete && anchored(&out.val)) {
                break;
            }
            best = Some((a, b, out));
            let p = tree.parent[a];
            if p == NONE || toks[p as usize].kind == TokKind::LBrace {
                break;
            }
            match self.element(toks, tree, p as usize) {
                Some((x, y)) => (a, b) = (x, y),
                None => break,
            }
        }
        best
    }

    /// Path values as assets.
    fn deep_assets(&mut self, toks: &[Token], tree: &Tree, main_refs: usize) {
        let src = self.src;
        // Spans the main pass already resolved.
        let covered: Vec<Span> = self.ex.refs[..main_refs]
            .iter()
            .filter(|r| !matches!(r.spec, Specifier::Unknown))
            .map(|r| r.span)
            .collect();
        // Top-level names holding an anchored path.
        let mut names: HashSet<&'s str> = self
            .consts
            .iter()
            .chain(self.vars.iter())
            .filter(|(_, (v, _))| anchored(v))
            .map(|(k, _)| *k)
            .collect();
        let mut seen: HashSet<String> = HashSet::new();
        let mut last = Span::default();
        let mut emitted = 0;
        // Function-local `const|let|var NAME = <anchored>` bindings, live until their
        // block closes: (token index where the scope ends, name, the shadowed
        // `vars` entry).
        let mut locals: Vec<Local<'s>> = Vec::new();
        for k in 0..toks.len() {
            while locals.last().is_some_and(|l| l.0 <= k) {
                if let Some((_, name, prev)) = locals.pop() {
                    match prev {
                        Some(v) => self.vars.insert(name, v),
                        None => self.vars.remove(name),
                    };
                }
            }
            let t = &toks[k];
            if t.kind != TokKind::Ident || !matches!(t.kw, Kw::None | Kw::Require) {
                continue;
            }
            let txt = t.text(src);
            let trigger = match txt {
                // `require.resolve('pkg')`: a path relative to the package's resolved file.
                b"require" => {
                    toks.get(k + 1).is_some_and(|d| d.kind == TokKind::Dot)
                        && toks.get(k + 2).is_some_and(|c| c.text(src) == b"resolve")
                        && toks.get(k + 3).is_some_and(|p| p.kind == TokKind::LParen)
                }
                b"__dirname" | b"__filename" => true,
                b"process" => {
                    toks.get(k + 1).is_some_and(|d| d.kind == TokKind::Dot)
                        && toks.get(k + 2).is_some_and(|c| c.text(src) == b"cwd")
                }
                _ => std::str::from_utf8(txt).is_ok_and(|s| names.contains(s)),
            };
            if !trigger
                || (k > 0 && matches!(toks[k - 1].kind, TokKind::Dot | TokKind::OptChain))
                || (k > 0 && matches!(toks[k - 1].kw, Kw::Const | Kw::Let | Kw::Var))
                || toks.get(k + 1).is_some_and(|n| n.kind == TokKind::Assign)
                || (t.start >= last.start && t.end <= last.end)
            {
                continue;
            }
            let Some((a, b, out)) = self.grow(toks, tree, k) else {
                continue;
            };
            // `const NAME = <this expression>` inside a function: NAME is an anchor too,
            // within its block (shadowing beyond that is not tracked; values read through
            // it carry a record).
            if a >= 3
                && toks[a - 1].kind == TokKind::Assign
                && toks[a - 2].kind == TokKind::Ident
                && toks[a - 2].kw == Kw::None
                && matches!(toks[a - 3].kw, Kw::Const | Kw::Let | Kw::Var)
                && toks[a - 2].depth > 0
                && locals.len() < MAX_ASSETS
                && let Ok(name) = std::str::from_utf8(toks[a - 2].text(src))
            {
                let p = tree.parent[a - 2];
                let end = if p == NONE {
                    toks.len()
                } else {
                    match tree.close[p as usize] {
                        NONE => toks.len(),
                        c => c as usize,
                    }
                };
                let prev = self.vars.insert(name, (out.val.clone(), out.env));
                locals.push((end, name, prev));
                // Keep the stack ordered by scope end (inner scopes end first).
                locals.sort_by_key(|l| std::cmp::Reverse(l.0));
                names.insert(name);
            }
            let span = Span::new(toks[a].start, toks[b].end);
            if covered
                .iter()
                .any(|c| c.start <= span.start && span.end <= c.end)
            {
                last = span;
                continue;
            }
            // The argument of `createRequire(...)` says where a `require` function searches
            // from: Node does not read that path.
            if a >= 2
                && toks[a - 1].kind == TokKind::LParen
                && toks[a - 2].text(src) == b"createRequire"
                && toks.get(b + 1).is_some_and(|c| c.kind == TokKind::RParen)
            {
                last = span;
                continue;
            }
            let leaves: Vec<JsVal> = evaluator::split_alts(out.val)
                .into_iter()
                .filter(|l| useful_leaf(l, self.file_name))
                .collect();
            if leaves.is_empty() {
                continue;
            }
            last = span;
            let val = evaluator::alts(leaves);
            if !seen.insert(format!("{val:?}")) {
                continue;
            }
            let detail = evaluator::reason_text(src, &toks[a..=b]);
            self.emit_outcome(
                Site {
                    kind: RefKind::Asset,
                    rule: BaseRule::Path,
                    span,
                    arg_span: span,
                    flags: RefFlags::empty(),
                    requires: false,
                },
                Outcome {
                    val,
                    env: out.env,
                    complete: true,
                    guess: out.guess,
                    named: out.named,
                    flow: false,
                },
                &detail,
                None,
            );
            emitted += 1;
            if emitted >= MAX_ASSETS {
                break;
            }
        }
        // Restore the shadowed top-level values.
        while let Some((_, name, prev)) = locals.pop() {
            match prev {
                Some(v) => self.vars.insert(name, v),
                None => self.vars.remove(name),
            };
        }
    }

    /// The name and parameters of the function whose body opens at token `o`.
    fn function_head(
        &self,
        toks: &[Token],
        tree: &Tree,
        o: usize,
    ) -> Option<(&'s str, Vec<&'s str>)> {
        let src = self.src;
        let ident = |i: usize| -> Option<&'s str> {
            let t = toks.get(i)?;
            (t.kind == TokKind::Ident && t.kw == Kw::None)
                .then(|| std::str::from_utf8(t.text(src)).ok())
                .flatten()
        };
        let params_of = |po: usize, pc: usize| -> Vec<&'s str> {
            let d = toks[po].depth + 1;
            let mut out = Vec::new();
            let mut expect = true;
            for (i, t) in toks.iter().enumerate().take(pc).skip(po + 1) {
                if t.depth != d {
                    continue;
                }
                if t.kind == TokKind::Comma {
                    expect = true;
                } else if expect {
                    out.push(ident(i).unwrap_or(""));
                    expect = false;
                }
            }
            out
        };
        // `name = <head> {`: the binding name before the head starting at `start`.
        let assigned = |mut start: usize| -> Option<&'s str> {
            if start > 0 && toks[start - 1].text(src) == b"async" {
                start -= 1;
            }
            (start >= 2 && toks[start - 1].kind == TokKind::Assign)
                .then(|| ident(start - 2))
                .flatten()
        };
        // `o` is a body `{`, or the first token of a concise arrow body.
        let prev = o.checked_sub(1)?;
        match toks[prev].kind {
            TokKind::Arrow => {
                let before = prev.checked_sub(1)?;
                match toks[before].kind {
                    TokKind::RParen => {
                        let po = tree.open[before];
                        if po == NONE {
                            return None;
                        }
                        let po = po as usize;
                        Some((assigned(po)?, params_of(po, before)))
                    }
                    TokKind::Ident => Some((assigned(before)?, vec![ident(before)?])),
                    _ => None,
                }
            }
            TokKind::RParen => {
                let po = tree.open[prev];
                if po == NONE || po < 2 {
                    return None;
                }
                let po = po as usize;
                let params = params_of(po, prev);
                if toks[po - 1].kw == Kw::Function {
                    Some((assigned(po - 1)?, params))
                } else if toks[po - 2].kw == Kw::Function {
                    Some((ident(po - 1)?, params))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Require wrappers (see the module docs).
    fn deep_wrappers(&mut self, toks: &[Token], tree: &Tree) {
        let src = self.src;
        let mut wrappers: Wrappers<&'s str> = Wrappers::new(super::api::LOADER, "require wrapper");
        for k in 0..toks.len().saturating_sub(3) {
            let t = &toks[k];
            if t.kw != Kw::Require
                || (k > 0 && matches!(toks[k - 1].kind, TokKind::Dot | TokKind::OptChain))
                || toks[k + 1].kind != TokKind::LParen
                || toks[k + 2].kind != TokKind::Ident
                || toks[k + 2].kw != Kw::None
                || toks[k + 3].kind != TokKind::RParen
            {
                continue;
            }
            let Ok(param) = std::str::from_utf8(toks[k + 2].text(src)) else {
                continue;
            };
            // A concise arrow body (`name => require(name)`), else the enclosing `{` body.
            let concise = self
                .element(toks, tree, k)
                .map(|(a, _)| a)
                .filter(|&a| a > 0 && toks[a - 1].kind == TokKind::Arrow);
            let body = match concise {
                Some(a) => a,
                None => {
                    let mut o = tree.parent[k];
                    while o != NONE && toks[o as usize].kind != TokKind::LBrace {
                        o = tree.parent[o as usize];
                    }
                    if o == NONE {
                        continue;
                    }
                    o as usize
                }
            };
            let Some((name, params)) = self.function_head(toks, tree, body) else {
                continue;
            };
            let Some(idx) = params.iter().position(|p| *p == param) else {
                continue;
            };
            if !wrappers.add(name, idx) {
                break;
            }
        }
        for w in wrappers.iter() {
            let (name, idx) = (w.name, w.param);
            let mut n = 0;
            for j in 0..toks.len().saturating_sub(1) {
                let t = &toks[j];
                if t.kind != TokKind::Ident
                    || t.kw != Kw::None
                    || t.text(src) != name.as_bytes()
                    || toks[j + 1].kind != TokKind::LParen
                    || (j > 0
                        && (matches!(toks[j - 1].kind, TokKind::Dot | TokKind::OptChain)
                            || toks[j - 1].kw == Kw::Function))
                {
                    continue;
                }
                let lp = j + 1;
                let close = tree.close[lp];
                if close == NONE {
                    continue;
                }
                let Some((a, b)) = arg_range(toks, lp, close as usize, idx) else {
                    continue;
                };
                let out = self.eval(&toks[a..b]);
                let (arg_span, detail) = args::arg_detail(src, &toks[a..b], false);
                let mut flags = RefFlags::empty();
                if t.in_try() {
                    flags |= RefFlags::OPTIONAL;
                }
                self.emit_outcome(
                    Site {
                        kind: RefKind::Require,
                        rule: BaseRule::Module,
                        span: Span::new(t.start, toks[close as usize].end),
                        arg_span,
                        flags,
                        requires: false,
                    },
                    out,
                    &detail,
                    Some(wrappers.detail(name)),
                );
                n += 1;
                if n >= MAX_CALLS {
                    break;
                }
            }
        }
    }

    /// Browserify module tables (see the module docs).
    fn deep_browserify(&mut self, toks: &[Token], tree: &Tree) {
        let src = self.src;
        for k in 0..toks.len().saturating_sub(4) {
            if toks[k].kind != TokKind::LBracket
                || toks[k + 1].kw != Kw::Function
                || toks[k + 2].kind != TokKind::LParen
                || toks[k + 3].kind != TokKind::Ident
            {
                continue;
            }
            let pc = tree.close[k + 2];
            if pc == NONE {
                continue;
            }
            let body = pc as usize + 1;
            if toks.get(body).map(|t| t.kind) != Some(TokKind::LBrace) {
                continue;
            }
            let bc = tree.close[body];
            if bc == NONE {
                continue;
            }
            let bc = bc as usize;
            let deps = bc + 2;
            if toks.get(bc + 1).map(|t| t.kind) != Some(TokKind::Comma)
                || toks.get(deps).map(|t| t.kind) != Some(TokKind::LBrace)
            {
                continue;
            }
            let dc = tree.close[deps];
            if dc == NONE {
                continue;
            }
            // Dependency names mapped to a bundled module id.
            let mut bundled: Vec<String> = Vec::new();
            let d = toks[deps].depth + 1;
            let mut i = deps + 1;
            while i + 2 < dc as usize {
                let key = &toks[i];
                if key.depth == d
                    && toks[i + 1].kind == TokKind::Colon
                    && toks[i + 2].kind == TokKind::Number
                {
                    let name = match key.kind {
                        TokKind::Str => args::literal(src, key)
                            .and_then(Result::ok)
                            .map(|s| s.into_owned()),
                        TokKind::Ident => {
                            std::str::from_utf8(key.text(src)).ok().map(str::to_owned)
                        }
                        _ => None,
                    };
                    bundled.extend(name);
                }
                i += 1;
            }
            let loader = toks[k + 3].text(src);
            if loader == b"require" {
                continue; // the main pass already sees `require('x')`
            }
            let mut n = 0;
            for j in body + 1..bc.saturating_sub(3) {
                if toks[j].kind != TokKind::Ident
                    || toks[j].text(src) != loader
                    || matches!(toks[j - 1].kind, TokKind::Dot | TokKind::OptChain)
                    || toks[j + 1].kind != TokKind::LParen
                    || toks[j + 2].kind != TokKind::Str
                    || toks[j + 3].kind != TokKind::RParen
                {
                    continue;
                }
                let Some(Ok(spec)) = args::literal(src, &toks[j + 2]) else {
                    continue;
                };
                if bundled.iter().any(|b| b == spec.as_ref()) {
                    continue;
                }
                let lname = String::from_utf8_lossy(loader);
                let r = RawRef::new(
                    RefKind::Require,
                    Specifier::Literal(spec),
                    Span::new(toks[j].start, toks[j + 3].end),
                )
                .with_flags(self.base_flags())
                .with_uncertainty(Uncertainty::new(
                    UncertaintyKind::AliasedLoader,
                    toks[j + 2].span(),
                    format!(
                        "browserify bundle: `{lname}` is the module's `require` and the \
                         dependency is not bundled"
                    ),
                ));
                self.ex.refs.push(r);
                n += 1;
                if n >= MAX_CALLS {
                    break;
                }
            }
        }
    }

    /// Module strings (see the module docs). Strings inside the span of a
    /// reference found so far are not scanned (import specifiers, arguments
    /// of tracked APIs, pino targets).
    fn deep_module_strings(&mut self, toks: &[Token], tree: &Tree) {
        let src = self.src;
        let covered: Vec<Span> = self.ex.refs.iter().map(|r| r.span).collect();
        let mut seen: HashSet<&'s [u8]> = HashSet::new();
        let mut out = Vec::new();
        for k in 1..toks.len() {
            let t = &toks[k];
            if !matches!(t.kind, TokKind::Str | TokKind::Template) {
                continue;
            }
            let p = tree.parent[k];
            if p == NONE {
                continue;
            }
            let p = p as usize;
            let prev = toks[k - 1].kind;
            let registry = match toks[p].kind {
                // `key: 'x'` in an object literal.
                TokKind::LBrace => prev == TokKind::Colon,
                // An array element; `obj['x']` and `f()['x']` are member accesses.
                TokKind::LBracket => {
                    matches!(prev, TokKind::LBracket | TokKind::Comma)
                        && !(p > 0
                            && (matches!(
                                toks[p - 1].kind,
                                TokKind::RParen | TokKind::RBracket | TokKind::Str
                            ) || (toks[p - 1].kind == TokKind::Ident
                                && toks[p - 1].kw == Kw::None)))
                }
                _ => false,
            };
            if !registry {
                continue;
            }
            let Some(content) = t.literal_content(src) else {
                continue;
            };
            if !module_string_shape(content)
                || covered.iter().any(|c| c.start <= t.start && t.end <= c.end)
                || !seen.insert(content)
            {
                continue;
            }
            let Ok(spec) = std::str::from_utf8(content) else {
                continue;
            };
            out.push(
                RawRef::new(
                    RefKind::DynamicImport,
                    Specifier::Literal(std::borrow::Cow::Borrowed(spec)),
                    t.span(),
                )
                .with_base(turborepo_trace_core::Base::Language(MODULE_STRING_BASE))
                .with_flags(RefFlags::LAZY | RefFlags::OPTIONAL | self.base_flags()),
            );
            if out.len() >= MAX_MODULE_STRINGS {
                break;
            }
        }
        self.ex.refs.extend(out);
    }

    /// pino transport targets (see the module docs).
    fn deep_pino(&mut self, toks: &[Token]) {
        let src = self.src;
        let mut n = 0;
        for j in 1..toks.len().saturating_sub(3) {
            let t = &toks[j];
            if t.kind != TokKind::Ident
                || !std::str::from_utf8(t.text(src)).is_ok_and(super::api::is_registry_key)
                || !matches!(toks[j - 1].kind, TokKind::LBrace | TokKind::Comma)
                || toks[j + 1].kind != TokKind::Colon
                || !matches!(toks[j + 2].kind, TokKind::Str | TokKind::Template)
                || !matches!(toks[j + 3].kind, TokKind::Comma | TokKind::RBrace)
            {
                continue;
            }
            let Some(Ok(spec)) = args::literal(src, &toks[j + 2]) else {
                continue;
            };
            if spec.is_empty() {
                continue;
            }
            let r = RawRef::new(
                RefKind::Require,
                Specifier::Literal(spec),
                Span::new(t.start, toks[j + 2].end),
            )
            .with_flags(RefFlags::OPTIONAL | self.base_flags())
            .with_uncertainty(Uncertainty::new(
                UncertaintyKind::AliasedLoader,
                toks[j + 2].span(),
                "pino transport `target`: loaded by pino in a worker thread",
            ));
            self.ex.refs.push(r);
            n += 1;
            if n >= MAX_CALLS {
                break;
            }
        }
    }
}

/// Token range `a..b` of argument `idx` of the call whose parentheses are `lp`
/// and `close`.
fn arg_range(toks: &[Token], lp: usize, close: usize, idx: usize) -> Option<(usize, usize)> {
    let d = toks[lp].depth + 1;
    let mut start = lp + 1;
    let mut index = 0;
    for (i, t) in toks.iter().enumerate().take(close).skip(lp + 1) {
        if t.depth == d && t.kind == TokKind::Comma {
            if index == idx {
                return (i > start).then_some((start, i));
            }
            index += 1;
            start = i + 1;
        }
    }
    (index == idx && close > start).then_some((start, close))
}
