//! Where a file's own location and the working directory go: the anchored names
//! of a file (`turborepo_trace_core::names::Anchored`).
//!
//! The token scan ([`crate::names`]) knows that a file uses `__dirname`,
//! `import.meta.url` or `process.cwd()` other than in a literal path, and has
//! to assume the worst: the file can produce any path below that directory,
//! raised by every `..` and `dirname` of the file. This module follows each
//! such use ([`AnchorUse`]) through the syntax tree instead:
//!
//! 1. the *maximal path expression* around it: the largest expression whose
//!    value the engine still evaluates to something anchored
//!    (`path.join(__dirname, '..', x)`, `` `${process.cwd()}/tmp` ``, `new
//!    URL('.', import.meta.url).pathname`);
//! 2. what consumes that expression:
//!    - an argument of a site the recognizer evaluates (a file API, a module
//!      load): with a known path the site's reference says what is read and
//!      nothing is named; with an unknown tail the reference is a pattern that
//!      names bound, so the directory the path is known to be below is named;
//!    - the initializer of a binding: every reference of the binding is
//!      followed in turn;
//!    - an element of an array literal that is iterated (`for…of`; `.forEach`,
//!      `.map`, `.flatMap`, `.some`, `.every` with an arrow function): the uses
//!      of the loop variable or callback parameter are followed (what `.map`
//!      returns is what the callback returns, and a returned value escapes;
//!      `.filter` and `.find` hand the elements on, so with them the array
//!      escapes);
//!    - a directory handed to a function known to read only fixed file names in
//!      it ([`DIR_CONSUMERS`]: `dotenv-flow` reads `.env*` files directly in
//!      `path`): an entry with that pattern;
//!    - a value that is only tested or dropped: nothing. Tests are a condition,
//!      a comparison, `.length`, and the string searches the token scan also
//!      treats as tests (`startsWith`, `endsWith`, `includes`, `indexOf`,
//!      `lastIndexOf`, as the receiver or as the argument), and what an
//!      assertion of a test runner compares: the subject and the arguments of
//!      `expect(a).toBe(b)` and the other comparing matchers, and the arguments
//!      of `expect.objectContaining(...)` and its like, also as a property or
//!      an element of a literal given to them (`toMatchFileSnapshot`, which
//!      reads a file, is not one of them);
//!    - a path that is only written: the path argument of `fs` functions that
//!      write, create or remove (`writeFile`, `appendFile`, `rm`, `unlink`,
//!      `mkdir`, the destination of `copyFile` / `cp`, and `fs-extra`'s
//!      `outputFile`, `ensureDir`, `remove`, ...). Nothing is read there by the
//!      call; `mkdir` and its like can return the path they made, so they count
//!      only where the result is not used;
//!    - the first argument of `path.relative(from, to)`: the result is `to`
//!      seen from `from`, a path that has no base any more and whose text comes
//!      from `to` (whoever builds `to` names it). As the second argument the
//!      base is what the result leads to: an escape;
//!    - an argument of a function of the file (`Walk::follow_into`): the uses
//!      of the parameter are followed in turn, like a binding's; so are the
//!      uses of a parameter the value is the default of;
//!    - a message: an argument of a `console` method or of an error constructor
//!      (`new Error(...)`, `TypeError(...)`), directly or inside the text built
//!      for it (a template, a `+`). The text goes to a stream or an error
//!      value, not into a path (the Go scan leaves the same calls out,
//!      `turborepo_trace_lang_go::names`);
//!    - the working directory of a child process (`{ cwd }` in the options of
//!      `exec`, `execSync`, `execFile`, `execFileSync`, `spawn` and `spawnSync`
//!      of `child_process`): this process reads nothing there. What the child
//!      reads through its command line is named: with a command made of known
//!      strings, every word of it that is a relative path is an entry below
//!      that directory; with a part that is not known (or that the shell
//!      expands), everything below the directory, as for an escape. What the
//!      program reads on its own account is not followed, with or without `cwd`
//!      (`Walk::child_cwd`);
//!    - anything else (an argument of code that is not evaluated, a return
//!      value, an export, a property): the value *escapes*, and the file names
//!      everything at or below the directory it has there.
//!
//! An escaping path whose unknown tail is text from the environment
//! (`process.env.X`, a call of an imported function that is given a variable
//! name, `getEnv('KEYS_PATH', d)`) is the environment registry's
//! (`turborepo-trace-store`, `env`): it is reported as a use ([`EnvEscape`]),
//! and its entry is conditional on what the registry leaves of the record. (The
//! registry decides whether the function is an environment helper; when it
//! is not, the record stays a computed read and the entry counts.)
//!
//! A use that cannot be followed (no expression there, the engine does not see
//! the anchor in the value, too many steps) leaves the file's coarse mark for
//! that base in place: [`Followed::location`] or [`Followed::cwd`] stay false.
//!
//! # Assumptions
//!
//! As for the token scan: an unknown tail does not climb (`join(__dirname, x)`
//! stays below the directory, apart from the parent steps the file has
//! elsewhere, which are applied at query time), and code that receives a path
//! does not cut it by other means than `dirname` and `..`. A method called
//! `startsWith`, `endsWith`, `includes`, `indexOf` or `lastIndexOf` only
//! searches, whatever its receiver is; the text of a log line or an
//! error message is not parsed back into a path; and a path does not come back
//! from a child process (its output is text from outside, like the
//! environment's).
//!
//! Each of these consumers can be switched off for an A/B measurement
//! ([`crate::names::rule_off`]: `relative`, `tests`, `local`, `message`,
//! `child`, `writes`).

use std::collections::HashSet;

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{NodeId, SymbolId};
use oxc_span::GetSpan;
use turborepo_trace_core::{
    Origin,
    env::EnvValue,
    eval::{PatPart, Value},
    names::{Anchored, EVERYTHING_BELOW},
};

use super::{
    engine::Engine,
    value::{Builtin, FnRef, V},
};
use crate::{
    extract::{
        evaluator::{self, Anchor, Func, JsVal, Obj},
        origin,
    },
    names::{AnchorUse, Followed},
};

/// Most expressions followed per file.
const MAX_STEPS: u32 = 512;
/// Most bindings a value is followed through.
const MAX_DEPTH: u32 = 8;

/// A function of a package that takes a directory (or a file) and reads only
/// what the entry says there.
struct DirConsumer {
    module: &'static str,
    /// The export path (`config`, `default.config`).
    exports: &'static [&'static str],
    /// The argument that holds the path.
    arg: usize,
    /// The property of that argument that holds the path; `None`: the argument
    /// itself.
    option: Option<&'static str>,
    /// Properties that change which files are read: with one of them the call
    /// is an ordinary escape.
    unless: &'static [&'static str],
    /// The file names read directly in the directory; `None`: the path is the
    /// file.
    pattern: Option<&'static str>,
}

/// The table of directory consumers.
///
/// - `dotenv-flow`: `config({ path })` and `listFiles({ path })` read `.env`,
///   `.env.<node_env>`, `.env.local`, `.env.<node_env>.local` and
///   `.env.defaults` in the directory `path`, unless `pattern` or `files` name
///   other files.
/// - `dotenv`: `config({ path })` reads the file `path` (or `<path>.vault` with
///   a key).
const DIR_CONSUMERS: &[DirConsumer] = &[
    DirConsumer {
        module: "dotenv-flow",
        exports: &["config", "default.config", "listFiles", "default.listFiles"],
        arg: 0,
        option: Some("path"),
        unless: &["pattern", "files"],
        pattern: Some(".env*"),
    },
    DirConsumer {
        module: "dotenv",
        exports: &["config", "default.config", "configDotenv"],
        arg: 0,
        option: Some("path"),
        unless: &[],
        pattern: None,
    },
];

/// An escaping path whose unknown tail is environment text: a use for the
/// environment registry (`turborepo_trace_core::env::EnvFact::Use`) with its
/// record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvEscape {
    /// The expression.
    pub span: turborepo_trace_core::Span,
    /// `Env` or `Call` (a call of an imported function that may be an
    /// environment helper).
    pub origin: Origin,
    /// The value as a template over variables and calls.
    pub value: EnvValue,
    /// The source text of the expression, for the record.
    pub detail: String,
}

/// The result of following the anchor uses of a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Anchors {
    /// Which bases were followed completely, and where they go.
    pub followed: Followed,
    /// The environment-tailed escapes of the followed bases.
    pub escapes: Vec<EnvEscape>,
}

/// One entry before lines are known.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    cwd: bool,
    at: u32,
    rel: String,
    pattern: String,
    conditional: bool,
}

