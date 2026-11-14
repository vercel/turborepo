//! Environment facts of a file ([`turborepo_trace_core::env`]): which
//! references read a path from the environment, which values the file assigns
//! to environment variables, and which exported functions return environment
//! values.
//!
//! # Uses
//!
//! When the value of a site has alternatives the recognizer cannot lower (it
//! records an uncertainty with origin `env` or `call` for them),
//! [`EnvState::uses`] expresses those alternatives as templates: literal text,
//! the value of a variable (`process.env.X`, exactly: a value derived from it,
//! such as `process.env.X.trim()`, is unknown), and the result of a call of an
//! imported function with its evaluated arguments (`getEnv('X', DEFAULT)`). The
//! engine records every such call in a table ([`Engine::import_call`]) and
//! marks the unknown it returns with the table index (`@call[<module>#<name>]
//! @ref[<index>] <text>`).
//!
//! # Assignments
//!
//! [`collect`] walks the syntax tree for the forms that give a variable a
//! value. The name is always literal at the assignment (see the soundness notes
//! of `turborepo_trace_store::compose::env`):
//!
//! | Form | Value |
//! |---|---|
//! | `process.env.X = v`, `process.env['X'] = v`, `<any object>.X = v` for an upper-case `X` | `v` |
//! | `{ X: v }` in any object literal (the `env` of a spawned process, an object spread into `process.env`, a test server's environment), `{ ...base, X }` | `v` |
//! | `f('X', v, …)`: a call with the name as a string argument (`vi.stubEnv('X', v)`, a setter helper, `getEnv('X', fallback)`) | every other argument: a string, what a function argument returns, the property values of an object argument |
//! | `'X=v'`, `` `X=${v}` `` inside a string (a shell command line) | `v` |
//!
//! Calls on `expect`, `assert`, `console`, `describe`, `it` and `test` are not
//! assignments. A value that can never be a path (`undefined`, a boolean, the
//! empty string) or that lies in the temporary directory is left out; an
//! assignment with no value left is not recorded.
//!
//! # Helpers
//!
//! In a file that reads a variable by a computed name (`process.env[name]`), an
//! exported function whose text reads `process.env` (directly or through
//! another function of the file) is evaluated once with its parameters bound to
//! argument placeholders. `process.env[<placeholder>]` is "the variable named
//! by that argument", a returned placeholder is "that argument", a called
//! placeholder is "what that argument returns". If every alternative of the
//! result is such a template, the function is recorded as a helper; otherwise
//! calls to it stay unknown.

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::AstNodes;
use oxc_span::{GetSpan, Span};
use turborepo_trace_core::{
    Origin,
    env::{
        EnvAlt, EnvArg, EnvBase, EnvCall, EnvFact, EnvName, EnvPart, EnvTerm, EnvValue,
        command_assignments, is_env_name,
    },
    eval::{PatPart, Value},
};

use super::{
    engine::{Args, Engine},
    value::{FnRef, V},
};
use crate::extract::{
    evaluator::{self, Anchor, JsVal},
    origin::{self, Tag},
};

/// Maximum imported calls recorded per file.
const MAX_CALLS: usize = 256;
/// Maximum arguments of a recorded call, and of a helper.
const MAX_ARGS: usize = 6;
/// Maximum alternatives of one fact value; more become one unknown.
const MAX_ALTS: usize = 64;
/// Maximum assignment facts per file. A file with more gets one unknown
/// assignment per further name, so nothing is silently dropped.
const MAX_SETS: usize = 4096;
/// Arguments a helper with a rest parameter is summarized for.
const REST_ARGS: usize = 4;

/// The function name in the payload of argument placeholders.
const ARG_FN: &str = "$env";

/// The placeholder for argument `i` of a helper being summarized.
fn arg_marker(i: usize) -> Box<str> {
    origin::mark(Tag::Param, &format!("{ARG_FN}#{i}"), &format!("$arg{i}"))
}

/// "What calling argument `i` returns."
fn arg_call_marker(i: usize) -> Box<str> {
    origin::mark(Tag::Param, &format!("{ARG_FN}#{i}"), &format!("$arg{i}()"))
}

/// "The variable named by argument `i`."
fn arg_env_marker(i: usize) -> Box<str> {
    origin::mark(
        Tag::Env,
        &format!("$arg{i}"),
        &format!("process.env[$arg{i}]"),
    )
}

/// The index of the argument placeholder `reason` is exactly.
fn marker_index(reason: &str) -> Option<usize> {
    (0..MAX_ARGS).find(|&i| *arg_marker(i) == *reason)
}

/// Calling the value with reason `reason`: for an argument placeholder, "what
/// that argument returns".
pub(super) fn call_of_arg<'a>(reason: &str) -> Option<V<'a>> {
    marker_index(reason).map(|i| V::Js(JsVal::Unknown(arg_call_marker(i))))
}