struct Walk<'e, 'a, 's, 'r> {
    engine: &'e mut Engine<'a, 's>,
    sites: &'r [turborepo_trace_core::Span],
    entries: Vec<Entry>,
    /// Evaluated expressions and the base that was followed through them.
    covered: Vec<(u32, u32, bool)>,
    /// With the base followed and the leaves the value is made of.
    escapes: Vec<(EnvEscape, bool, Vec<JsVal>)>,
    /// Per base (location, working directory): a use could not be followed.
    failed: [bool; 2],
    seen: HashSet<(SymbolId, bool)>,
    steps: u32,
}

/// Follows `uses` (from [`crate::names::scan_parts`]). `sites` are the argument
/// spans of the sites the recognizer evaluates.
pub(super) fn follow<'a>(
    engine: &mut Engine<'a, '_>,
    text: &str,
    uses: &[AnchorUse],
    sites: &[turborepo_trace_core::Span],
    templates: &[u32],
) -> Anchors {
    let leads = leading(engine, templates);
    let starts = start_nodes(engine, uses);
    let mut w = Walk {
        engine,
        sites,
        entries: Vec::new(),
        covered: Vec::new(),
        escapes: Vec::new(),
        failed: [false; 2],
        seen: HashSet::new(),
        steps: 0,
    };
    for (u, start) in uses.iter().zip(starts) {
        match start {
            Some(n) => w.from(n, u.cwd, 0),
            None => w.failed[usize::from(u.cwd)] = true,
        }
    }
    let done = |cwd: bool| uses.iter().any(|u| u.cwd == cwd) && !w.failed[usize::from(cwd)];
    let (location, cwd) = (done(false), done(true));
    let kept = |c: bool| if c { cwd } else { location };
    let line_of = |at: u32| {
        1 + memchr::memchr_iter(b'\n', &text.as_bytes()[..(at as usize).min(text.len())]).count()
            as u32
    };
    let mut entries: Vec<Anchored> = Vec::new();
    for e in w.entries.iter().filter(|e| kept(e.cwd)) {
        let a = Anchored {
            cwd: e.cwd,
            line: line_of(e.at),
            rel: e.rel.as_str().into(),
            pattern: e.pattern.as_str().into(),
            conditional: e.conditional,
        };
        // An unconditional entry says more than a conditional one for the same place.
        if !entries.iter().any(|x| {
            (x.cwd, &x.rel, &x.pattern, x.line) == (a.cwd, &a.rel, &a.pattern, a.line)
                && (!x.conditional || a.conditional)
        }) {
            entries.retain(|x| {
                (x.cwd, &x.rel, &x.pattern, x.line) != (a.cwd, &a.rel, &a.pattern, a.line)
            });
            entries.push(a);
        }
    }
    let mut escapes: Vec<EnvEscape> = Vec::new();
    for (e, c, _) in w.escapes {
        if kept(c)
            && !escapes
                .iter()
                .any(|x| x.span == e.span && x.origin == e.origin)
        {
            escapes.push(e);
        }
    }
    Anchors {
        followed: Followed {
            location,
            cwd,
            covered: w
                .covered
                .iter()
                .filter(|c| kept(c.2))
                .map(|c| (c.0, c.1))
                .collect(),
            entries,
            leads,
        },
        escapes,
    }
}

/// Most values of a leading hole.
const MAX_LEAD_VALUES: usize = 8;

/// The values of the first hole of the templates at the byte offsets
/// `templates`, for those where every alternative is a known string.
fn leading<'a>(engine: &mut Engine<'a, '_>, templates: &[u32]) -> Vec<(u32, Vec<String>)> {
    if templates.is_empty() || crate::names::rule_off("leads") {
        return Vec::new();
    }
    let mut firsts: Vec<(u32, &'a Expression<'a>)> = Vec::new();
    for node in engine.nodes.iter() {
        if let AstKind::TemplateLiteral(t) = node.kind()
            && templates.contains(&t.span.start)
            && let Some(e) = t.expressions.first()
        {
            firsts.push((t.span.start, e));
        }
    }
    let mut out = Vec::new();
    for (at, e) in firsts {
        let Some(v) = engine.site(e) else { continue };
        let mut values = Vec::new();
        let known = evaluator::split_alts(super::to_js(v))
            .into_iter()
            .all(|l| match l {
                JsVal::Str(Value::Str(s)) => {
                    if !values.contains(&s) {
                        values.push(s);
                    }
                    true
                }
                _ => false,
            });
        if known && !values.is_empty() && values.len() <= MAX_LEAD_VALUES {
            out.push((at, values));
        }
    }
    out
}