/// `process.env[k]` when every alternative of `k` is an argument placeholder.
pub(super) fn env_of_args<'a>(k: &V<'a>) -> Option<V<'a>> {
    let mut out = Vec::new();
    for l in k.leaves() {
        match l {
            V::Js(JsVal::Unknown(r)) => {
                out.push(V::Js(JsVal::Unknown(arg_env_marker(marker_index(r)?))))
            }
            V::Void => {}
            _ => return None,
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(super::value::join(out))
}

/// The variable `reason` stands for exactly (`process.env.X` itself, not a
/// value derived from it).
pub(super) fn exact_env(reason: &str) -> Option<&str> {
    let (marks, text) = origin::parse(reason)?;
    let [(Tag::Env, name)] = marks.as_slice() else {
        return None;
    };
    if name.is_empty() || *name == "*" || name.starts_with('$') {
        return None;
    }
    (text.strip_prefix("process.env.") == Some(name)).then_some(*name)
}

/// One recorded call of an imported function.
struct CallRec {
    /// The reason of the unknown the call evaluated to.
    reason: Box<str>,
    call: EnvCall,
}

/// The imported calls a file's evaluation has seen.
#[derive(Default)]
pub(crate) struct EnvState {
    calls: Vec<CallRec>,
}

impl EnvState {
    /// The template part `reason` stands for exactly.
    fn part_of(&self, reason: &str) -> Option<EnvPart> {
        if let Some(i) = marker_index(reason) {
            return Some(EnvPart::Arg(i as u32));
        }
        for i in 0..MAX_ARGS {
            if *arg_call_marker(i) == *reason {
                return Some(EnvPart::ArgCall(i as u32));
            }
            if *arg_env_marker(i) == *reason {
                return Some(EnvPart::Var(EnvName::Arg(i as u32)));
            }
        }
        if let Some(name) = exact_env(reason) {
            return Some(EnvPart::Var(EnvName::Name(name.to_owned())));
        }
        if origin::has_tag(reason, Tag::Ref) {
            return self
                .calls
                .iter()
                .position(|c| *c.reason == *reason)
                .map(|k| EnvPart::Call(k as u32));
        }
        None
    }

    fn unknown(reason: &str) -> EnvAlt {
        let mut text = origin::strip(reason);
        if text.len() > 160 {
            let mut n = 160;
            while !text.is_char_boundary(n) {
                n -= 1;
            }
            text.truncate(n);
        }
        EnvAlt::Unknown(text)
    }

    /// The alternative for a wholly unknown value with `reason`.
    fn reason_alt(&self, reason: &str) -> EnvAlt {
        match self.part_of(reason) {
            Some(p) => EnvAlt::Term(EnvTerm {
                base: EnvBase::Str,
                parts: vec![p],
            }),
            None => Self::unknown(reason),
        }
    }

    /// The alternatives of string value `v` anchored at `base`.
    fn value_alts(&self, base: EnvBase, v: &Value, out: &mut Vec<EnvAlt>) {
        match v {
            Value::Str(s) => {
                if base == EnvBase::Str && s.is_empty() {
                    // The empty string is never a path.
                    return;
                }
                out.push(EnvAlt::Term(EnvTerm {
                    base,
                    parts: if s.is_empty() {
                        Vec::new()
                    } else {
                        vec![EnvPart::Lit(s.clone())]
                    },
                }));
            }
            Value::Unknown { reason } => match self.part_of(reason) {
                Some(p) => out.push(EnvAlt::Term(EnvTerm {
                    base,
                    parts: vec![p],
                })),
                None if base == EnvBase::FileDir => out.push(EnvAlt::Term(EnvTerm {
                    base,
                    parts: vec![EnvPart::Wild],
                })),
                None => out.push(Self::unknown(reason)),
            },
            Value::Pattern(parts) => {
                let mut ps = Vec::with_capacity(parts.len());
                for (i, p) in parts.iter().enumerate() {
                    match p {
                        PatPart::Lit(s) => ps.push(EnvPart::Lit(s.clone())),
                        PatPart::Wild { reason } => match self.part_of(reason) {
                            Some(p) => ps.push(p),
                            // An unknown head of an unanchored string can be anything.
                            None if i == 0 && base != EnvBase::FileDir => {
                                out.push(Self::unknown(reason));
                                return;
                            }
                            None => ps.push(EnvPart::Wild),
                        },
                    }
                }
                out.push(EnvAlt::Term(EnvTerm { base, parts: ps }));
            }
            Value::Alternatives(a) => {
                for x in a {
                    self.value_alts(base, x, out);
                }
            }
        }
    }

    /// The alternatives of a string-domain value. Values that are never a path
    /// or lie in the temporary directory add nothing.
    fn js_alts(&self, j: &JsVal, out: &mut Vec<EnvAlt>) {
        match j {
            JsVal::Str(v) => self.value_alts(EnvBase::Str, v, out),
            JsVal::Path(Anchor::Tmp, _) | JsVal::Url(Anchor::Tmp, _) => {}
            JsVal::Path(an, v) | JsVal::Url(an, v) => {
                let base = match an {
                    Anchor::Cwd => EnvBase::Cwd,
                    _ => EnvBase::FileDir,
                };
                self.value_alts(base, &evaluator::normalize_value(v.clone(), true), out);
            }
            JsVal::Unknown(r) => out.push(self.reason_alt(r)),
            JsVal::File => out.push(EnvAlt::Unknown("__filename".into())),
            JsVal::FileUrl => out.push(EnvAlt::Unknown("import.meta.url".into())),
            JsVal::Resolved(m, _) => {
                out.push(EnvAlt::Unknown(m.to_string()));
            }
            JsVal::Alts(vs) => vs.iter().for_each(|x| self.js_alts(x, out)),
            JsVal::Obj(_) | JsVal::Func(_) | JsVal::Arr(_) => {}
        }
    }

    /// The alternatives of `v` as a string: containers, functions and
    /// `undefined` are not strings.
    fn alts(&self, v: &V<'_>, out: &mut Vec<EnvAlt>) {
        for l in v.leaves() {
            match l {
                V::Js(j) => self.js_alts(j, out),
                V::Import(i) => {
                    out.push(EnvAlt::Unknown(format!("imported {}#{}", i.module, i.name)))
                }
                V::Void
                | V::Object(_)
                | V::Array(_)
                | V::Func(_)
                | V::Builtin(_)
                | V::Method(..)
                | V::Loader(_)
                | V::Alts(_) => {}
            }
        }
    }

    /// A fact value from alternatives: deduplicated, capped, with the calls its
    /// parts refer to.
    fn finish(&self, alts: Vec<EnvAlt>) -> EnvValue {
        let mut out: Vec<EnvAlt> = Vec::new();
        for a in alts {
            if !out.contains(&a) {
                out.push(a);
            }
        }
        if out.len() > MAX_ALTS {
            out = vec![EnvAlt::Unknown("too many alternatives".into())];
        }
        let mut map: Vec<(u32, u32)> = Vec::new();
        let mut calls: Vec<EnvCall> = Vec::new();
        self.remap(&mut out, &mut map, &mut calls);
        EnvValue { alts: out, calls }
    }

    /// Rewrites the call indices of `alts` from the file's table to `calls`,
    /// copying the calls (and the calls their arguments refer to) on first
    /// use.
    fn remap(&self, alts: &mut [EnvAlt], map: &mut Vec<(u32, u32)>, calls: &mut Vec<EnvCall>) {
        for a in alts {
            let EnvAlt::Term(t) = a else { continue };
            for p in &mut t.parts {
                let EnvPart::Call(k) = p else { continue };
                if let Some(m) = map.iter().find(|m| m.0 == *k) {
                    *k = m.1;
                    continue;
                }
                let Some(rec) = self.calls.get(*k as usize) else {
                    continue;
                };
                let new = calls.len() as u32;
                map.push((*k, new));
                let mut c = rec.call.clone();
                calls.push(EnvCall {
                    module: String::new(),
                    export: String::new(),
                    args: Vec::new(),
                });
                for arg in &mut c.args {
                    self.remap(&mut arg.value, map, calls);
                    if let Some(r) = &mut arg.returns {
                        self.remap(r, map, calls);
                    }
                }
                calls[new as usize] = c;
                *k = new;
            }
        }
    }

    /// The whole value `val` as a template ([`super::anchors`]: an anchored
    /// path with an environment tail).
    pub(super) fn value_of(&self, val: &JsVal) -> EnvValue {
        let mut alts = Vec::new();
        self.js_alts(val, &mut alts);
        self.finish(alts)
    }

    /// The alternatives of site value `val` that the recognizer drops with an
    /// uncertainty of origin `env` or `call` (mirrors `evaluator::lower`),
    /// as one value per origin.
    pub(super) fn uses(&self, val: &JsVal) -> Vec<(Origin, EnvValue)> {
        let mut buckets: Vec<(Origin, Vec<EnvAlt>)> = Vec::new();
        for leaf in evaluator::split_alts(val.clone()) {
            let mut reasons: Vec<Box<str>> = Vec::new();
            let mut alts = Vec::new();
            match &leaf {
                JsVal::Unknown(r) => {
                    reasons.push(r.clone());
                    alts.push(self.reason_alt(r));
                }
                JsVal::Str(v) if leading_wild(v) => {
                    evaluator::wild_reasons(v, &mut reasons);
                    self.value_alts(EnvBase::Str, v, &mut alts);
                }
                JsVal::Path(Anchor::Cwd, v) | JsVal::Url(Anchor::Cwd, v) => {
                    let v = evaluator::normalize_value(v.clone(), true);
                    if !leading_wild(&v) {
                        continue;
                    }
                    evaluator::wild_reasons(&v, &mut reasons);
                    self.value_alts(EnvBase::Cwd, &v, &mut alts);
                }
                _ => continue,
            }
            let (o, _) = origin::combine(reasons.iter().map(|r| &**r));
            if !matches!(o, Origin::Env | Origin::Call) {
                continue;
            }
            match buckets.iter_mut().find(|b| b.0 == o) {
                Some(b) => b.1.extend(alts),
                None => buckets.push((o, alts)),
            }
        }
        buckets
            .into_iter()
            .map(|(o, alts)| (o, self.finish(alts)))
            .collect()
    }
}

fn leading_wild(v: &Value) -> bool {
    match v {
        Value::Unknown { .. } => true,
        Value::Pattern(p) => matches!(p.first(), Some(PatPart::Wild { .. })),
        _ => false,
    }
}

impl<'a> Engine<'a, '_> {
    /// The result of calling the imported binding `module#export` with `args`:
    /// an unknown of origin `call` that refers to the recorded call.
    pub(super) fn import_call(
        &mut self,
        module: &str,
        export: &str,
        args: &Args<'a>,
        span: Span,
    ) -> V<'a> {
        let payload = format!("{module}#{export}");
        if args.has_rest() || args.positional().len() > MAX_ARGS {
            return self.unk(Tag::Call, &payload, span);
        }
        let vals: Vec<V<'a>> = args.positional().to_vec();
        let mut call = EnvCall {
            module: module.to_owned(),
            export: export.to_owned(),
            args: Vec::with_capacity(vals.len()),
        };
        for v in &vals {
            let arg = self.env_arg(v, span);
            call.args.push(arg);
        }
        let k = match self.env.calls.iter().position(|c| c.call == call) {
            Some(k) => k,
            None => {
                if self.env.calls.len() >= MAX_CALLS {
                    return self.unk(Tag::Call, &payload, span);
                }
                let text = self.text(span);
                let reason = origin::mark(
                    Tag::Call,
                    &payload,
                    &origin::mark(Tag::Ref, &self.env.calls.len().to_string(), &text),
                );
                self.env.calls.push(CallRec { reason, call });
                self.env.calls.len() - 1
            }
        };
        V::Js(JsVal::Unknown(self.env.calls[k].reason.clone()))
    }

    /// One call argument: its alternatives as a string, and what it returns
    /// when it can be a function.
    fn env_arg(&mut self, v: &V<'a>, span: Span) -> EnvArg {
        let mut arg = EnvArg::default();
        self.env.alts(v, &mut arg.value);
        let fns: Vec<FnRef<'a>> = v
            .leaves()
            .iter()
            .filter_map(|l| match l {
                V::Func(f) => Some(*f),
                _ => None,
            })
            .collect();
        if !fns.is_empty() {
            let mut returns = Vec::new();
            for f in fns {
                let r = self.inline(f, &Args::of(Vec::new()), span);
                self.env.alts(&r, &mut returns);
            }
            arg.returns = Some(returns);
        }
        arg
    }

    /// The alternatives of the value of expression `e` as an assigned value.
    /// With `deep` (a call argument next to the variable's name): also what
    /// a function returns and the property values and elements of a
    /// container.
    fn assigned(&mut self, e: &'a Expression<'a>, deep: bool, out: &mut Vec<EnvAlt>) {
        let Some(v) = self.site(e) else {
            out.push(EnvAlt::Unknown(self.text(e.span())));
            return;
        };
        self.env.alts(&v, out);
        if !deep {
            return;
        }
        for l in v.into_leaves() {
            match l {
                V::Func(f) => {
                    let r = self.site_inline(f, &Args::of(Vec::new()));
                    self.env.alts(&r, out);
                }
                V::Object(o) => self.env.alts(&o.values(), out),
                V::Array(a) => self.env.alts(&a.elements(), out),
                _ => {}
            }
        }
    }

    /// The summary of function `f`: its result with the parameters bound to
    /// argument placeholders. `None` when it is not a template over its
    /// arguments and the environment.
    fn summarize(&mut self, f: FnRef<'a>) -> Option<EnvValue> {
        let params = match f {
            FnRef::Function(func) => &func.params,
            FnRef::Arrow(a) => &a.params,
        };
        let n = if params.rest.is_some() {
            REST_ARGS.max(params.items.len())
        } else {
            params.items.len()
        };
        if n > MAX_ARGS {
            return None;
        }
        let args = Args::of(
            (0..n)
                .map(|i| V::Js(JsVal::Unknown(arg_marker(i))))
                .collect(),
        );
        let v = self.site_inline(f, &args);
        let mut alts = Vec::new();
        self.env.alts(&v, &mut alts);
        let value = self.env.finish(alts);
        let template = !value.alts.is_empty()
            && value.alts.iter().all(|a| matches!(a, EnvAlt::Term(_)))
            && value.alts.iter().any(|a| match a {
                EnvAlt::Term(t) => t
                    .parts
                    .iter()
                    .any(|p| matches!(p, EnvPart::Var(_) | EnvPart::Call(_))),
                EnvAlt::Unknown(_) => false,
            });
        template.then_some(value)
    }
}

/// Whether `obj` is `process.env` syntactically.
fn is_process_env(obj: &Expression<'_>) -> bool {
    match obj.get_inner_expression() {
        Expression::StaticMemberExpression(m) => {
            m.property.name == "env"
                && matches!(m.object.get_inner_expression(), Expression::Identifier(id) if id.name == "process")
        }
        _ => false,
    }
}

/// Whether a call with this callee is an assertion, a log line or a test
/// declaration: the variable's name next to a value does not assign it.
fn not_an_assignment(callee: &Expression<'_>) -> bool {
    let mut e = callee;
    loop {
        match e.get_inner_expression() {
            Expression::StaticMemberExpression(m) => e = &m.object,
            Expression::ComputedMemberExpression(m) => e = &m.object,
            Expression::CallExpression(c) => e = &c.callee,
            Expression::Identifier(id) => {
                return matches!(
                    id.name.as_str(),
                    "expect" | "assert" | "console" | "describe" | "it" | "test"
                );
            }
            _ => return false,
        }
    }
}

/// The value of a literal `NAME=value` in command-line text.
fn shell_value(v: &str) -> Vec<EnvAlt> {
    if v.is_empty() {
        return Vec::new();
    }
    for prefix in ["$PWD/", "${PWD}/", "$(pwd)/"] {
        if let Some(rest) = v.strip_prefix(prefix)
            && !rest.contains(['$', '`'])
        {
            return vec![EnvAlt::Term(EnvTerm {
                base: EnvBase::Cwd,
                parts: vec![EnvPart::Lit(rest.to_owned())],
            })];
        }
    }
    if v.contains(['$', '`']) {
        return vec![EnvAlt::Unknown(format!("shell expansion {v}"))];
    }
    vec![EnvAlt::Term(EnvTerm::str(v))]
}