/// The expression node each use starts from: the `__dirname` / `__filename`
/// reference, `import.meta`, the call `process.cwd()`, or the member
/// `process.env.PWD`.
fn start_nodes(engine: &Engine<'_, '_>, uses: &[AnchorUse]) -> Vec<Option<NodeId>> {
    let mut out = vec![None; uses.len()];
    let find = |at: u32, cwd: bool| uses.iter().position(|u| u.at == at && u.cwd == cwd);
    for node in engine.nodes.iter() {
        match node.kind() {
            AstKind::IdentifierReference(id) => {
                if let Some(i) = find(id.span.start, false) {
                    out[i] = Some(node.id());
                }
            }
            AstKind::ImportMeta(m) => {
                for (i, u) in uses.iter().enumerate() {
                    if !u.cwd && m.span.start <= u.at && u.at < m.span.end {
                        out[i] = Some(node.id());
                    }
                }
            }
            AstKind::StaticMemberExpression(m) => {
                if let Some(i) = find(m.property.span.start, true) {
                    // `process.cwd()`: the call.
                    let (pid, pk) = engine.parent(node.id());
                    out[i] = Some(match pk {
                        AstKind::CallExpression(c)
                            if c.callee.get_inner_expression().span() == m.span =>
                        {
                            pid
                        }
                        _ => node.id(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// The leaves of `val` that are anchored: the base and the relative part
/// (`None`: the file itself or its URL).
fn anchored_leaves(val: &JsVal) -> Vec<(bool, Option<Value>, JsVal)> {
    let mut out = Vec::new();
    for leaf in evaluator::split_alts(val.clone()) {
        let found = match &leaf {
            JsVal::Path(Anchor::FileDir, v) | JsVal::Url(Anchor::FileDir, v) => {
                Some((false, Some(v.clone())))
            }
            JsVal::Path(Anchor::Cwd, v) | JsVal::Url(Anchor::Cwd, v) => {
                Some((true, Some(v.clone())))
            }
            JsVal::File | JsVal::FileUrl | JsVal::Obj(Obj::ImportMeta) => Some((false, None)),
            _ => None,
        };
        if let Some((cwd, v)) = found {
            out.push((cwd, v, leaf));
        }
    }
    out
}

fn is_anchored(val: &JsVal, cwd: bool) -> bool {
    anchored_leaves(val).iter().any(|l| l.0 == cwd)
}

/// The directory part and the reasons of the unknown tail of the relative part
/// `v` of an anchored path.
fn prefix_of(v: &Value) -> (String, Vec<Box<str>>) {
    let clean = |s: &str| s.trim_matches('/').to_owned();
    match evaluator::normalize_value(v.clone(), true) {
        Value::Str(s) => (clean(&s), Vec::new()),
        v @ Value::Pattern(_) => {
            let mut reasons = Vec::new();
            evaluator::wild_reasons(&v, &mut reasons);
            let Value::Pattern(parts) = &v else {
                return (String::new(), reasons);
            };
            let dir = match parts.first() {
                // The literal start up to its last separator: what follows is a partial
                // segment.
                Some(PatPart::Lit(s)) => s.rfind('/').map_or(String::new(), |i| clean(&s[..i])),
                _ => String::new(),
            };
            if reasons.is_empty() {
                reasons.push("".into());
            }
            (dir, reasons)
        }
        v => {
            let mut reasons = Vec::new();
            evaluator::wild_reasons(&v, &mut reasons);
            if reasons.is_empty() {
                reasons.push("".into());
            }
            (String::new(), reasons)
        }
    }
}

impl<'a> Walk<'_, 'a, '_, '_> {
    fn fail(&mut self, cwd: bool) {
        self.failed[usize::from(cwd)] = true;
    }

    fn value(&mut self, node: NodeId) -> Option<JsVal> {
        let kind = self.engine.nodes.kind(node);
        self.engine.site_node(kind).map(super::to_js)
    }

    fn in_site(&self, span: oxc_span::Span) -> bool {
        self.sites
            .iter()
            .any(|s| s.start <= span.start && span.end <= s.end)
    }

    /// Whether the callee is a function the engine evaluates itself
    /// (`path.join`, `fileURLToPath`, `new URL`): its result is all that
    /// becomes of the arguments. A `.join` or `.resolve` of an unknown
    /// object that the engine takes for `path`'s is not one.
    fn evaluated_callee(&mut self, callee: &'a Expression<'a>) -> bool {
        match self.engine.site(callee) {
            Some(V::Js(JsVal::Func(f))) => !matches!(f, Func::GuessJoin | Func::GuessResolve),
            Some(V::Js(JsVal::Obj(Obj::UrlCtor))) => true,
            _ => false,
        }
    }

    /// Whether the parent `pk` of the expression spanning `child` derives its
    /// value from it, as far as the syntax says.
    fn derives(&mut self, pk: AstKind<'a>, child: oxc_span::Span) -> bool {
        let is = |e: &Expression<'a>| e.get_inner_expression().span() == child;
        match pk {
            AstKind::CallExpression(c) => {
                if is(&c.callee) {
                    return true;
                }
                c.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(is))
                    && self.evaluated_callee(&c.callee)
            }
            AstKind::NewExpression(n) => {
                n.arguments
                    .iter()
                    .any(|a| a.as_expression().is_some_and(is))
                    && self.evaluated_callee(&n.callee)
            }
            AstKind::StaticMemberExpression(m) => is(&m.object),
            AstKind::BinaryExpression(b) => b.operator == BinaryOperator::Addition,
            AstKind::ConditionalExpression(c) => !is(&c.test),
            AstKind::TemplateLiteral(_)
            | AstKind::AwaitExpression(_)
            | AstKind::ChainExpression(_)
            | AstKind::LogicalExpression(_) => true,
            _ => false,
        }
    }

    /// Follows the anchored expression at `node`.
    fn from(&mut self, node: NodeId, cwd: bool, depth: u32) {
        if self.failed[usize::from(cwd)] {
            return;
        }
        self.steps += 1;
        if self.steps > MAX_STEPS || depth > MAX_DEPTH {
            return self.fail(cwd);
        }
        let Some(mut val) = self.value(node) else {
            return self.fail(cwd);
        };
        if !is_anchored(&val, cwd) {
            // The engine does not see the anchor here.
            return self.fail(cwd);
        }
        // The maximal path expression.
        let mut cur = node;
        loop {
            let span = self.engine.nodes.kind(cur).span();
            let (pid, pk) = self.engine.parent(cur);
            if pid == cur || !self.derives(pk, span) {
                break;
            }
            match self.value(pid) {
                Some(v) if is_anchored(&v, cwd) => {
                    cur = pid;
                    val = v;
                }
                _ => break,
            }
        }
        let span = self.engine.nodes.kind(cur).span();
        self.covered.push((span.start, span.end, cwd));
        self.consume(cur, &val, cwd, depth);
    }

    /// What becomes of the expression `cur` with value `val`.
    fn consume(&mut self, cur: NodeId, val: &JsVal, cwd: bool, depth: u32) {
        let span = self.engine.nodes.kind(cur).span();
        if self.in_site(span) {
            // The read's own reference says what is read. When the tail is unknown that
            // reference is a pattern (or a record the names bound): the known directory
            // is its name. The read has its own record: no use is added.
            for (base, v, _) in anchored_leaves(val) {
                let (dir, reasons) = v.as_ref().map(prefix_of).unwrap_or_default();
                if !reasons.is_empty() {
                    self.entry(base, span.start, dir, EVERYTHING_BELOW, false);
                }
            }
            return;
        }
        let is = |e: &Expression<'a>| e.get_inner_expression().span() == span;
        let (pid, pk) = self.engine.parent(cur);
        match pk {
            AstKind::VariableDeclarator(d) if d.init.as_ref().is_some_and(is) => {
                match binding_symbol(&d.id) {
                    Some(sym) => self.symbol(sym, Some(pid), span, val, cwd, depth),
                    None => self.escape(span, val, cwd),
                }
            }
            AstKind::ArrayExpression(_) => self.carried(pid, None, val, cwd, depth),
            AstKind::ObjectProperty(p) if !p.computed && is(&p.value) => {
                let key = p
                    .key
                    .static_name()
                    .and_then(|k| k.as_str().map(str::to_owned));
                let (oid, ok) = self.engine.parent(pid);
                match (key, ok) {
                    (Some(key), AstKind::ObjectExpression(_)) => {
                        self.carried(oid, Some(&key), val, cwd, depth)
                    }
                    _ => self.escape(span, val, cwd),
                }
            }
            AstKind::CallExpression(c) => {
                let index = c
                    .arguments
                    .iter()
                    .position(|a| a.as_expression().is_some_and(is));
                if let Some(i) = index {
                    match self.argument(c, pid, i) {
                        Passed::Dropped => return,
                        Passed::Function(f) => {
                            if self.follow_into(f, i, span, val, cwd, depth) {
                                return;
                            }
                        }
                        Passed::Other => {}
                    }
                }
                match index.and_then(|i| self.consumer(c, i, None)) {
                    Some(t) => self.table(t, span, val, cwd),
                    None => self.escape(span, val, cwd),
                }
            }
            // A message: `new Error(dir)`.
            AstKind::NewExpression(n)
                if error_ctor(self.engine, &n.callee)
                    && n.arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(is))
                    && !crate::names::rule_off("message") => {}
            // `dir.length`, `dir.startsWith(x)`: a number, a truth value.
            AstKind::StaticMemberExpression(m)
                if is(&m.object) && self.only_tests(pid, m) && !crate::names::rule_off("tests") => {
            }
            // The default of a parameter: the parameter's uses.
            AstKind::FormalParameter(p)
                if p.initializer.as_ref().is_some_and(|e| is(e))
                    && !crate::names::rule_off("local") =>
            {
                match binding_symbol(&p.pattern) {
                    Some(sym) => self.symbol(sym, None, span, val, cwd, depth),
                    None => self.escape(span, val, cwd),
                }
            }
            // Text that is no path any more (the engine did not keep the anchor): a
            // message when that is all it is built for.
            AstKind::TemplateLiteral(_) if self.in_message(pid) => {}
            AstKind::BinaryExpression(b)
                if b.operator == BinaryOperator::Addition && self.in_message(pid) => {}
            // The value is dropped or only tested.
            AstKind::ExpressionStatement(_)
            | AstKind::IfStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::UnaryExpression(_) => {}
            AstKind::BinaryExpression(b) if b.operator != BinaryOperator::Addition => {}
            AstKind::ConditionalExpression(c) if is(&c.test) => {}
            _ => self.escape(span, val, cwd),
        }
    }

    /// The value is bound to `sym` (declared at `decl`): every reference is
    /// followed.
    fn symbol(
        &mut self,
        sym: SymbolId,
        decl: Option<NodeId>,
        span: oxc_span::Span,
        val: &JsVal,
        cwd: bool,
        depth: u32,
    ) {
        if !self.seen.insert((sym, cwd)) {
            return;
        }
        if decl.is_some_and(|d| self.exported(d)) {
            // `export const dir = ...`: other modules get it.
            self.escape(span, val, cwd);
        }
        let scoping = self.engine.scoping;
        let refs: Vec<NodeId> = scoping
            .get_resolved_references(sym)
            .filter(|r| r.is_read())
            .map(|r| r.node_id())
            .collect();
        for r in refs {
            self.from(r, cwd, depth + 1);
        }
    }

    /// Whether the declarator `decl` is part of an `export` declaration.
    fn exported(&self, decl: NodeId) -> bool {
        let (p, pk) = self.engine.parent(decl);
        matches!(pk, AstKind::VariableDeclaration(_))
            && matches!(
                self.engine.nodes.parent_kind(p),
                AstKind::ExportDeclaration(_)
                    | AstKind::ExportNamedDeclaration(_)
                    | AstKind::ExportDefaultDeclaration(_)
            )
    }

    /// The value is an element of the array literal `container` (`key` is
    /// `None`) or the property `key` of the object literal `container`.
    fn carried(
        &mut self,
        container: NodeId,
        key: Option<&str>,
        val: &JsVal,
        cwd: bool,
        depth: u32,
    ) {
        let ck = self.engine.nodes.kind(container);
        let span = ck.span();
        let is = |e: &Expression<'a>| e.get_inner_expression().span() == span;
        let (pid, pk) = self.engine.parent(container);
        match (pk, key) {
            // Part of what an assertion compares with.
            (AstKind::CallExpression(c), _)
                if self.matcher(c) && !crate::names::rule_off("tests") => {}
            // An option of a call.
            (AstKind::CallExpression(c), Some(k)) => {
                let index = c
                    .arguments
                    .iter()
                    .position(|a| a.as_expression().is_some_and(is));
                if k == "cwd"
                    && let Some(i) = index
                    && self.child_cwd(c, i, span, val, cwd)
                {
                    return;
                }
                let plain = match ck {
                    AstKind::ObjectExpression(o) => Some(o),
                    _ => None,
                };
                let found = index
                    .and_then(|i| self.consumer(c, i, Some(k)))
                    .filter(|t| {
                        plain.is_some_and(|o| {
                            o.properties.iter().all(|p| match p {
                                ObjectPropertyKind::ObjectProperty(p) => p
                                    .key
                                    .static_name()
                                    .and_then(|n| n.as_str().map(str::to_owned))
                                    .is_some_and(|n| !t.unless.contains(&n.as_str())),
                                ObjectPropertyKind::SpreadProperty(_) => false,
                            })
                        })
                    });
                match found {
                    Some(t) => self.table(t, span, val, cwd),
                    None => self.escape(span, val, cwd),
                }
            }
            (AstKind::VariableDeclarator(d), None) if d.init.as_ref().is_some_and(is) => {
                let Some(sym) = binding_symbol(&d.id) else {
                    return self.escape(span, val, cwd);
                };
                if !self.seen.insert((sym, cwd)) {
                    return;
                }
                if self.exported(pid) {
                    self.escape(span, val, cwd);
                }
                let scoping = self.engine.scoping;
                let refs: Vec<NodeId> = scoping
                    .get_resolved_references(sym)
                    .filter(|r| r.is_read())
                    .map(|r| r.node_id())
                    .collect();
                for r in refs {
                    self.iterated(r, val, cwd, depth);
                }
            }
            (AstKind::ForOfStatement(_) | AstKind::StaticMemberExpression(_), None) => {
                self.iterated(container, val, cwd, depth)
            }
            _ => self.escape(span, val, cwd),
        }
    }

    /// `array` (an array literal carrying the value, or a reference to its
    /// binding) is iterated: the loop variable or the callback's parameter
    /// gets the elements.
    fn iterated(&mut self, array: NodeId, val: &JsVal, cwd: bool, depth: u32) {
        let span = self.engine.nodes.kind(array).span();
        let is = |e: &Expression<'a>| e.get_inner_expression().span() == span;
        let (pid, pk) = self.engine.parent(array);
        let element = match pk {
            AstKind::ForOfStatement(f) if is(&f.right) => match &f.left {
                ForStatementLeft::VariableDeclaration(d) if d.declarations.len() == 1 => {
                    d.declarations.first().and_then(|d| binding_symbol(&d.id))
                }
                _ => None,
            },
            AstKind::StaticMemberExpression(m)
                if is(&m.object)
                    && matches!(
                        m.property.name.as_str(),
                        "forEach" | "map" | "flatMap" | "some" | "every"
                    ) =>
            {
                let mspan = m.span;
                match self.engine.parent(pid).1 {
                    AstKind::CallExpression(c)
                        if c.callee.get_inner_expression().span() == mspan =>
                    {
                        // An arrow function has no `arguments` of its own.
                        let params = match c.arguments.first().and_then(|a| a.as_expression()) {
                            Some(Expression::ArrowFunctionExpression(f)) => Some(&f.params),
                            _ => None,
                        };
                        params
                            .and_then(|p| p.items.first())
                            .and_then(|p| binding_symbol(&p.pattern))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        match element {
            Some(sym) => self.symbol(sym, None, span, val, cwd, depth),
            None => self.escape(span, val, cwd),
        }
    }

    /// What the call `c` does with its argument `index`, as far as the callee
    /// says.
    fn argument(&mut self, c: &'a CallExpression<'a>, call: NodeId, index: usize) -> Passed<'a> {
        if !crate::names::rule_off("tests") && (self.matcher(c) || self.asserted(c, call)) {
            return Passed::Dropped;
        }
        if let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression() {
            if search_method(m.property.name.as_str()) && !crate::names::rule_off("tests") {
                return Passed::Dropped;
            }
            if is_global(self.engine, &m.object, "console") && !crate::names::rule_off("message") {
                return Passed::Dropped;
            }
        }
        if error_ctor(self.engine, &c.callee) && !crate::names::rule_off("message") {
            return Passed::Dropped;
        }
        // A spread before the argument: its position in the callee is not known.
        if c.arguments[..index]
            .iter()
            .any(|a| matches!(a, Argument::SpreadElement(_)))
        {
            return Passed::Other;
        }
        match self.engine.site(&c.callee) {
            Some(V::Js(JsVal::Func(Func::PathRelative)))
                if index == 0 && !crate::names::rule_off("relative") =>
            {
                Passed::Dropped
            }
            Some(V::Func(f)) if !crate::names::rule_off("local") => Passed::Function(f),
            // A path this process only writes to.
            Some(V::Method(Builtin::Fs | Builtin::FsPromises, name))
                if written(&name, index)
                    && (!returns_path(&name) || self.dropped(call))
                    && !crate::names::rule_off("writes") =>
            {
                Passed::Dropped
            }
            _ => Passed::Other,
        }
    }

    /// Whether the value of the call at `call` is not used: a statement, also
    /// awaited.
    fn dropped(&self, call: NodeId) -> bool {
        let (pid, pk) = self.engine.parent(call);
        match pk {
            AstKind::ExpressionStatement(_) => true,
            AstKind::AwaitExpression(_) => {
                matches!(self.engine.parent(pid).1, AstKind::ExpressionStatement(_))
            }
            _ => false,
        }
    }

    /// Whether `e` is the `expect` of a test runner: the global, or the import.
    fn is_expect(&mut self, e: &'a Expression<'a>) -> bool {
        let Expression::Identifier(id) = e.get_inner_expression() else {
            return false;
        };
        if id.name != "expect" {
            return false;
        }
        is_global(self.engine, e, "expect")
            || matches!(
                self.engine.site(e),
                Some(V::Import(i))
                    if &*i.name == "expect"
                        && matches!(&*i.module, "vitest" | "@jest/globals" | "bun:test")
            )
    }

    /// Whether `c` is a call of a comparing matcher: `expect(x).toBe(…)` (also
    /// behind `.not`, `.resolves`, `.rejects`) or
    /// `expect.objectContaining(…)`. Its arguments are what the tested
    /// value is compared with.
    fn matcher(&mut self, c: &'a CallExpression<'a>) -> bool {
        let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression() else {
            return false;
        };
        let name = m.property.name.as_str();
        let mut object = m.object.get_inner_expression();
        while let Expression::StaticMemberExpression(o) = object {
            if !matches!(o.property.name.as_str(), "not" | "resolves" | "rejects") {
                return false;
            }
            object = o.object.get_inner_expression();
        }
        match object {
            Expression::CallExpression(subject) => {
                comparing_matcher(name) && self.is_expect(&subject.callee)
            }
            _ => asymmetric_matcher(name) && self.is_expect(object),
        }
    }

    /// Whether `c` (the node `call`) is `expect(value)` followed by a comparing
    /// matcher: the value is only compared.
    fn asserted(&mut self, c: &'a CallExpression<'a>, call: NodeId) -> bool {
        if !self.is_expect(&c.callee) {
            return false;
        }
        let mut cur = call;
        loop {
            let span = self.engine.nodes.kind(cur).span();
            let (pid, pk) = self.engine.parent(cur);
            let AstKind::StaticMemberExpression(m) = pk else {
                return false;
            };
            if m.object.get_inner_expression().span() != span {
                return false;
            }
            match m.property.name.as_str() {
                "not" | "resolves" | "rejects" => cur = pid,
                name => {
                    return comparing_matcher(name)
                        && matches!(
                            self.engine.parent(pid).1,
                            AstKind::CallExpression(mc)
                                if mc.callee.get_inner_expression().span() == m.span
                        );
                }
            }
        }
    }

    /// The value is argument `index` of a call of the file's function `f`: the
    /// uses of that parameter are followed. `false` (and nothing is kept of
    /// the attempt) when the parameter is not a plain name, the function
    /// reads `arguments`, or a use inside cannot be followed: the caller
    /// then treats the call as an escape.
    fn follow_into(
        &mut self,
        f: FnRef<'a>,
        index: usize,
        span: oxc_span::Span,
        val: &JsVal,
        cwd: bool,
        depth: u32,
    ) -> bool {
        let items = match f {
            FnRef::Function(f) => &f.params.items,
            FnRef::Arrow(f) => &f.params.items,
        };
        let Some(sym) = items.get(index).and_then(|p| binding_symbol(&p.pattern)) else {
            return false;
        };
        if self.engine.facts().uses_arguments(f.span().start) {
            return false;
        }
        let before = (
            self.entries.len(),
            self.covered.len(),
            self.escapes.len(),
            self.failed,
            self.seen.clone(),
        );
        self.symbol(sym, None, span, val, cwd, depth);
        if self.failed == before.3 {
            return true;
        }
        self.entries.truncate(before.0);
        self.covered.truncate(before.1);
        self.escapes.truncate(before.2);
        self.failed = before.3;
        self.seen = before.4;
        false
    }

    /// Whether the member `m` of the value (the node `member`) only tests it:
    /// `.length`, or a call of one of its searching methods.
    fn only_tests(&self, member: NodeId, m: &StaticMemberExpression<'a>) -> bool {
        let name = m.property.name.as_str();
        if name == "length" {
            return true;
        }
        search_method(name)
            && matches!(
                self.engine.parent(member).1,
                AstKind::CallExpression(c) if c.callee.get_inner_expression().span() == m.span
            )
    }

    /// Whether the text built at `node` (a template or a concatenation that is
    /// no path) is an argument of a message function and nothing else.
    fn in_message(&self, node: NodeId) -> bool {
        if crate::names::rule_off("message") {
            return false;
        }
        let mut cur = node;
        loop {
            let span = self.engine.nodes.kind(cur).span();
            let is = |e: &Expression<'a>| e.get_inner_expression().span() == span;
            let (pid, pk) = self.engine.parent(cur);
            match pk {
                AstKind::TemplateLiteral(_) => cur = pid,
                AstKind::BinaryExpression(b) if b.operator == BinaryOperator::Addition => cur = pid,
                AstKind::CallExpression(c) => {
                    let arg = c
                        .arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(is));
                    let console = matches!(
                        c.callee.get_inner_expression(),
                        Expression::StaticMemberExpression(m)
                            if is_global(self.engine, &m.object, "console")
                    );
                    return arg && (console || error_ctor(self.engine, &c.callee));
                }
                AstKind::NewExpression(n) => {
                    return n
                        .arguments
                        .iter()
                        .any(|a| a.as_expression().is_some_and(is))
                        && error_ctor(self.engine, &n.callee);
                }
                _ => return false,
            }
        }
    }

    /// The value is the `cwd` option (in the options object at argument
    /// `options`) of the call `c`. `true` when `c` starts a child process
    /// and the entries are made: the words of a known command line that are
    /// relative paths, each below the directory; everything below it when a
    /// part of the command is not known.
    ///
    /// This process reads nothing through the value, so it names nothing by
    /// itself. What the child reads on its own is not modelled, here or for
    /// a call without `cwd` (a child process is outside the model: README,
    /// "Limitations"): the rule assumes nothing about it, and only the
    /// command line's own words are names.
    fn child_cwd(
        &mut self,
        c: &'a CallExpression<'a>,
        options: usize,
        at: oxc_span::Span,
        val: &JsVal,
        cwd: bool,
    ) -> bool {
        if crate::names::rule_off("child") {
            return false;
        }
        let Some(V::Import(i)) = self.engine.site(&c.callee) else {
            return false;
        };
        let module = i.module.strip_prefix("node:").unwrap_or(&i.module);
        let name = i.name.strip_prefix("default.").unwrap_or(&i.name);
        if module != "child_process"
            || !matches!(
                name,
                "exec" | "execSync" | "execFile" | "execFileSync" | "spawn" | "spawnSync"
            )
        {
            return false;
        }
        let leaves = anchored_leaves(val);
        if !leaves.iter().any(|l| l.0 == cwd) {
            return false;
        }
        // The command line: every argument before the options.
        let mut words: Option<Vec<String>> = Some(Vec::new());
        for a in &c.arguments[..options] {
            let v = a.as_expression().and_then(|e| self.engine.site(e));
            let known = match (&v, &mut words) {
                (Some(v), Some(w)) => command_words(v, w),
                _ => false,
            };
            if !known {
                words = None;
            }
        }
        for (base, v, leaf) in leaves {
            let dir = v.as_ref().map(prefix_of).filter(|p| p.1.is_empty());
            match (&words, dir) {
                (Some(words), Some((dir, _))) => {
                    for w in words {
                        let joined =
                            evaluator::normalize_value(Value::str(format!("{dir}/{w}")), true);
                        match joined.as_str() {
                            Some(rel) => self.entry(
                                base,
                                at.start,
                                rel.trim_matches('/').to_owned(),
                                EVERYTHING_BELOW,
                                false,
                            ),
                            None => self.escape_leaf(at, base, v.as_ref(), &leaf),
                        }
                    }
                }
                _ => self.escape_leaf(at, base, v.as_ref(), &leaf),
            }
        }
        true
    }

    /// The table entry for argument `index` of the call `c` (its property
    /// `key`).
    fn consumer(
        &mut self,
        c: &'a CallExpression<'a>,
        index: usize,
        key: Option<&str>,
    ) -> Option<&'static DirConsumer> {
        if crate::names::rule_off("table") {
            return None;
        }
        let Some(V::Import(i)) = self.engine.site(&c.callee) else {
            return None;
        };
        DIR_CONSUMERS.iter().find(|t| {
            t.module == &*i.module
                && t.exports.iter().any(|e| *e == &*i.name)
                && t.arg == index
                && t.option == key
        })
    }

    /// The value goes to the directory consumer `t`.
    fn table(&mut self, t: &DirConsumer, span: oxc_span::Span, val: &JsVal, cwd: bool) {
        let leaves = anchored_leaves(val);
        if !leaves.iter().any(|l| l.0 == cwd) {
            return self.fail(cwd);
        }
        for (base, v, leaf) in leaves {
            let known = v.as_ref().map(prefix_of).filter(|p| p.1.is_empty());
            match (known, t.pattern) {
                (Some((dir, _)), Some(pattern)) => {
                    self.entry(base, span.start, dir, pattern, false)
                }
                (Some((file, _)), None) if !file.is_empty() && !file.contains('*') => {
                    let (dir, name) = file.rsplit_once('/').unwrap_or(("", &file));
                    self.entry(base, span.start, dir.to_owned(), &format!("{name}*"), false);
                }
                _ => self.escape_leaf(span, base, v.as_ref(), &leaf),
            }
        }
    }

    fn entry(&mut self, cwd: bool, at: u32, rel: String, pattern: &str, conditional: bool) {
        // The fact is line-based text.
        if rel.chars().chain(pattern.chars()).any(char::is_control) {
            return self.fail(cwd);
        }
        let e = Entry {
            cwd,
            at,
            rel,
            pattern: pattern.to_owned(),
            conditional,
        };
        if !self.entries.contains(&e) {
            self.entries.push(e);
        }
    }

    /// The value leaves what is evaluated here: everything at or below each
    /// anchored alternative is named.
    fn escape(&mut self, span: oxc_span::Span, val: &JsVal, cwd: bool) {
        let leaves = anchored_leaves(val);
        if !leaves.iter().any(|l| l.0 == cwd) {
            return self.fail(cwd);
        }
        for (base, v, leaf) in leaves {
            self.escape_leaf(span, base, v.as_ref(), &leaf);
        }
    }

    fn escape_leaf(&mut self, span: oxc_span::Span, cwd: bool, v: Option<&Value>, leaf: &JsVal) {
        let (dir, reasons) = match v {
            Some(v) => prefix_of(v),
            // The file itself: its directory.
            None => (String::new(), Vec::new()),
        };
        let origin = origin::combine(reasons.iter().map(|r| &**r)).0;
        // Every unknown part has to be the environment's: a tail that also has a
        // parameter or a listing in it is not.
        let mut env = !reasons.is_empty()
            && reasons
                .iter()
                .all(|r| matches!(origin::combine([&**r]).0, Origin::Env | Origin::Call))
            && matches!(origin, Origin::Env | Origin::Call)
            && !crate::names::rule_off("env");
        // A call is a candidate for an environment helper only when a variable name is
        // among its arguments (`getEnv('KEYS_PATH', fallback)`); the path with any
        // other call in its tail (`randomUUID()`) is an ordinary escape, with
        // no record.
        if env && origin == Origin::Call && !env_helper_calls(&self.engine.env.value_of(leaf)) {
            env = false;
        }
        if env {
            // One use per place and origin: the value of all its leaves together.
            let at = turborepo_trace_core::Span::new(span.start, span.end);
            let known = self
                .escapes
                .iter()
                .position(|(e, c, _)| e.span == at && e.origin == origin && *c == cwd);
            match known {
                Some(i) => {
                    if !self.escapes[i].2.contains(leaf) {
                        self.escapes[i].2.push(leaf.clone());
                        let all = JsVal::Alts(self.escapes[i].2.clone());
                        self.escapes[i].0.value = self.engine.env.value_of(&all);
                    }
                }
                None => {
                    let value = self.engine.env.value_of(leaf);
                    let detail = self.engine.text(span);
                    self.escapes.push((
                        EnvEscape {
                            span: at,
                            origin,
                            value,
                            detail,
                        },
                        cwd,
                        vec![leaf.clone()],
                    ));
                }
            }
        }
        self.entry(cwd, span.start, dir, EVERYTHING_BELOW, env);
    }
}

/// Whether `value` has calls and each of them is given the name of an
/// environment variable (by convention: upper case) as a literal argument.
fn env_helper_calls(value: &EnvValue) -> bool {
    use turborepo_trace_core::env::{EnvAlt, is_env_name};
    !value.calls.is_empty()
        && value.calls.iter().all(|c| {
            c.args.iter().any(|a| {
                a.value.iter().any(
                    |alt| matches!(alt, EnvAlt::Term(t) if t.literal().is_some_and(is_env_name)),
                )
            })
        })
}

/// What a call does with one of its arguments.
enum Passed<'a> {
    /// The value is tested, measured against or printed: nothing becomes of it.
    Dropped,
    /// A function of the file gets it as a parameter.
    Function(FnRef<'a>),
    /// Nothing is known.
    Other,
}

/// String (and array) methods that search for their argument and return a truth
/// value or a position: the ones the token scan treats as tests
/// ([`crate::names`]).
fn search_method(name: &str) -> bool {
    matches!(
        name,
        "startsWith" | "endsWith" | "includes" | "indexOf" | "lastIndexOf"
    )
}

/// The matchers of `expect(value)` that compare the value with their arguments
/// and touch no file (`toMatchFileSnapshot` reads one).
fn comparing_matcher(name: &str) -> bool {
    matches!(
        name,
        "toBe"
            | "toEqual"
            | "toStrictEqual"
            | "toContain"
            | "toContainEqual"
            | "toMatch"
            | "toMatchObject"
            | "toHaveProperty"
            | "toHaveLength"
            | "toHaveBeenCalledWith"
            | "toHaveBeenLastCalledWith"
            | "toHaveBeenNthCalledWith"
            | "toBeCalledWith"
            | "lastCalledWith"
            | "nthCalledWith"
            | "toHaveReturnedWith"
            | "toHaveLastReturnedWith"
            | "toHaveNthReturnedWith"
            | "toThrow"
            | "toThrowError"
    )
}

/// The asymmetric matchers of `expect`: values an assertion compares with.
fn asymmetric_matcher(name: &str) -> bool {
    matches!(
        name,
        "objectContaining" | "arrayContaining" | "stringContaining" | "stringMatching"
    )
}

/// `fs` functions that only write to the path at argument `index` (the
/// destination of a copy): nothing is read there by the call.
fn written(name: &str, index: usize) -> bool {
    let name = name.strip_suffix("Sync").unwrap_or(name);
    match index {
        0 => matches!(
            name,
            "writeFile"
                | "appendFile"
                | "rm"
                | "rmdir"
                | "unlink"
                | "truncate"
                | "mkdir"
                | "mkdirp"
                | "mkdirs"
                | "ensureDir"
                | "ensureFile"
                | "outputFile"
                | "outputJson"
                | "outputJSON"
                | "writeJson"
                | "writeJSON"
                | "remove"
        ),
        1 => matches!(name, "copyFile" | "cp" | "copy"),
        _ => false,
    }
}

/// Of the [`written`] functions, the ones whose result can be the path they
/// made.
fn returns_path(name: &str) -> bool {
    let name = name.strip_suffix("Sync").unwrap_or(name);
    matches!(name, "mkdir" | "mkdirp" | "mkdirs" | "ensureDir")
}

/// Whether `e` is the global `name` (no binding of the file shadows it).
fn is_global(engine: &Engine<'_, '_>, e: &Expression<'_>, name: &str) -> bool {
    match e.get_inner_expression() {
        Expression::Identifier(id) => {
            id.name == name
                && id
                    .reference_id
                    .get()
                    .and_then(|r| engine.scoping.get_reference(r).symbol_id())
                    .is_none()
        }
        _ => false,
    }
}

/// Whether `callee` is one of the global error constructors.
fn error_ctor(engine: &Engine<'_, '_>, callee: &Expression<'_>) -> bool {
    [
        "Error",
        "TypeError",
        "RangeError",
        "SyntaxError",
        "ReferenceError",
        "EvalError",
        "URIError",
        "AggregateError",
    ]
    .iter()
    .any(|n| is_global(engine, callee, n))
}

/// The words of the command-line part `v` that are relative paths, appended to
/// `out`. `false` when the part is not made of known strings, or has something
/// a shell expands (a variable, a substitution, a glob, `~`): which paths it
/// names is then not known.
fn command_words(v: &V<'_>, out: &mut Vec<String>) -> bool {
    match v {
        V::Void => true,
        V::Js(JsVal::Str(Value::Str(s))) => {
            if s.contains(['$', '`', '*', '?', '[', '{', '~']) {
                return false;
            }
            for w in turborepo_trace_core::names::words(s) {
                let relative = !w.starts_with('/') && !w.starts_with('-');
                let path = matches!(w, "." | "..")
                    || turborepo_trace_core::names::pattern_of(w, false).is_some();
                if relative && path && !out.iter().any(|o| o == w) {
                    out.push(w.to_owned());
                }
            }
            true
        }
        V::Array(a) => a.rest.is_none() && a.elems.iter().all(|e| command_words(e, out)),
        V::Alts(alts) => alts.iter().all(|a| command_words(a, out)),
        _ => false,
    }
}

/// The symbol a plain identifier pattern binds.
fn binding_symbol(p: &BindingPattern<'_>) -> Option<SymbolId> {
    match p {
        BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::{Origin, names::Names};

    use super::EnvEscape;
    use crate::{
        extract::{ScanOptions, scan_named},
        flow::follow_anchors,
        lexer::Dialect,
        names::scan_parts,
    };

    fn names(src: &str) -> (Names, Vec<EnvEscape>) {
        let dialect = Dialect::TS;
        let ex = scan_named(
            src.as_bytes(),
            dialect,
            ScanOptions::default(),
            Some("file.ts"),
        );
        let parts = scan_parts(src.as_bytes(), dialect);
        let templates: Vec<u32> = parts.templates.iter().map(|t| t.at).collect();
        let a = follow_anchors(
            src.as_bytes(),
            dialect,
            Some("file.ts"),
            &parts.uses,
            &ex.site_args,
            &templates,
        );
        let n = parts.finish(a.as_ref().map(|a| &a.followed));
        (n, a.map(|a| a.escapes).unwrap_or_default())
    }

    /// `<L|C>[?] <rel> <pattern>` per entry, and whether the coarse marks are
    /// left (location, working directory) with the parent steps that still
    /// count.
    fn entries(src: &str) -> (Vec<String>, (bool, bool, u32)) {
        let (n, _) = names(src);
        let list = n
            .anchored
            .iter()
            .map(|a| {
                format!(
                    "{}{} {} {}",
                    if a.cwd { 'C' } else { 'L' },
                    if a.conditional { "?" } else { "" },
                    a.rel,
                    a.pattern
                )
            })
            .collect();
        (list, (n.anchor.is_some(), n.cwd.is_some(), n.ups))
    }

    #[test]
    fn directories_handed_to_dotenv_flow_name_their_env_files() {
        let src = "import { resolve } from 'path';\nimport { config as loadEnvFiles } from \
                   'dotenv-flow';\nif (process.env.NODE_ENV !== 'production') {\nconst paths = \
                   [\nprocess.cwd(),\nresolve(process.cwd(), '..', '..'),\nresolve(__dirname, \
                   '..', '..'),\n];\nfor (const path of paths) {\nloadEnvFiles({ path: path, \
                   default_node_env: 'development' });\n}\n}\n";
        let (list, marks) = entries(src);
        assert_eq!(list, ["C  .env*", "C ../.. .env*", "L ../.. .env*"]);
        // Nothing of the coarse marks is left, and the `..` are part of the entries.
        assert_eq!(marks, (false, false, 0));
        // Other files in the directory: an option that names them is an escape.
        let (list, _) = entries(
            "import flow from 'dotenv-flow';\nflow.config({ path: __dirname, pattern: \
             'cfg[.node_env]' });\n",
        );
        assert_eq!(list, ["L  **"]);
        // `dotenv` takes the file.
        let (list, _) = entries(
            "require('dotenv').config({ path: require('path').resolve(process.cwd(), '.env.test') \
             });\n",
        );
        assert_eq!(list, ["C  .env.test*"]);
    }

    #[test]
    fn an_escaping_path_names_what_is_below_it() {
        for (src, want) in [
            ("helper(__dirname);", vec!["L  **"]),
            (
                "import { resolve } from 'path'; export const ROOT = resolve(__dirname, '..', \
                 '..');",
                vec!["L ../.. **"],
            ),
            (
                "import { join } from 'path'; const d = join(__dirname, '..'); load(d); \
                 other(join(d, 'x'));",
                vec!["L .. **", "L ../x **"],
            ),
            // An unknown tail stays below the known directory.
            (
                "import { join } from 'path'; export const f = (name) => load(join(__dirname, \
                 'fixtures', name));",
                vec!["L fixtures **"],
            ),
            (
                "spawn('node', ['x.js'], { cwd: process.cwd() });",
                vec!["C  **"],
            ),
            // The elements of a filtered array go on with the result.
            (
                "const dirs = [__dirname].filter((d) => ok(d)); use(dirs);",
                vec!["L  **"],
            ),
            // What a callback returns goes on with the result of `map`.
            (
                "import { join } from 'path'; const all = [__dirname].map((d) => join(d, '..', \
                 'x')); use(all);",
                vec!["L ../x **"],
            ),
            // A join of an object that is only assumed to be `path`.
            (
                "const p = tool.join(__dirname, 'a', name); use(p);",
                vec!["L  **"],
            ),
            // Returned from a function of the file.
            (
                "import path from 'path'; function root() { return path.resolve(__dirname, '..'); \
                 } export default root;",
                vec!["L .. **"],
            ),
        ] {
            let (list, marks) = entries(src);
            assert_eq!(list, want, "{src} {marks:?}");
            assert_eq!((marks.0, marks.1), (false, false), "{src}");
        }
    }

    #[test]
    fn a_site_consumes_its_argument() {
        // A known path: the read's reference says what is read, nothing is named, and
        // the parent step is part of the evaluated path.
        let (list, marks) = entries(
            "import { readFileSync } from 'fs'; import { resolve, join } from 'path';\nconst d = \
             resolve(__dirname, '..');\nconst part = 'conf' + 'ig';\nexport const load = () => \
             readFileSync(join(d, part));\n",
        );
        assert_eq!(list, Vec::<String>::new());
        assert_eq!(marks, (false, false, 0));
        // An unknown tail: the read is a pattern below the directory, which is named.
        let (list, marks) = entries(
            "import { readFileSync } from 'fs'; import { resolve, join } from 'path';\nconst d = \
             resolve(__dirname, '..');\nexport const load = (name) => readFileSync(join(d, \
             'data', name));\n",
        );
        assert_eq!(list, ["L ../data **"]);
        assert_eq!(marks, (false, false, 0));
        // A member of the location escapes with it; a value that is only tested or
        // measured names nothing.
        let (list, marks) = entries("if (__dirname.split('/').pop() === 'x') { run(); }");
        assert_eq!((list.len(), marks.0), (1, false));
        let (list, marks) = entries("if (__dirname.length > 3) { run(); }");
        assert_eq!((list, marks.0), (Vec::<String>::new(), false));
        let (list, marks) = entries("const here = __dirname; if (here === x) { run(); }");
        assert_eq!((list, marks.0), (Vec::<String>::new(), false));
    }

    #[test]
    fn a_base_that_is_only_measured_against_or_printed_names_nothing() {
        for (src, want) in [
            // The base of `path.relative`: the result is the other path, without a base.
            (
                "import path from 'path'; const root = path.resolve(__dirname, '../..');\nexport \
                 const rel = (f) => path.relative(root, f);",
                vec![],
            ),
            // As the target it is what the result leads to.
            (
                "import path from 'path'; const root = path.resolve(__dirname, '../..');\nexport \
                 const rel = (f) => path.relative(f, root);",
                vec!["L ../.. **"],
            ),
            // Searches and the length.
            (
                "const root = require('path').resolve(__dirname, '..');\nexport const inside = \
                 (p) => p.startsWith(root) && root.length < p.length;\nexport const has = (p) => \
                 root.includes(p) || p.indexOf(root) === 0;",
                vec![],
            ),
            // A part of the path is still a path.
            (
                "const root = require('path').resolve(__dirname, '..'); use(root.slice(1));",
                vec!["L .. **"],
            ),
            // Messages, also as text built from the path.
            (
                "const root = require('path').resolve(__dirname, '..');\nconsole.log(root); \
                 console.warn(`root is ${root}`);\nexport function f() { throw new Error('not \
                 below ' + root + '!'); }",
                vec![],
            ),
            // Text that goes somewhere else is not a message.
            (
                "const root = require('path').resolve(__dirname, '..'); run(`--root=${root} x`);",
                vec!["L .. **"],
            ),
            // A `console` of the file is some function.
            (
                "const console = make(); console.log(__dirname);",
                vec!["L  **"],
            ),
        ] {
            let (list, marks) = entries(src);
            assert_eq!(list, want, "{src} {marks:?}");
            assert_eq!(marks, (false, false, 0), "{src}");
        }
    }

    #[test]
    fn what_an_assertion_compares_and_what_is_only_written_names_nothing() {
        for (src, want) in [
            // The arguments and the subject of a comparing matcher.
            (
                "const root = require('path').resolve(__dirname, '..');\ntest('t', () => { \
                 expect(find()).toBe(root); expect(root).not.toEqual(other()); });",
                vec![],
            ),
            // An expected call: also inside the literal an asymmetric matcher is given.
            (
                "import { expect, test, vi } from 'vitest';\ntest('t', () => \
                 {\nexpect(spawn).toHaveBeenCalledWith('pnpm', ['run', \
                 'dev'],\nexpect.objectContaining({ cwd: new URL('../../', \
                 import.meta.url).pathname }));\n});",
                vec![],
            ),
            // A matcher that reads the file it is given, and a subject handed to one
            // that is not known.
            (
                "test('t', async () => { await expect(render()).toMatchFileSnapshot(__dirname); \
                 });",
                vec!["L  **"],
            ),
            (
                "test('t', () => { expect(__dirname).toBeADirectory(); });",
                vec!["L  **"],
            ),
            // An `expect` of the file is some function.
            (
                "import { expect } from './my-assert'; expect(x).toBe(__dirname);",
                vec!["L  **"],
            ),
            // Writes: nothing is read there.
            (
                "import fs from 'node:fs'; import { writeFile, rm } from 'node:fs/promises'; \
                 import path from 'node:path';\nconst out = path.resolve(__dirname, \
                 '..');\nexport async function emit(name, text) {\nfs.mkdirSync(path.join(out, \
                 'generated'), { recursive: true });\nawait writeFile(path.join(out, 'generated', \
                 name), text);\nfs.copyFileSync(tmp(name), path.join(out, name + '.bak'));\nawait \
                 rm(path.join(out, 'stale', name), { force: true });\n}",
                vec![],
            ),
            // The source of a copy is read, and so is what `readFile` is given.
            (
                "import fs from 'fs'; import path from 'path';\nexport const keep = (name) => \
                 fs.copyFileSync(path.join(__dirname, 'in', name), tmp(name));",
                vec!["L in **"],
            ),
            // `mkdir` can return the path it made.
            (
                "import fs from 'fs'; export const made = fs.mkdirSync(__dirname, { recursive: \
                 true });",
                vec!["L  **"],
            ),
        ] {
            let (list, marks) = entries(src);
            assert_eq!(list, want, "{src} {marks:?}");
            assert_eq!((marks.0, marks.1), (false, false), "{src}");
        }
    }

    #[test]
    fn the_working_directory_of_a_child_process_names_the_paths_of_its_command() {
        for (src, want) in [
            // A known command without a path: nothing is read through it.
            (
                "import { execSync } from 'child_process'; import path from \
                 'path';\nexecSync('git status --porcelain', { stdio: 'pipe', cwd: \
                 path.join(__dirname, '../..') });",
                vec![],
            ),
            // Its relative paths are below the directory; a bare word is no name.
            (
                "import { spawnSync } from 'node:child_process';\nspawnSync('node', \
                 ['scripts/build.js', '--out=../dist/a.json', 'fast'], { cwd: \
                 require('path').join(__dirname, '..') });",
                vec!["L ../scripts/build.js **", "L ../../dist/a.json **"],
            ),
            (
                "const cp = require('child_process'); cp.execFileSync('tsc', ['-p', '.'], { cwd: \
                 process.cwd() });",
                vec!["C  **"],
            ),
            // A part that is not known may be a path.
            (
                "import cp from 'child_process'; export const run = (cmd) => cp.execSync(cmd, { \
                 cwd: __dirname });",
                vec!["L  **"],
            ),
            // What the shell expands is not known either.
            (
                "import { execSync } from 'child_process'; execSync('cat *.json', { cwd: \
                 __dirname });",
                vec!["L  **"],
            ),
            // Another function called `spawn`.
            (
                "import { spawn } from './run'; spawn('git', ['status'], { cwd: __dirname });",
                vec!["L  **"],
            ),
        ] {
            let (list, marks) = entries(src);
            assert_eq!(list, want, "{src} {marks:?}");
            assert_eq!((marks.0, marks.1), (false, false), "{src}");
        }
    }

    #[test]
    fn an_argument_of_a_function_of_the_file_is_followed_into_it() {
        for (src, want) in [
            // A root that only bounds an upward search.
            (
                "import { join } from 'path'; import { existsSync } from 'fs';\nfunction \
                 nearest(rootDir, start) {\nlet cur = start;\nwhile (!existsSync(join(cur, \
                 'package.json'))) {\ncur = join(cur, '..');\nif (!cur.startsWith(rootDir)) \
                 return null;\n}\nreturn cur;\n}\nexport const find = (dir) => \
                 nearest(join(process.cwd(), '..', '..'), dir);",
                vec![],
            ),
            // A reader with the directory as a parameter: the reads are references.
            (
                "import { readFileSync } from 'fs'; import path from 'path';\nconst root = \
                 path.resolve(__dirname, '..');\nconst read = (base, file) => \
                 readFileSync(path.join(base, file), 'utf8');\nread(root, 'package.json'); \
                 read(path.join(root, '..'), 'agent/lib/lint.ts');",
                vec![],
            ),
            // What the function does with it counts.
            (
                "function use(dir) { other(dir); } use(__dirname);",
                vec!["L  **"],
            ),
            (
                "import { join } from 'path'; const pkg = (dir, name) => load(join(dir, \
                 'packages', name)); pkg(join(__dirname, '..'), 'a');",
                vec!["L ../packages/a **"],
            ),
            // What the function returns goes on with the result of the call.
            (
                "const same = (d) => d; other(same(__dirname));",
                vec!["L  **"],
            ),
            (
                "function up(d) { return require('path').join(d, '..'); } other(up(__dirname));",
                vec!["L .. **"],
            ),
            // The default of a parameter.
            (
                "import { join } from 'path'; const root = join(__dirname, '..');\nexport \
                 function depth(dir = root) { return dir.length; }",
                vec![],
            ),
            // `arguments` reads the parameter under another name: the call is an escape.
            (
                "function f(d) { return other(arguments[0]); } f(__dirname);",
                vec!["L  **"],
            ),
            // A spread before it: the position is not known.
            (
                "function f(a, d) { return d.length; } f(...xs, __dirname);",
                vec!["L  **"],
            ),
        ] {
            let (list, marks) = entries(src);
            assert_eq!(list, want, "{src} {marks:?}");
            assert_eq!((marks.0, marks.1), (false, false), "{src}");
        }
    }

    #[test]
    fn an_environment_tail_is_a_conditional_entry_and_a_use() {
        let src = "import { getEnv } from '@fx/envlib';\nimport { loadKey } from \
                   '@fx/keys';\nimport { isAbsolute, join } from 'path';\nconst keysPath = \
                   getEnv('KEYS_PATH', '/etc/keys');\nexport async function open(id: string) \
                   {\nconst abs = isAbsolute(keysPath) ? keysPath : join(__dirname, '..', '..', \
                   keysPath);\nreturn await loadKey(abs, id);\n}\n";
        let (n, escapes) = names(src);
        assert_eq!(n.anchor, None);
        assert_eq!(n.ups, 0);
        assert_eq!(n.anchored.len(), 1);
        let a = &n.anchored[0];
        assert_eq!(
            (a.cwd, &*a.rel, &*a.pattern, a.conditional, a.line),
            (false, "../..", "**", true, 7)
        );
        // The use is where the path leaves: the argument of the imported function.
        assert_eq!(escapes[0].detail, "abs");
        assert_eq!(escapes.len(), 1);
        assert_eq!(escapes[0].origin, Origin::Call);
        assert_eq!(escapes[0].value.calls.len(), 1);
        assert_eq!(escapes[0].value.calls[0].export, "getEnv");
        // A variable read directly.
        let (n, escapes) = names(
            "import { join } from 'path'; export const dir = join(__dirname, '..', \
             process.env.DATA_DIR);",
        );
        assert_eq!(n.anchored.len(), 1);
        assert!(n.anchored[0].conditional);
        assert_eq!(escapes[0].origin, Origin::Env);
        // Nor is a call that is given no variable name: an ordinary escape, no record.
        let (n, escapes) = names(
            "import { randomUUID } from 'node:crypto'; import path from 'node:path';\nexport \
             function tempFile(): string { return path.resolve(__dirname, '../.tmp', \
             `${randomUUID()}`); }",
        );
        assert_eq!(
            (&*n.anchored[0].rel, n.anchored[0].conditional),
            ("../.tmp", false)
        );
        assert!(escapes.is_empty());
        // A tail with a parameter in it is not the environment's alone.
        let (n, escapes) = names(
            "import { join } from 'path'; export const f = (x) => g(join(__dirname, x, \
             process.env.DATA_DIR));",
        );
        assert!(!n.anchored[0].conditional);
        assert!(escapes.is_empty());
        // A parameter is not the environment's.
        let (n, escapes) =
            names("import { join } from 'path'; export const f = (x) => g(join(__dirname, x));");
        assert!(!n.anchored[0].conditional);
        assert!(escapes.is_empty());
    }

    #[test]
    fn a_template_is_judged_on_the_value_of_its_leading_constant() {
        let shapes = |src: &str| -> Vec<String> {
            names(src)
                .0
                .names
                .iter()
                .map(|n| n.shape.to_string())
                .collect()
        };
        // A URL is not a path.
        assert_eq!(
            shapes(
                "const API = 'https://api.example.com'; export const u = (id) => \
                 `${API}/apps/${id}/installations`;"
            ),
            Vec::<String>::new()
        );
        // Also through a constant that is itself a template (the host is a name of its
        // own).
        assert_eq!(
            shapes(
                "export const HOST = 'example.com'; export const BASE = `https://${HOST}`; export \
                 const u = (slug) => `${BASE}/apps/${slug}`; export const s = \
                 `${BASE}/settings/apps`;"
            ),
            ["example.com"]
        );
        // A directory makes the name longer.
        assert_eq!(
            shapes("const BASE = 'fixtures/data'; export const f = `${BASE}/a.json`;"),
            ["fixtures/data", "fixtures/data/a.json"]
        );
        // A constant that is shadowed where the template is: the parameter is not
        // known.
        assert_eq!(
            shapes(
                "const API = 'https://api.example.com'; export const u = (API) => \
                 `${API}/apps/list`;"
            ),
            ["apps/list"]
        );
        // A binding with another assignment is not a constant.
        assert_eq!(
            shapes(
                "let API = 'https://api.example.com'; API = pick(); export const u = \
                 `${API}/apps/list`;"
            ),
            ["apps/list"]
        );
    }

    #[test]
    fn what_cannot_be_followed_keeps_the_coarse_mark() {
        // The engine has no value for `PWD`: the working directory mark stays, with the
        // parent steps of the file.
        let (list, marks) =
            entries("const up = require('path').resolve(process.env.PWD, '..'); helper(up);");
        assert_eq!(list, Vec::<String>::new());
        assert_eq!(marks, (false, true, 1));
        // `eval`: the file is not analyzed.
        let (list, marks) = entries("eval(code); helper(__dirname);");
        assert_eq!(list, Vec::<String>::new());
        assert_eq!(marks, (true, false, 0));
        // One base followed, the other not.
        let (list, marks) = entries("helper(__dirname); other(process.env.INIT_CWD);");
        assert_eq!(list, ["L  **"]);
        assert_eq!(marks, (false, true, 0));
    }
}