/// A function of the file's top level.
struct TopFn<'a> {
    name: &'a str,
    f: FnRef<'a>,
    exports: Vec<String>,
    envish: bool,
}

/// The top-level functions of the program, with the names they are exported
/// under.
fn top_functions<'a>(program: &'a Program<'a>, text: &str) -> Vec<TopFn<'a>> {
    fn declared<'a>(d: &'a Declaration<'a>, exported: bool, out: &mut Vec<TopFn<'a>>) {
        match d {
            Declaration::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    out.push(TopFn {
                        name: id.name.as_str(),
                        f: FnRef::Function(f),
                        exports: if exported {
                            vec![id.name.as_str().to_owned()]
                        } else {
                            Vec::new()
                        },
                        envish: false,
                    });
                }
            }
            Declaration::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        continue;
                    };
                    let f = match d.init.as_ref().map(|e| e.get_inner_expression()) {
                        Some(Expression::ArrowFunctionExpression(a)) => FnRef::Arrow(a),
                        Some(Expression::FunctionExpression(f)) => FnRef::Function(f),
                        _ => continue,
                    };
                    out.push(TopFn {
                        name: id.name.as_str(),
                        f,
                        exports: if exported {
                            vec![id.name.as_str().to_owned()]
                        } else {
                            Vec::new()
                        },
                        envish: false,
                    });
                }
            }
            _ => {}
        }
    }
    let mut out: Vec<TopFn<'a>> = Vec::new();
    let mut named: Vec<(&str, String)> = Vec::new();
    for s in &program.body {
        match s {
            Statement::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    out.push(TopFn {
                        name: id.name.as_str(),
                        f: FnRef::Function(f),
                        exports: Vec::new(),
                        envish: false,
                    });
                }
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        continue;
                    };
                    let f = match d.init.as_ref().map(|e| e.get_inner_expression()) {
                        Some(Expression::ArrowFunctionExpression(a)) => FnRef::Arrow(a),
                        Some(Expression::FunctionExpression(f)) => FnRef::Function(f),
                        _ => continue,
                    };
                    out.push(TopFn {
                        name: id.name.as_str(),
                        f,
                        exports: Vec::new(),
                        envish: false,
                    });
                }
            }
            Statement::ExportDeclaration(d) => declared(&d.declaration, true, &mut out),
            Statement::ExportDefaultDeclaration(d) => {
                if let ExportDefaultDeclarationKind::FunctionDeclaration(f) = &d.declaration {
                    out.push(TopFn {
                        name: f.id.as_ref().map_or("default", |id| id.name.as_str()),
                        f: FnRef::Function(f),
                        exports: vec!["default".to_owned()],
                        envish: false,
                    });
                }
            }
            Statement::ExportNamedDeclaration(n) => {
                for sp in &n.specifiers {
                    if let (ModuleExportName::IdentifierReference(l), Some(e)) =
                        (&sp.local, export_name(&sp.exported))
                    {
                        named.push((l.name.as_str(), e));
                    }
                }
            }
            _ => {}
        }
    }
    for (local, exported) in named {
        if let Some(f) = out.iter_mut().find(|f| f.name == local)
            && !f.exports.contains(&exported)
        {
            f.exports.push(exported);
        }
    }
    // Functions that read the environment, directly or through another function of
    // the file (by name, textually: a cheap filter, the evaluation decides).
    let body = |f: &TopFn<'a>| -> &str {
        let sp = f.f.span();
        text.get(sp.start as usize..sp.end as usize).unwrap_or("")
    };
    for f in &mut out {
        f.envish = body(f).contains("process.env");
    }
    loop {
        let names: Vec<&str> = out.iter().filter(|f| f.envish).map(|f| f.name).collect();
        let calls: Vec<bool> = out
            .iter()
            .map(|f| {
                let b = body(f);
                !f.envish
                    && names.iter().any(|n| {
                        b.match_indices(n).any(|(at, _)| {
                            b.as_bytes().get(at + n.len()) == Some(&b'(')
                                && (at == 0 || !is_ident_byte(b.as_bytes()[at - 1]))
                        })
                    })
            })
            .collect();
        let mut changed = false;
        for (f, hit) in out.iter_mut().zip(calls) {
            if hit {
                f.envish = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn export_name(n: &ModuleExportName<'_>) -> Option<String> {
    match n {
        ModuleExportName::IdentifierName(i) => Some(i.name.as_str().to_owned()),
        ModuleExportName::IdentifierReference(i) => Some(i.name.as_str().to_owned()),
        ModuleExportName::StringLiteral(s) => s.value.as_str().map(str::to_owned),
    }
}

/// A string literal that is exactly a variable name.
fn name_literal<'a>(e: &'a Expression<'a>) -> Option<&'a str> {
    match e {
        Expression::StringLiteral(s) => s.value.as_str().filter(|n| is_env_name(n)),
        _ => None,
    }
}

/// Collects assignment facts.
struct Sets {
    facts: Vec<EnvFact>,
    /// `(name, value)` already recorded.
    seen: std::collections::HashSet<(String, EnvValue)>,
    count: usize,
}

impl Sets {
    /// Records `name = alts` found at offset `at`; `expr` is the assigned
    /// expression when there is exactly one.
    fn push(
        &mut self,
        state: &EnvState,
        name: &str,
        at: u32,
        alts: Vec<EnvAlt>,
        expr: Option<Span>,
    ) {
        if alts.is_empty() {
            return;
        }
        let value = if self.count >= MAX_SETS {
            EnvValue {
                alts: vec![EnvAlt::Unknown("too many assignments in one file".into())],
                calls: Vec::new(),
            }
        } else {
            state.finish(alts)
        };
        if !self.seen.insert((name.to_owned(), value.clone())) {
            return;
        }
        self.count += 1;
        let unknown = value.alts.iter().any(|a| matches!(a, EnvAlt::Unknown(_)));
        self.facts.push(EnvFact::Set {
            name: name.to_owned(),
            // The byte offset; `collect` turns it into a line.
            line: at,
            value,
            at: expr.filter(|_| unknown).map(|s| [s.start, s.end]),
        });
    }
}

/// The assignment and helper facts of a file. `line` fields hold byte offsets.
pub(super) fn collect<'a>(engine: &mut Engine<'a, '_>, nodes: &AstNodes<'a>) -> Vec<EnvFact> {
    let text = engine.src;
    let mut sets = Sets {
        facts: Vec::new(),
        seen: Default::default(),
        count: 0,
    };
    let mut program: Option<&'a Program<'a>> = None;
    for node in nodes.iter() {
        match node.kind() {
            AstKind::Program(p) => program = Some(p),
            AstKind::AssignmentExpression(a) => {
                let (obj, name) = match &a.left {
                    AssignmentTarget::StaticMemberExpression(m) => {
                        (&m.object, Some(m.property.name.as_str()))
                    }
                    AssignmentTarget::ComputedMemberExpression(m) => (
                        &m.object,
                        match &m.expression {
                            Expression::StringLiteral(s) => s.value.as_str(),
                            _ => None,
                        },
                    ),
                    _ => continue,
                };
                let Some(name) = name else { continue };
                if !(is_env_name(name) || is_process_env(obj)) {
                    continue;
                }
                let mut alts = Vec::new();
                match a.operator {
                    AssignmentOperator::Assign
                    | AssignmentOperator::LogicalOr
                    | AssignmentOperator::LogicalNullish
                    | AssignmentOperator::LogicalAnd => engine.assigned(&a.right, false, &mut alts),
                    _ => alts.push(EnvAlt::Unknown(engine.text(a.span))),
                }
                sets.push(&engine.env, name, a.span.start, alts, Some(a.right.span()));
            }
            AstKind::ObjectExpression(o) => {
                let mut spread = false;
                for prop in &o.properties {
                    let p = match prop {
                        ObjectPropertyKind::ObjectProperty(p) => p,
                        ObjectPropertyKind::SpreadProperty(_) => {
                            spread = true;
                            continue;
                        }
                    };
                    if p.computed || p.method || p.kind != PropertyKind::Init {
                        continue;
                    }
                    // `{ X }` counts in an object built on another one (`{ ...process.env,
                    // X }`): elsewhere it is a list of names more often than an environment.
                    if p.shorthand && !spread {
                        continue;
                    }
                    let name = match &p.key {
                        PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                        PropertyKey::StringLiteral(s) => s.value.as_str(),
                        _ => None,
                    };
                    let Some(name) = name.filter(|n| is_env_name(n)) else {
                        continue;
                    };
                    let mut alts = Vec::new();
                    engine.assigned(&p.value, false, &mut alts);
                    sets.push(&engine.env, name, p.span.start, alts, Some(p.value.span()));
                }
            }
            AstKind::CallExpression(c) => {
                if c.arguments.len() < 2
                    || !c
                        .arguments
                        .iter()
                        .any(|a| a.as_expression().and_then(name_literal).is_some())
                    || not_an_assignment(&c.callee)
                {
                    continue;
                }
                for (i, a) in c.arguments.iter().enumerate() {
                    let Some(name) = a.as_expression().and_then(name_literal) else {
                        continue;
                    };
                    let mut alts = Vec::new();
                    for (j, other) in c.arguments.iter().enumerate() {
                        if j == i {
                            continue;
                        }
                        match other.as_expression() {
                            Some(e) => engine.assigned(e, true, &mut alts),
                            None => alts.push(EnvAlt::Unknown(engine.text(other.span()))),
                        }
                    }
                    sets.push(&engine.env, name, c.span.start, alts, None);
                }
            }
            AstKind::StringLiteral(s) => {
                let Some(v) = s.value.as_str() else { continue };
                if !v.contains('=') {
                    continue;
                }
                for (name, value, _) in command_assignments(v) {
                    sets.push(&engine.env, name, s.span.start, shell_value(value), None);
                }
            }
            AstKind::TemplateLiteral(t) => {
                for (i, q) in t.quasis.iter().enumerate() {
                    let raw = q.value.raw.as_str();
                    if !raw.contains('=') {
                        continue;
                    }
                    for (name, value, at_end) in command_assignments(raw) {
                        let mut alts = Vec::new();
                        match (at_end, t.expressions.get(i)) {
                            (true, Some(e)) => {
                                // `X=${v}`: the expression, when the value ends there.
                                let next = t.quasis.get(i + 1).map_or("", |q| q.value.raw.as_str());
                                let ends = (next.is_empty() && i + 2 >= t.quasis.len())
                                    || next.starts_with([
                                        ' ', '\t', '\n', ';', '&', '|', ')', '"', '\'',
                                    ]);
                                if ends {
                                    engine.assigned(e, false, &mut alts);
                                } else {
                                    alts.push(EnvAlt::Unknown(engine.text(t.span)));
                                }
                            }
                            (true, None) => {}
                            (false, _) => alts = shell_value(value),
                        }
                        sets.push(&engine.env, name, t.span.start, alts, None);
                    }
                }
            }
            _ => {}
        }
    }
    let mut facts = sets.facts;
    if let Some(p) = program
        && text.contains("process.env[")
    {
        for f in top_functions(p, text) {
            if f.exports.is_empty() || !f.envish {
                continue;
            }
            let Some(value) = engine.summarize(f.f) else {
                continue;
            };
            for export in f.exports {
                facts.push(EnvFact::Helper {
                    export,
                    value: value.clone(),
                });
            }
        }
    }
    facts
}

/// The index of the last significant byte before `k` (inclusive): whitespace,
/// `//` lines, trailing `//` comments and `/* */` comments are skipped.
fn prev_significant(src: &[u8], mut k: usize) -> Option<usize> {
    loop {
        while matches!(src.get(k), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            k = k.checked_sub(1)?;
        }
        let line_start = memchr::memrchr(b'\n', &src[..=k]).map_or(0, |i| i + 1);
        let line = &src[line_start..=k];
        let indent = line
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t'))
            .unwrap_or(0);
        let text = &line[indent..];
        if text.starts_with(b"//") || text.starts_with(b"*") || text.starts_with(b"/*") {
            // A comment line.
            k = line_start.checked_sub(1)?;
            continue;
        }
        if let Some(c) = memchr::memmem::rfind(text, b" //") {
            // Code followed by a comment.
            k = (line_start + indent + c).checked_sub(1)?;
            continue;
        }
        if text.ends_with(b"*/")
            && let Some(open) = memchr::memmem::rfind(&src[..k], b"/*")
        {
            k = open.checked_sub(1)?;
            continue;
        }
        return Some(k);
    }
}

/// Whether `src` can have assignment or helper facts: a cheap scan that never
/// misses a form [`collect`] records. It looks for an upper-case name
///
/// - as an object key (`X: v` after `{` or `,`), or as a shorthand key after a
///   spread;
/// - written through a member (`.X = v`, `['X'] = v`);
/// - as a string argument of a call with further arguments (`f('X', v)`, `f(v,
///   'X')`);
/// - followed by `=` in command-line text (`X=v`);
///
/// and for `process.env[...]`, `process.env = ...` and `process.env.x = ...`.
pub(crate) fn may_have_facts(src: &[u8]) -> bool {
    let n = src.len();
    let skip_ws = |mut j: usize| {
        while j < n && matches!(src[j], b' ' | b'\t' | b'\r' | b'\n') {
            j += 1;
        }
        j
    };
    let assigns = |j: usize| {
        src.get(j) == Some(&b'=') && !matches!(src.get(j + 1), Some(b'=' | b'>'))
            || (matches!(src.get(j), Some(b'|' | b'?' | b'&'))
                && src.get(j + 1) == src.get(j)
                && src.get(j + 2) == Some(&b'='))
            || (matches!(src.get(j), Some(b'+' | b'-' | b'*' | b'/' | b'%'))
                && src.get(j + 1) == Some(&b'='))
    };
    for at in memchr::memmem::find_iter(src, b"process.env") {
        let mut j = at + b"process.env".len();
        match src.get(j) {
            Some(b'[') => return true,
            Some(b'.') | Some(b'?') => {
                j += 1;
                if src.get(j) == Some(&b'.') {
                    j += 1;
                }
                while j < n && is_ident_byte(src[j]) {
                    j += 1;
                }
                if assigns(skip_ws(j)) {
                    return true;
                }
            }
            _ => {
                if assigns(skip_ws(j)) {
                    return true;
                }
            }
        }
    }
    let mut i = 0;
    while i < n {
        let c = src[i];
        if !(c.is_ascii_uppercase() || c == b'_') {
            // Skip the rest of a token that does not start in upper case.
            if is_ident_byte(c) {
                while i < n && is_ident_byte(src[i]) {
                    i += 1;
                }
            } else {
                i += 1;
            }
            continue;
        }
        let s = i;
        let mut upper = false;
        let mut ok = true;
        while i < n && is_ident_byte(src[i]) {
            let b = src[i];
            upper |= b.is_ascii_uppercase();
            ok &= b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_';
            i += 1;
        }
        if !ok || !upper || i - s < 3 {
            continue;
        }
        let prev = if s > 0 { src[s - 1] } else { b' ' };
        let quoted = matches!(prev, b'\'' | b'"' | b'`') && src.get(i) == Some(&prev);
        // The next significant byte after the name (and its closing quote).
        let j = skip_ws(if quoted { i + 1 } else { i });
        let next = src.get(j).copied().unwrap_or(b' ');
        if prev == b'.' && assigns(j) {
            return true;
        }
        if !quoted && j == i && assigns(j) && matches!(prev, b' ' | b'\'' | b'"' | b'`' | b';') {
            // `X=value` in command-line text (no space before `=`).
            return true;
        }
        // The significant byte before the name (and its opening quote).
        let before = (if quoted {
            s.checked_sub(2)
        } else {
            s.checked_sub(1)
        })
        .and_then(|k| prev_significant(src, k))
        .map(|k| src[k]);
        match (before, next) {
            // An object key.
            (Some(b'{' | b','), b':') if src.get(j + 1) != Some(&b':') => return true,
            // A shorthand key in an object with a spread.
            (Some(b'{' | b','), b'}' | b',') if !quoted => {
                let from = s.saturating_sub(2048);
                if memchr::memmem::find(&src[from..s], b"...").is_some() {
                    return true;
                }
            }
            // A string argument of a call with further arguments.
            (Some(b'('), b',') | (Some(b','), b',' | b')') if quoted => return true,
            // `x['X'] = v`.
            (Some(b'['), b']') if quoted && assigns(skip_ws(j + 1)) => return true,
            _ => {}
        }
    }
    false
}

/// Turns the byte offsets in the `line` fields of `facts` into 1-based lines.
pub(super) fn set_lines(facts: &mut [EnvFact], text: &str) {
    if facts.is_empty() {
        return;
    }
    let idx = turborepo_trace_core::lexkit::LineIndex::new(text.as_bytes());
    for f in facts {
        match f {
            EnvFact::Set { line, .. } | EnvFact::Use { line, .. } => {
                *line = idx.line_col(*line).line;
            }
            EnvFact::Helper { .. } => {}
        }
    }
}
