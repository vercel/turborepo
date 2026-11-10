//! The abstract evaluator: expressions, bindings, calls.

use std::{collections::HashMap, rc::Rc};

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{AstNodes, NodeId, Scoping, SymbolId};
use oxc_span::GetSpan;
use turborepo_trace_core::{env::EnvFrom, eval::Value};

use super::{
    stable::{Facts, any_written, written},
    sym::SymState,
    value::{ArrV, Builtin, FnRef, ImportRef, MAX_ITEMS, ObjV, Trust, V, join},
};
use crate::extract::{
    evaluator::{self, Anchor, Func, JsVal, Obj, Scope},
    origin::{self, Tag},
};

/// Evaluation steps per site while the file budget lasts.
const SITE_STEPS: u32 = 20_000;
/// Evaluation steps per site once the file budget is used up.
const MIN_STEPS: u32 = 300;
/// Evaluation steps per file (beyond it every site gets [`MIN_STEPS`]).
const FILE_STEPS: u32 = 400_000;
/// Maximum evaluation nesting.
const MAX_DEPTH: u32 = 160;
/// Maximum nested inlined calls.
const MAX_INLINE: usize = 8;
/// Maximum call sites joined for one parameter.
const MAX_CALL_SITES: usize = 64;
/// Maximum argument combinations of one builtin call or concatenation.
const MAX_COMBOS: usize = 64;
/// Passes over a binding whose definition refers to itself.
const MAX_PASSES: usize = 3;

/// A binding being evaluated.
struct Busy<'a> {
    sym: SymbolId,
    /// The value handed to definitions that refer to the binding itself.
    partial: V<'a>,
}

/// How a function can be called.
pub(super) enum Reach<'a> {
    /// Through a name: its direct calls in the file; `escapes` when the name is
    /// also used as a value or exported.
    Named {
        calls: Vec<&'a CallExpression<'a>>,
        escapes: bool,
        name: Box<str>,
    },
    /// Passed as argument number `arg` of `call`.
    Callback {
        call: &'a CallExpression<'a>,
        arg: usize,
    },
    /// By callers the analysis cannot see (a method, a returned closure, ...).
    Unseen { name: Box<str> },
}

/// What is known about a function's body.
pub(super) struct FnInfo<'a> {
    pub returns: Vec<&'a Expression<'a>>,
    /// A generator, or a body whose returns could not all be collected: calls
    /// are unknown.
    pub generator: bool,
}

/// Evaluated call arguments.
pub(super) struct Args<'a> {
    vals: Vec<V<'a>>,
    /// From this index on, every argument is one of these (a spread of unknown
    /// length).
    rest: Option<V<'a>>,
}

impl<'a> Args<'a> {
    pub(super) fn get(&self, i: usize) -> V<'a> {
        match (self.vals.get(i), &self.rest) {
            (Some(v), _) => v.clone(),
            (None, Some(r)) => join([r.clone(), V::Void]),
            (None, None) => V::Void,
        }
    }

    /// Arguments with known values and no spread.
    pub(super) fn of(vals: Vec<V<'a>>) -> Self {
        Args { vals, rest: None }
    }

    /// The positional arguments (without a spread of unknown length).
    pub(super) fn positional(&self) -> &[V<'a>] {
        &self.vals
    }

    /// Whether a spread of unknown length follows the positional arguments.
    pub(super) fn has_rest(&self) -> bool {
        self.rest.is_some()
    }

    /// The positional arguments and the value of every further one.
    pub(super) fn into_parts(self) -> (Vec<V<'a>>, Option<V<'a>>) {
        (self.vals, self.rest)
    }

    /// The arguments from index `i` on, as an array.
    pub(super) fn tail(&self, i: usize) -> V<'a> {
        V::array(
            self.vals.iter().skip(i).cloned().collect(),
            self.rest.clone(),
        )
    }
}

pub(crate) struct Engine<'a, 's> {
    pub(super) nodes: &'s AstNodes<'a>,
    pub(super) scoping: &'s Scoping,
    pub(super) src: &'a str,
    file_name: Option<&'s str>,
    no_consts: HashMap<&'a str, (JsVal, bool)>,
    site_steps: u32,
    file_steps: u32,
    depth: u32,
    memo: HashMap<SymbolId, V<'a>>,
    busy: Vec<Busy<'a>>,
    /// Lowest index into `busy` whose partial value was read since the marker
    /// was reset.
    lowest_hit: usize,
    /// Parameter bindings of the calls being inlined (innermost last).
    pub(super) frames: Vec<Vec<(SymbolId, V<'a>)>>,
    /// Whether the innermost frame was read since the flag was reset.
    frame_used: bool,
    inlining: Vec<u32>,
    guess: Option<Box<str>>,
    /// Where the load being evaluated searches from, for the part of its
    /// request that is an environment variable ([`Engine::site_load`]).
    asked_from: Option<EnvFrom>,
    fns: HashMap<u32, Rc<FnInfo<'a>>>,
    reach: HashMap<u32, Rc<Reach<'a>>>,
    pub(super) facts: Option<Rc<Facts<'a>>>,
    pub(super) checking: Vec<SymbolId>,
    /// Calls of imported functions evaluated so far ([`super::env`]).
    pub(super) env: super::env::EnvState,
    /// Symbolic mode ([`super::sym`]): the expression nodes of the file.
    pub(super) sym: Option<SymState<'a>>,
    /// What the tests around a use say about a binding ([`super::guard`]).
    pub(super) guards: super::guard::Guards<'a>,
}

impl<'a, 's> Engine<'a, 's> {
    pub(super) fn new(
        nodes: &'s AstNodes<'a>,
        scoping: &'s Scoping,
        src: &'a str,
        file_name: Option<&'s str>,
    ) -> Self {
        Engine {
            nodes,
            scoping,
            src,
            file_name,
            no_consts: HashMap::new(),
            site_steps: 0,
            file_steps: FILE_STEPS,
            depth: 0,
            memo: HashMap::new(),
            busy: Vec::new(),
            lowest_hit: usize::MAX,
            frames: Vec::new(),
            frame_used: false,
            inlining: Vec::new(),
            guess: None,
            asked_from: None,
            fns: HashMap::new(),
            reach: HashMap::new(),
            facts: None,
            checking: Vec::new(),
            env: super::env::EnvState::default(),
            sym: None,
            guards: super::guard::Guards::default(),
        }
    }

    // ---- sites ----

    pub(super) fn begin(&mut self) {
        self.site_steps = if self.file_steps > 0 {
            SITE_STEPS.min(self.file_steps.max(MIN_STEPS))
        } else {
            MIN_STEPS
        };
        self.depth = 0;
        self.guess = None;
        self.frames.clear();
        self.busy.clear();
        self.inlining.clear();
        self.checking.clear();
        self.lowest_hit = usize::MAX;
        self.frame_used = false;
    }

    pub(super) fn end(&mut self, budget: u32) {
        let used = budget.saturating_sub(self.site_steps);
        self.file_steps = self.file_steps.saturating_sub(used);
    }

    /// The value of the expression `e` of a site.
    pub(super) fn site(&mut self, e: &'a Expression<'a>) -> Option<V<'a>> {
        self.begin();
        let budget = self.site_steps;
        let v = self.eval(e);
        self.end(budget);
        Some(v)
    }

    /// The value of a spread argument (`f(...xs)`): any element.
    pub(super) fn site_spread(&mut self, e: &'a Expression<'a>) -> Option<V<'a>> {
        self.begin();
        let budget = self.site_steps;
        let v = self.eval(e);
        let v = self.elements(v, e.span());
        self.end(budget);
        Some(v)
    }

    /// The value of a `new X(...)` site.
    pub(super) fn site_new(&mut self, n: &'a NewExpression<'a>) -> Option<V<'a>> {
        self.begin();
        let budget = self.site_steps;
        let v = self.new_expr(n);
        self.end(budget);
        Some(v)
    }

    /// The result of calling the file's function `f` with `args`, as a site of
    /// its own.
    pub(super) fn site_inline(&mut self, f: FnRef<'a>, args: &Args<'a>) -> V<'a> {
        self.begin();
        let budget = self.site_steps;
        let v = self.inline(f, args, f.span());
        self.end(budget);
        v
    }

    /// The value of the expression node `k`, as a site of its own
    /// ([`super::anchors`]: the expressions a path is built with). `None`
    /// for other kinds of nodes.
    pub(super) fn site_node(&mut self, k: AstKind<'a>) -> Option<V<'a>> {
        self.begin();
        let budget = self.site_steps;
        let v = self.eval_node(k);
        self.end(budget);
        self.guess = None;
        v
    }

    fn eval_node(&mut self, k: AstKind<'a>) -> Option<V<'a>> {
        Some(match k {
            AstKind::IdentifierReference(id) => self.ident(id),
            AstKind::ImportMeta(_) => V::Js(JsVal::Obj(Obj::ImportMeta)),
            AstKind::CallExpression(c) => self.call(c),
            AstKind::NewExpression(n) => self.new_expr(n),
            AstKind::StaticMemberExpression(m) => self.static_member(m),
            AstKind::ComputedMemberExpression(m) => self.computed_member(m),
            AstKind::TemplateLiteral(t) => self.template(t),
            AstKind::BinaryExpression(b) if b.operator == BinaryOperator::Addition => {
                let l = self.eval(&b.left);
                let r = self.eval(&b.right);
                self.add(l, r, b.span)
            }
            AstKind::ConditionalExpression(c) => {
                let a = self.eval(&c.consequent);
                let b = self.eval(&c.alternate);
                join([a, b])
            }
            AstKind::LogicalExpression(l) => {
                let a = self.eval(&l.left);
                let b = self.eval(&l.right);
                join([a, b])
            }
            AstKind::AwaitExpression(a) => self.eval(&a.argument),
            AstKind::ChainExpression(c) => match &c.expression {
                ChainElement::CallExpression(call) => self.call(call),
                ChainElement::TSNonNullExpression(x) => self.eval(&x.expression),
                ChainElement::StaticMemberExpression(m) => self.static_member(m),
                ChainElement::ComputedMemberExpression(m) => self.computed_member(m),
                ChainElement::PrivateFieldExpression(m) => self.private_member(m),
            },
            _ => return None,
        })
    }

    pub(super) fn take_guess(&mut self) -> Option<Box<str>> {
        self.guess.take()
    }

    /// Where the load evaluated last searches from ([`Engine::site_load`]); the
    /// file for any other site.
    pub(super) fn take_from(&mut self) -> EnvFrom {
        self.asked_from.take().unwrap_or_default()
    }

    /// The step budget of the site being evaluated.
    pub(super) fn budget(&self) -> u32 {
        self.site_steps
    }

    /// Evaluation steps used so far (of the file budget).
    pub(super) fn steps_used(&self) -> u32 {
        FILE_STEPS - self.file_steps
    }

    // ---- helpers ----

    pub(super) fn tick(&mut self) -> bool {
        if self.site_steps == 0 {
            return false;
        }
        self.site_steps -= 1;
        true
    }

    fn scope(&self) -> Scope<'_, 'a> {
        Scope {
            src: self.src.as_bytes(),
            consts: &self.no_consts,
            apis: &[],
            loaders: &[],
            file_name: self.file_name,
        }
    }

    /// Cleaned source text of `span`.
    pub(super) fn text(&self, span: Span) -> String {
        origin::clean(
            self.src
                .get(span.start as usize..span.end as usize)
                .unwrap_or(""),
        )
    }

    pub(super) fn unk(&self, tag: Tag, payload: &str, span: Span) -> V<'a> {
        V::unknown(tag, payload, &self.text(span))
    }

    /// An unknown derived from the leaf `j` (a property of it, a method
    /// result): it keeps the origin of `j`'s unknown parts.
    pub(super) fn derived(&mut self, j: &JsVal, span: Span) -> V<'a> {
        if let Some(v) = self.sym_derived(j, span) {
            return v;
        }
        let mut rs: Vec<Box<str>> = Vec::new();
        match j {
            JsVal::Unknown(r) => rs.push(r.clone()),
            JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v) | JsVal::Resolved(_, v) => {
                evaluator::wild_reasons(v, &mut rs)
            }
            _ => {}
        }
        let text = self.text(span);
        if rs.is_empty() {
            return V::unknown(Tag::Unknown, "", &text);
        }
        V::Js(JsVal::Unknown(origin::derive(&rs.join(" "), &text)))
    }

    /// The result of calling the unknown function value `j`.
    fn derived_call(&mut self, j: &JsVal, span: Span) -> V<'a> {
        if let JsVal::Unknown(r) = j
            && let Some(v) = super::env::call_of_arg(r)
        {
            // A helper summary: calling an argument.
            return v;
        }
        match j {
            JsVal::Unknown(r)
                if origin::has_tag(r, Tag::Param)
                    || origin::has_tag(r, Tag::Env)
                    || origin::has_tag(r, Tag::Argv)
                    || origin::has_tag(r, Tag::Data)
                    || origin::has_tag(r, Tag::Import) =>
            {
                self.derived(j, span)
            }
            _ => self.unk(Tag::Call, "", span),
        }
    }

    /// The parent of `id`, skipping parentheses and TypeScript value wrappers.
    pub(super) fn parent(&self, mut id: NodeId) -> (NodeId, AstKind<'a>) {
        loop {
            let p = self.nodes.parent_id(id);
            let k = self.nodes.kind(p);
            match k {
                AstKind::ParenthesizedExpression(_)
                | AstKind::TSAsExpression(_)
                | AstKind::TSSatisfiesExpression(_)
                | AstKind::TSNonNullExpression(_)
                | AstKind::TSTypeAssertion(_)
                    if p != id =>
                {
                    id = p;
                }
                _ => return (p, k),
            }
        }
    }

    // ---- expressions ----

    pub(super) fn eval(&mut self, e: &'a Expression<'a>) -> V<'a> {
        if !self.tick() || self.depth >= MAX_DEPTH {
            return self.unk(Tag::Unknown, "", e.span());
        }
        self.depth += 1;
        let v = self.eval_inner(e);
        self.depth -= 1;
        v
    }

    fn eval_inner(&mut self, e: &'a Expression<'a>) -> V<'a> {
        match e {
            Expression::StringLiteral(s) => match s.value.as_str() {
                Some(x) => V::str(x),
                None => self.unk(Tag::Unknown, "", s.span),
            },
            Expression::TemplateLiteral(t) => self.template(t),
            Expression::NumericLiteral(n) => {
                let txt = self.src.get(n.span.start as usize..n.span.end as usize);
                match txt {
                    // Decimal digits are their own text (`010` is octal in sloppy code).
                    Some(t)
                        if t.len() <= 15
                            && t.bytes().all(|b| b.is_ascii_digit())
                            && (t.len() == 1 || !t.starts_with('0')) =>
                    {
                        V::str(t)
                    }
                    _ => self.unk(Tag::Unknown, "", n.span),
                }
            }
            Expression::BooleanLiteral(_) => V::scalar(),
            Expression::NullLiteral(_) => V::Void,
            Expression::BigIntLiteral(b) => self.unk(Tag::Unknown, "", b.span),
            Expression::RegExpLiteral(r) => self.unk(Tag::Unknown, "", r.span),
            Expression::Identifier(id) => self.ident(id),
            Expression::ParenthesizedExpression(p) => self.eval(&p.expression),
            Expression::TSAsExpression(x) => self.eval(&x.expression),
            Expression::TSSatisfiesExpression(x) => self.eval(&x.expression),
            Expression::TSNonNullExpression(x) => self.eval(&x.expression),
            Expression::TSTypeAssertion(x) => self.eval(&x.expression),
            Expression::TSInstantiationExpression(x) => self.eval(&x.expression),
            // A promise and its settled value are not distinguished.
            Expression::AwaitExpression(a) => self.eval(&a.argument),
            Expression::SequenceExpression(s) => match s.expressions.last() {
                Some(last) => self.eval(last),
                None => V::Void,
            },
            Expression::ConditionalExpression(c) => {
                let a = self.eval(&c.consequent);
                let b = self.eval(&c.alternate);
                join([a, b])
            }
            Expression::LogicalExpression(l) => {
                let left = self.eval(&l.left);
                match l.operator {
                    // `a && b`: `b`, or `a` when it is falsy. The falsy value matters:
                    // `(cond && x) || y` must still see `y`, and `` `f${s && '-' + s}` `` is
                    // `f` for an empty `s`.
                    LogicalOperator::And => {
                        let right = self.eval(&l.right);
                        match falsy_part(&left) {
                            Some(f) => join([right, f]),
                            None => right,
                        }
                    }
                    LogicalOperator::Or | LogicalOperator::Coalesce => {
                        if always_truthy(&left) {
                            left
                        } else {
                            let right = self.eval(&l.right);
                            join([left, right])
                        }
                    }
                }
            }
            Expression::BinaryExpression(b) => match b.operator {
                BinaryOperator::Addition => {
                    let l = self.eval(&b.left);
                    let r = self.eval(&b.right);
                    self.add(l, r, b.span)
                }
                BinaryOperator::Equality
                | BinaryOperator::Inequality
                | BinaryOperator::StrictEquality
                | BinaryOperator::StrictInequality
                | BinaryOperator::LessThan
                | BinaryOperator::LessEqualThan
                | BinaryOperator::GreaterThan
                | BinaryOperator::GreaterEqualThan
                | BinaryOperator::In
                | BinaryOperator::Instanceof => V::scalar(),
                _ => self.unk(Tag::Unknown, "", b.span),
            },
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::Void => V::Void,
                UnaryOperator::LogicalNot | UnaryOperator::Delete => V::scalar(),
                _ => self.unk(Tag::Unknown, "", u.span),
            },
            Expression::ObjectExpression(o) => self.object(o),
            Expression::ArrayExpression(a) => self.array(a),
            Expression::ArrowFunctionExpression(f) => V::Func(FnRef::Arrow(f)),
            Expression::FunctionExpression(f) => V::Func(FnRef::Function(f)),
            Expression::CallExpression(c) => self.call(c),
            Expression::NewExpression(n) => self.new_expr(n),
            Expression::ChainExpression(c) => {
                // `a?.b`: the value, or `undefined` (never a path).
                match &c.expression {
                    ChainElement::CallExpression(call) => self.call(call),
                    ChainElement::TSNonNullExpression(x) => self.eval(&x.expression),
                    ChainElement::StaticMemberExpression(m) => self.static_member(m),
                    ChainElement::ComputedMemberExpression(m) => self.computed_member(m),
                    ChainElement::PrivateFieldExpression(m) => self.private_member(m),
                }
            }
            Expression::StaticMemberExpression(m) => self.static_member(m),
            Expression::ComputedMemberExpression(m) => self.computed_member(m),
            Expression::PrivateFieldExpression(m) => self.private_member(m),
            Expression::AssignmentExpression(a) => match a.operator {
                AssignmentOperator::Assign => self.eval(&a.right),
                _ => self.unk(Tag::Unknown, "", a.span),
            },
            Expression::ImportMeta(_) => V::Js(JsVal::Obj(Obj::ImportMeta)),
            Expression::TaggedTemplateExpression(t) => {
                let tag = self.eval(&t.tag);
                if tag == V::Js(JsVal::Func(Func::StringRaw)) && t.quasi.expressions.is_empty() {
                    match t.quasi.quasis.first() {
                        Some(q) => V::str(q.value.raw.as_str()),
                        None => V::str(""),
                    }
                } else {
                    self.unk(Tag::Unknown, "", t.span)
                }
            }
            Expression::ImportExpression(i) => {
                let m = self.eval(&i.source);
                match &m {
                    V::Js(JsVal::Str(Value::Str(s))) => self.module_value(s),
                    _ => match self.sym_require(&m, i.span) {
                        Some(x) => x,
                        None => self.unk(Tag::Unknown, "", i.span),
                    },
                }
            }
            Expression::ThisExpression(t) => self.unk(Tag::Unknown, "", t.span),
            Expression::JSXElement(_) | Expression::JSXFragment(_) => V::Void,
            Expression::ClassExpression(c) => self.unk(Tag::Unknown, "", c.span),
            other => self.unk(Tag::Unknown, "", other.span()),
        }
    }

    fn ident(&mut self, id: &'a IdentifierReference<'a>) -> V<'a> {
        let sym = id
            .reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id());
        match sym {
            Some(s) => {
                let v = self.symbol(s);
                self.guarded(id, s, v)
            }
            None => self.global(id.name.as_str()),
        }
    }

    fn global(&mut self, name: &str) -> V<'a> {
        match name {
            "__dirname" => V::Js(JsVal::dirname()),
            "__filename" => V::Js(self.scope().this_file()),
            "process" => V::Js(JsVal::Obj(Obj::Process)),
            "require" => V::Js(JsVal::Obj(Obj::Require)),
            "String" => V::Js(JsVal::Obj(Obj::StringCtor)),
            "URL" => V::Js(JsVal::Obj(Obj::UrlCtor)),
            "undefined" => V::Void,
            "NaN" | "Infinity" => V::scalar(),
            "Object" => V::Builtin(Builtin::Object),
            "Array" => V::Builtin(Builtin::Array),
            "JSON" => V::Builtin(Builtin::Json),
            "Promise" => V::Builtin(Builtin::Promise),
            "Set" => V::Builtin(Builtin::SetCtor),
            "Map" => V::Builtin(Builtin::MapCtor),
            _ => V::unknown(Tag::Unknown, "", &origin::clean(name)),
        }
    }

    fn template(&mut self, t: &'a TemplateLiteral<'a>) -> V<'a> {
        let mut acc: Option<V<'a>> = None;
        for (i, q) in t.quasis.iter().enumerate() {
            let Some(s) = q.value.cooked.and_then(|c| c.as_str()) else {
                return self.unk(Tag::Unknown, "", t.span);
            };
            if !s.is_empty() {
                let lit = V::str(s);
                acc = Some(match acc {
                    None => lit,
                    Some(a) => self.concat(a, lit, t.span),
                });
            }
            if let Some(e) = t.expressions.get(i) {
                let v = self.eval(e);
                acc = Some(match acc {
                    None => self.concat(v, V::str(""), t.span),
                    Some(a) => self.concat(a, v, t.span),
                });
            }
        }
        acc.unwrap_or_else(|| V::str(""))
    }

    /// A leaf as a `+` operand. `None`: a value that is never part of a path.
    fn operand(&self, l: &V<'a>, left: bool, span: Span) -> Option<JsVal> {
        let reason = || origin::mark(Tag::Unknown, "", &self.text(span));
        Some(match l {
            // A number or boolean becomes its text.
            l if l.is_scalar() => JsVal::Unknown(reason()),
            V::Js(j) if left => {
                let j = match (j, self.file_name) {
                    // `${import.meta.url}/../x`
                    (JsVal::FileUrl, Some(n)) => JsVal::Url(Anchor::FileDir, Value::str(n)),
                    (j, _) => j.clone(),
                };
                evaluator::stringy(j, reason)
            }
            V::Js(j) => evaluator::stringy_right(j.clone(), reason),
            V::Void => return None,
            V::Builtin(Builtin::Tmp) if left => JsVal::Path(
                Anchor::Tmp,
                Value::unknown(origin::mark(Tag::Unknown, "", "temporary path")),
            ),
            V::Import(i) => JsVal::Unknown(origin::mark(
                Tag::Import,
                &format!("{}#{}", i.module, i.name),
                &origin::clean(&i.name),
            )),
            _ => JsVal::Unknown(reason()),
        })
    }

    /// `a + b` (also template literals).
    pub(super) fn concat(&mut self, a: V<'a>, b: V<'a>, span: Span) -> V<'a> {
        let (la, lb) = (a.leaves(), b.leaves());
        if la.len() * lb.len() > MAX_COMBOS {
            return self.unk(Tag::Unknown, "", span);
        }
        let reason = origin::mark(Tag::Unknown, "", &self.text(span));
        let mut out = Vec::with_capacity(la.len() * lb.len());
        for x in la {
            for y in lb {
                let (Some(x), Some(y)) =
                    (self.operand(x, true, span), self.operand(y, false, span))
                else {
                    continue;
                };
                out.push(V::Js(match self.sym_concat(&x, &y, span) {
                    Some(j) => j,
                    None => evaluator::concat(x, y, &reason),
                }));
            }
        }
        join(out)
    }

    /// The `+` operator: a concatenation, unless both operands can be numbers
    /// (a numeric literal is kept as its digits, so `n + 1` would otherwise
    /// read as `"11"`): then the result is unknown.
    fn add(&mut self, a: V<'a>, b: V<'a>, span: Span) -> V<'a> {
        let numeric = |v: &V<'a>| v.leaves().iter().any(V::maybe_number);
        if !(numeric(&a) && numeric(&b)) {
            return self.concat(a, b, span);
        }
        let (la, lb) = (a.into_leaves(), b.into_leaves());
        if la.len() * lb.len() > MAX_COMBOS {
            return self.unk(Tag::Unknown, "", span);
        }
        let mut out = Vec::with_capacity(la.len() * lb.len());
        for x in &la {
            for y in &lb {
                out.push(if x.maybe_number() && y.maybe_number() {
                    match (x, y) {
                        (V::Js(j @ JsVal::Unknown(_)), _) | (_, V::Js(j @ JsVal::Unknown(_)))
                            if !V::Js(j.clone()).is_scalar() =>
                        {
                            self.derived(j, span)
                        }
                        _ => self.unk(Tag::Unknown, "", span),
                    }
                } else {
                    self.concat(x.clone(), y.clone(), span)
                });
            }
        }
        join(out)
    }

    // ---- members ----

    fn static_member(&mut self, m: &'a StaticMemberExpression<'a>) -> V<'a> {
        if matches!(
            m.object.get_inner_expression(),
            Expression::ThisExpression(_)
        ) {
            return self.this_field(m.node_id(), m.property.name.as_str(), m.span);
        }
        let o = self.eval(&m.object);
        self.member(o, m.property.name.as_str(), m.span)
    }

    fn private_member(&mut self, m: &'a PrivateFieldExpression<'a>) -> V<'a> {
        if matches!(
            m.object.get_inner_expression(),
            Expression::ThisExpression(_)
        ) {
            let name = format!("#{}", m.field.name);
            return self.this_field(m.node_id(), &name, m.span);
        }
        self.unk(Tag::Unknown, "", m.span)
    }

    fn computed_member(&mut self, m: &'a ComputedMemberExpression<'a>) -> V<'a> {
        let o = self.eval(&m.object);
        let k = self.eval(&m.expression);
        self.member_by(o, k, m.span)
    }

    /// `o[k]`.
    pub(super) fn member_by(&mut self, o: V<'a>, k: V<'a>, span: Span) -> V<'a> {
        if matches!(o, V::Js(JsVal::Obj(Obj::ProcessEnv)))
            && let Some(v) = super::env::env_of_args(&k)
        {
            // A helper summary: `process.env[<argument>]`.
            return v;
        }
        let mut out = Vec::new();
        for key in k.leaves() {
            match key {
                V::Js(JsVal::Str(Value::Str(name))) => out.push(self.member(o.clone(), name, span)),
                // `o[undefined]`, `o[null]`: the properties of those names.
                V::Void => {
                    out.push(self.member(o.clone(), "undefined", span));
                    out.push(self.member(o.clone(), "null", span));
                }
                _ => out.push(match self.sym_index(&o, key, span) {
                    Some(v) => v,
                    None => self.any_member(o.clone(), span),
                }),
            }
        }
        join(out)
    }

    /// `o[<unknown key>]`: any property or element.
    pub(super) fn any_member(&mut self, o: V<'a>, span: Span) -> V<'a> {
        let leaves = o.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match l {
                V::Object(obj) => {
                    let v = obj.values().shared();
                    if any_written(&obj, &self.facts()) {
                        join([
                            v,
                            V::unknown(Tag::Unknown, "", "property written elsewhere"),
                        ])
                    } else {
                        join([v, V::Void])
                    }
                }
                V::Array(_) => self.elements(l, span),
                V::Js(JsVal::Obj(Obj::ProcessEnv)) => V::unknown(Tag::Env, "*", &self.text(span)),
                V::Js(JsVal::Unknown(_)) => match &l {
                    V::Js(j) => self.derived_any(j, span),
                    _ => V::Void,
                },
                V::Import(i) => V::Import(Rc::new(ImportRef {
                    module: i.module.clone(),
                    name: format!("{}.*", i.name).into(),
                })),
                V::Void => V::Void,
                V::Builtin(Builtin::Tmp) => l,
                _ => self.unk(Tag::Unknown, "", span),
            });
        }
        join(out)
    }

    /// `o.name`.
    pub(super) fn member(&mut self, o: V<'a>, name: &str, span: Span) -> V<'a> {
        let leaves = o.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(self.member_leaf(l, name, span));
        }
        join(out)
    }

    fn env(&self, name: &str) -> V<'a> {
        match name {
            // The temporary directory by convention.
            "TMPDIR" | "TMP" | "TEMP" | "RUNNER_TEMP" => {
                V::Js(JsVal::Path(Anchor::Tmp, Value::str("")))
            }
            _ => V::unknown(
                Tag::Env,
                name,
                &origin::clean(&format!("process.env.{name}")),
            ),
        }
    }

    fn member_leaf(&mut self, l: V<'a>, name: &str, span: Span) -> V<'a> {
        match l {
            V::Js(JsVal::Obj(Obj::ProcessEnv)) => self.env(name),
            V::Js(JsVal::Obj(Obj::Process)) if name == "argv" => {
                V::unknown(Tag::Argv, "", "process.argv")
            }
            V::Js(JsVal::Obj(o)) => match evaluator::obj_member(o, name, &self.scope()) {
                Some(j) => V::Js(j),
                None if o == Obj::Path && name == "win32" => V::Js(JsVal::Obj(Obj::Path)),
                None => self.unk(Tag::Unknown, "", span),
            },
            V::Js(JsVal::Url(a, p)) if matches!(name, "pathname" | "href") => {
                V::Js(JsVal::Url(a, p))
            }
            V::Js(JsVal::FileUrl) if name == "href" => V::Js(JsVal::FileUrl),
            V::Js(JsVal::FileUrl) if name == "pathname" => V::Js(self.scope().this_file()),
            // The URL of a file next to a resolved module, as a string or as its path.
            V::Js(JsVal::Resolved(m, p)) if matches!(name, "pathname" | "href") => {
                V::Js(JsVal::Resolved(m, p))
            }
            V::Js(j @ JsVal::Unknown(_)) => self.derived_member(&j, name, span),
            // `length` of a string or path is a number; anything else of a string-like
            // value (a character, a part of a URL object) is a string derived from it.
            V::Js(JsVal::Str(_) | JsVal::Path(..) | JsVal::File | JsVal::Resolved(..))
                if name == "length" =>
            {
                V::scalar()
            }
            V::Js(j) => self.derived(&j, span),
            V::Void => V::Void,
            V::Object(o) => self.obj_get(&o, name),
            V::Array(a) => match name.parse::<usize>() {
                Ok(i) => self.arr_at(&a, i),
                Err(_) if matches!(name, "length" | "size") => V::scalar(),
                Err(_) => V::Void,
            },
            V::Builtin(Builtin::Tmp) => V::Builtin(Builtin::Tmp),
            V::Builtin(Builtin::Fs) if name == "promises" => V::Builtin(Builtin::FsPromises),
            V::Builtin(b) => V::Method(b, name.into()),
            // `fs.realpathSync.native`
            V::Method(b, m) if name == "native" => V::Method(b, m),
            V::Method(..) | V::Func(_) | V::Loader(_) => self.unk(Tag::Unknown, "", span),
            V::Import(i) => V::Import(Rc::new(ImportRef {
                module: i.module.clone(),
                name: if &*i.name == "*" {
                    name.into()
                } else {
                    format!("{}.{name}", i.name).into()
                },
            })),
            V::Alts(_) => V::Void,
        }
    }

    pub(super) fn obj_get(&mut self, o: &ObjV<'a>, name: &str) -> V<'a> {
        let v = match o.get(name) {
            // Cross-file facts only: the per-file pass keeps joining the open value.
            Some(v) if self.sym.is_some() && o.is_exact(name) => {
                join(std::iter::once(v.clone()).chain(o.late.clone()))
            }
            Some(v) => join(std::iter::once(v.clone()).chain(o.open.clone())),
            None if o.map && name == "size" => V::scalar(),
            // Some other property may hold the open value; this one may also be missing.
            None => match &o.open {
                Some(open) => join([open.clone(), V::Void]),
                None => V::Void,
            },
        }
        .shared();
        if written(o, name, &self.facts()) {
            join([
                v,
                V::unknown(Tag::Unknown, "", "property written elsewhere"),
            ])
        } else {
            v
        }
    }

    fn arr_at(&mut self, a: &ArrV<'a>, i: usize) -> V<'a> {
        // Reordered somewhere in the file (`xs.sort()` on an alias): any element.
        let at = if a.trust == Trust::Shared && self.facts().array_reorder() {
            a.elems.to_vec()
        } else {
            a.elems.get(i).cloned().into_iter().collect()
        };
        let v = join(at.into_iter().chain(a.rest.clone())).shared();
        if a.trust == Trust::Shared && self.facts().array_mutation() {
            join([v, V::unknown(Tag::Unknown, "", "array modified elsewhere")])
        } else {
            v
        }
    }

    /// Any element of an iterable.
    pub(super) fn elements(&mut self, v: V<'a>, span: Span) -> V<'a> {
        let leaves = v.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match l {
                V::Array(a) => {
                    let v = a.elements().shared();
                    if a.trust == Trust::Shared && self.facts().array_mutation() {
                        join([v, V::unknown(Tag::Unknown, "", "array modified elsewhere")])
                    } else {
                        v
                    }
                }
                V::Js(j @ JsVal::Unknown(_)) => self.derived_elem(&j, span),
                V::Void => V::Void,
                V::Builtin(Builtin::Tmp) => l,
                V::Import(i) => V::Import(Rc::new(ImportRef {
                    module: i.module.clone(),
                    name: format!("{}[]", i.name).into(),
                })),
                _ => self.unk(Tag::Unknown, "", span),
            });
        }
        join(out)
    }

    /// Any key of an object (`for…in`, `Object.keys`).
    fn keys(&mut self, v: V<'a>, span: Span) -> V<'a> {
        let leaves = v.into_leaves();
        let mut out = Vec::new();
        for l in leaves {
            match l {
                V::Object(o) => {
                    out.extend(o.props.iter().map(|(k, _)| V::str(&**k)));
                    if o.open.is_some() || any_written(&o, &self.facts()) {
                        out.push(self.unk(Tag::Unknown, "", span));
                    }
                }
                V::Js(j @ JsVal::Unknown(_)) => out.push(self.derived(&j, span)),
                V::Void => {}
                _ => out.push(self.unk(Tag::Unknown, "", span)),
            }
        }
        join(out)
    }

    // ---- literals ----

    fn prop_key(&mut self, key: &'a PropertyKey<'a>, computed: bool) -> Option<String> {
        match key {
            PropertyKey::StaticIdentifier(id) => Some(id.name.as_str().to_owned()),
            PropertyKey::PrivateIdentifier(id) => Some(format!("#{}", id.name)),
            PropertyKey::StringLiteral(s) if !computed => s.value.as_str().map(str::to_owned),
            PropertyKey::NumericLiteral(n) if !computed => self
                .src
                .get(n.span.start as usize..n.span.end as usize)
                .map(str::to_owned),
            other => {
                let e = other.as_expression()?;
                match self.eval(e) {
                    V::Js(JsVal::Str(Value::Str(s))) => Some(s),
                    _ => None,
                }
            }
        }
    }

    fn object(&mut self, o: &'a ObjectExpression<'a>) -> V<'a> {
        let mut obj = ObjV {
            props: Vec::new(),
            open: None,
            exact: Vec::new(),
            late: None,
            trust: Trust::Fresh,
            map: false,
        };
        for p in &o.properties {
            if obj.props.len() >= MAX_ITEMS || !self.tick() {
                obj.widen(self.unk(Tag::Unknown, "", o.span));
                break;
            }
            match p {
                ObjectPropertyKind::ObjectProperty(p) => {
                    let key = self.prop_key(&p.key, p.computed);
                    let val = match p.kind {
                        PropertyKind::Init => self.eval(&p.value),
                        // An accessor computes its value.
                        PropertyKind::Get | PropertyKind::Set => self.unk(Tag::Unknown, "", p.span),
                    };
                    // A setter that uses `this` can write any property on an assignment
                    // to its own; `__proto__: base` inherits every property of `base`.
                    let setter = p.kind == PropertyKind::Set
                        && matches!(&p.value, Expression::FunctionExpression(f)
                            if self.facts().uses_this(f.span.start));
                    let proto = p.kind == PropertyKind::Init
                        && !p.computed
                        && !p.shorthand
                        && key.as_deref() == Some("__proto__");
                    if setter || proto {
                        obj.widen(self.unk(Tag::Unknown, "", p.span));
                    }
                    match key {
                        Some(k) => {
                            obj.set(&k, val, false);
                            // Written by name after every spread so far: the spreads'
                            // unknown properties do not reach it (cross-file facts
                            // only: the per-file pass does not track it).
                            if self.sym.is_some()
                                && p.kind == PropertyKind::Init
                                && !setter
                                && !proto
                            {
                                obj.set_exact(&k);
                            }
                        }
                        None => obj.widen(val),
                    }
                }
                ObjectPropertyKind::SpreadProperty(s) => {
                    let v = self.eval(&s.argument);
                    let weak = v.leaves().len() > 1;
                    for l in v.into_leaves() {
                        match l {
                            V::Object(src) => {
                                let shared = src.trust == Trust::Shared;
                                for (k, v) in &src.props {
                                    let v = if shared {
                                        self.obj_get(&src, k)
                                    } else {
                                        v.clone().shared()
                                    };
                                    obj.set(k, v, weak);
                                }
                                if let Some(op) = &src.open {
                                    obj.widen(op.clone());
                                }
                                if any_written(&src, &self.facts()) {
                                    obj.widen(V::unknown(
                                        Tag::Unknown,
                                        "",
                                        "property written elsewhere",
                                    ));
                                }
                            }
                            V::Void => {}
                            V::Js(j @ JsVal::Unknown(_)) => obj.widen(self.derived(&j, s.span)),
                            _ => obj.widen(self.unk(Tag::Unknown, "", s.span)),
                        }
                    }
                }
            }
        }
        V::Object(Rc::new(obj))
    }

    fn array(&mut self, a: &'a ArrayExpression<'a>) -> V<'a> {
        let mut arr = ArrV {
            elems: Vec::new(),
            rest: None,
            trust: Trust::Fresh,
        };
        for el in &a.elements {
            if arr.elems.len() >= MAX_ITEMS || !self.tick() {
                arr.add(self.unk(Tag::Unknown, "", a.span));
                break;
            }
            match el {
                ArrayExpressionElement::SpreadElement(s) => {
                    let v = self.eval(&s.argument);
                    match &v {
                        V::Array(src)
                            if src.rest.is_none()
                                && arr.rest.is_none()
                                && src.trust != Trust::Shared =>
                        {
                            arr.elems.extend(src.elems.iter().cloned());
                        }
                        _ => {
                            let els = self.elements(v, s.span);
                            arr.add(els);
                        }
                    }
                }
                ArrayExpressionElement::Elision(_) => arr.elems.push(V::Void),
                other => {
                    let Some(e) = other.as_expression() else {
                        continue;
                    };
                    let v = self.eval(e);
                    if arr.rest.is_some() {
                        arr.add(v);
                    } else {
                        arr.elems.push(v);
                    }
                }
            }
        }
        V::Array(Rc::new(arr))
    }

    // ---- bindings ----

    /// The value of a binding: the join of all its definitions.
    pub(super) fn symbol(&mut self, sym: SymbolId) -> V<'a> {
        // A parameter of the call being inlined.
        if let Some(frame) = self.frames.last()
            && let Some((_, v)) = frame.iter().find(|(s, _)| *s == sym)
        {
            self.frame_used = true;
            let v = v.clone();
            if !self
                .scoping
                .get_resolved_references(sym)
                .any(|r| r.is_write())
            {
                return v;
            }
            // Reassigned in the body: also every assigned value.
            let (mut vals, plus) = self.writes(sym);
            vals.push(v);
            return self.with_plus(join(vals), plus);
        }
        if let Some(v) = self.memo.get(&sym) {
            return v.clone();
        }
        if let Some(i) = self.busy.iter().rposition(|b| b.sym == sym) {
            self.lowest_hit = self.lowest_hit.min(i);
            return self.busy[i].partial.clone();
        }
        if !self.tick() || self.busy.len() as u32 >= MAX_DEPTH {
            return V::unknown(
                Tag::Unknown,
                "",
                &origin::clean(self.scoping.symbol_name(sym)),
            );
        }
        let k = self.busy.len();
        self.busy.push(Busy {
            sym,
            partial: V::Void,
        });
        let outer_low = std::mem::replace(&mut self.lowest_hit, usize::MAX);
        let outer_used = std::mem::replace(&mut self.frame_used, false);
        let mut v = self.definitions(sym);
        // A definition that refers to the binding itself (`dir = resolve(dir)`):
        // evaluate again with the previous result until it is stable; otherwise
        // widen.
        let mut pass = 1;
        while self.lowest_hit <= k && pass < MAX_PASSES {
            self.busy[k].partial = v.clone();
            let next = self.definitions(sym);
            pass += 1;
            if next == v {
                break;
            }
            v = next;
            if pass == MAX_PASSES {
                // In symbolic mode, a binding that keeps reading deeper into the same
                // values (a function that walks a structure by calling itself) is
                // everything those reads can reach.
                v = match self.sym_widen(sym, k, &v) {
                    Some(w) => w,
                    None => {
                        let name = origin::clean(self.scoping.symbol_name(sym));
                        join([v, V::unknown(Tag::Unknown, "", &name)])
                    }
                };
            }
        }
        self.busy.truncate(k);
        // Containers: trusted only as far as the uses of the binding allow.
        if v.has_container() {
            v = if self.literal_binding(sym) {
                self.stabilize(sym, v)
            } else {
                v.shared()
            };
        }
        let low = self.lowest_hit;
        let used = self.frame_used;
        // Memoize unless the value depends on an outer binding still being evaluated or
        // on the parameters of an inlined call.
        if low >= k && !used && self.site_steps > 0 {
            self.memo.insert(sym, v.clone());
        }
        self.lowest_hit = if low < k {
            outer_low.min(low)
        } else {
            outer_low
        };
        self.frame_used = outer_used || used;
        v
    }

    /// Sets the value handed to definitions that refer to the binding at
    /// position `k` of the bindings being evaluated.
    pub(super) fn busy_partial(&mut self, k: usize, v: V<'a>) {
        if let Some(b) = self.busy.get_mut(k) {
            b.partial = v;
        }
    }

    /// The join of the definitions of `sym` (one evaluation, no fixpoint).
    pub(super) fn definitions_of(&mut self, sym: SymbolId) -> V<'a> {
        self.definitions(sym)
    }

    fn definitions(&mut self, sym: SymbolId) -> V<'a> {
        let scoping = self.scoping;
        let mut vals = Vec::new();
        let mut decls: Vec<NodeId> = scoping.symbol_declarations(sym).take(9).collect();
        if decls.len() > 8 {
            // Too many redeclarations to join: one of them is not known.
            decls.truncate(8);
            vals.push(V::unknown(
                Tag::Unknown,
                "",
                &origin::clean(scoping.symbol_name(sym)),
            ));
        }
        for d in decls {
            vals.push(self.declared(sym, d));
        }
        let (w, plus) = self.writes(sym);
        vals.extend(w);
        self.with_plus(join(vals), plus)
    }

    /// `x += e`: any of the other values, optionally followed by something
    /// derived from `e`.
    fn with_plus(&mut self, base: V<'a>, plus: Vec<&'a Expression<'a>>) -> V<'a> {
        if plus.is_empty() {
            return base;
        }
        let mut out = vec![base.clone()];
        // A value that may be a number is added to, not appended to (`n += 1`).
        let (numbers, strings): (Vec<V<'a>>, Vec<V<'a>>) =
            base.into_leaves().into_iter().partition(V::maybe_number);
        let strings = join(strings);
        for e in plus {
            let rhs = self.eval(e);
            let mut tails = Vec::new();
            for l in rhs.leaves() {
                tails.push(match l {
                    l if l.is_scalar() => self.unk(Tag::Unknown, "", e.span()),
                    V::Js(j @ JsVal::Unknown(_)) => self.derived(j, e.span()),
                    V::Void => V::Void,
                    _ => self.unk(Tag::Listing, "", e.span()),
                });
            }
            let tail = join(tails);
            if !numbers.is_empty() {
                out.push(self.unk(Tag::Unknown, "", e.span()));
            }
            if strings != V::Void {
                out.push(self.concat(strings.clone(), tail, e.span()));
            }
        }
        join(out)
    }

    /// The values assigned to `sym` after its declaration, and the operands of
    /// `+=`.
    fn writes(&mut self, sym: SymbolId) -> (Vec<V<'a>>, Vec<&'a Expression<'a>>) {
        let scoping = self.scoping;
        let mut vals = Vec::new();
        let mut plus = Vec::new();
        for r in scoping.get_resolved_references(sym) {
            if !r.is_write() {
                continue;
            }
            if !self.tick() {
                vals.push(V::unknown(Tag::Unknown, "", "too many assignments"));
                break;
            }
            let node = r.node_id();
            let AstKind::IdentifierReference(id) = self.nodes.kind(node) else {
                vals.push(V::unknown(Tag::Unknown, "", "assignment"));
                continue;
            };
            let p = self.nodes.parent_id(node);
            match self.nodes.kind(p) {
                AstKind::AssignmentExpression(a) if a.left.span() == id.span => match a.operator {
                    AssignmentOperator::Assign
                    | AssignmentOperator::LogicalOr
                    | AssignmentOperator::LogicalAnd
                    | AssignmentOperator::LogicalNullish => vals.push(self.eval(&a.right)),
                    AssignmentOperator::Addition => plus.push(&a.right),
                    _ => vals.push(self.unk(Tag::Unknown, "", a.span)),
                },
                AstKind::ForOfStatement(f) if f.left.span() == id.span => {
                    let it = self.eval(&f.right);
                    vals.push(self.elements(it, f.right.span()));
                }
                AstKind::ForInStatement(f) if f.left.span() == id.span => {
                    let o = self.eval(&f.right);
                    vals.push(self.keys(o, f.right.span()));
                }
                other => vals.push(self.unk(Tag::Unknown, "", other.span())),
            }
        }
        (vals, plus)
    }

    /// The value `sym` gets from the declaration node `decl`.
    fn declared(&mut self, sym: SymbolId, decl: NodeId) -> V<'a> {
        let name = origin::clean(self.scoping.symbol_name(sym));
        match self.nodes.kind(decl) {
            AstKind::VariableDeclarator(d) => {
                let list = self.nodes.parent_id(decl);
                let list_span = self.nodes.kind(list).span();
                let init = match self.nodes.parent_kind(list) {
                    AstKind::ForOfStatement(f) if f.left.span() == list_span => {
                        let it = self.eval(&f.right);
                        self.elements(it, f.right.span())
                    }
                    AstKind::ForInStatement(f) if f.left.span() == list_span => {
                        let o = self.eval(&f.right);
                        self.keys(o, f.right.span())
                    }
                    _ => match &d.init {
                        Some(e) => self.eval(e),
                        None => match self.nodes.kind(list) {
                            AstKind::VariableDeclaration(v) if v.declare => {
                                V::unknown(Tag::Unknown, "", &name)
                            }
                            _ => V::Void,
                        },
                    },
                };
                self.pick(&d.id, init, sym)
            }
            AstKind::FormalParameter(p) => self.param(p, decl, sym),
            AstKind::Function(f) => {
                if f.body.is_some() {
                    // In sloppy code a function declared in a block is also a `var` of the
                    // enclosing function, shared with the declarations of that name in
                    // other blocks: which one a call outside the block reaches is not
                    // known here.
                    let nested = !matches!(
                        self.nodes.parent_kind(decl),
                        AstKind::Program(_)
                            | AstKind::FunctionBody(_)
                            | AstKind::ExportNamedDeclaration(_)
                            | AstKind::ExportDefaultDeclaration(_)
                            | AstKind::TSModuleBlock(_)
                    );
                    let strict = self
                        .scoping
                        .scope_flags(self.scoping.symbol_scope_id(sym))
                        .is_strict_mode();
                    if nested && !strict {
                        join([
                            V::Func(FnRef::Function(f)),
                            V::unknown(Tag::Unknown, "", &name),
                        ])
                    } else {
                        V::Func(FnRef::Function(f))
                    }
                } else if f.declare {
                    // `declare function`: defined somewhere else.
                    V::unknown(Tag::Unknown, "", &name)
                } else {
                    // An overload signature: the implementation is another declaration.
                    V::Void
                }
            }
            AstKind::ImportDefaultSpecifier(_) => match self.import_source(decl) {
                Some(m) => match self.module_value(m) {
                    V::Import(i) => V::Import(Rc::new(ImportRef {
                        module: i.module.clone(),
                        name: "default".into(),
                    })),
                    v => v,
                },
                None => V::unknown(Tag::Unknown, "", &name),
            },
            AstKind::ImportNamespaceSpecifier(_) => match self.import_source(decl) {
                Some(m) => self.module_value(m),
                None => V::unknown(Tag::Unknown, "", &name),
            },
            AstKind::ImportSpecifier(s) => {
                let imported = match &s.imported {
                    ModuleExportName::IdentifierName(i) => Some(i.name.as_str()),
                    ModuleExportName::IdentifierReference(i) => Some(i.name.as_str()),
                    ModuleExportName::StringLiteral(l) => l.value.as_str(),
                };
                match (self.import_source(decl), imported) {
                    (Some(m), Some(n)) if s.import_kind == ImportOrExportKind::Value => {
                        let module = self.module_value(m);
                        if n == "default" {
                            match module {
                                V::Import(i) => V::Import(Rc::new(ImportRef {
                                    module: i.module.clone(),
                                    name: "default".into(),
                                })),
                                v => v,
                            }
                        } else {
                            self.member(module, n, s.span)
                        }
                    }
                    _ => V::unknown(Tag::Unknown, "", &name),
                }
            }
            AstKind::TSImportEqualsDeclaration(d) => match &d.module_reference {
                TSModuleReference::ExternalModuleReference(r) => {
                    match r.expression.value.as_str() {
                        Some(m) => self.module_value(m),
                        None => V::unknown(Tag::Unknown, "", &name),
                    }
                }
                _ => V::unknown(Tag::Unknown, "", &name),
            },
            AstKind::TSEnumDeclaration(e) => {
                let mut props = Vec::new();
                // Numeric members also map their number back to their name (`E[1]`).
                let mut reverse: Vec<(Rc<str>, V<'a>)> = Vec::new();
                let mut next: Option<u64> = Some(0);
                let mut open = None;
                for m in &e.body.members {
                    let key = match &m.id {
                        TSEnumMemberName::Identifier(i) => Some(i.name.as_str()),
                        TSEnumMemberName::String(s) | TSEnumMemberName::ComputedString(s) => {
                            s.value.as_str()
                        }
                        TSEnumMemberName::ComputedTemplateString(_) => None,
                    };
                    let Some(key) = key else {
                        open = Some(self.unk(Tag::Unknown, "", m.span));
                        next = None;
                        continue;
                    };
                    let v = match &m.initializer {
                        Some(init) => self.eval(init),
                        None => self.unk(Tag::Unknown, "", m.span),
                    };
                    let number = match &m.initializer {
                        None => next,
                        Some(Expression::NumericLiteral(_)) => match &v {
                            V::Js(JsVal::Str(Value::Str(d))) => d.parse().ok(),
                            _ => None,
                        },
                        Some(Expression::StringLiteral(_) | Expression::TemplateLiteral(_)) => {
                            // A string member has no reverse mapping.
                            next = None;
                            props.push((Rc::from(key), v));
                            continue;
                        }
                        Some(_) => None,
                    };
                    match number {
                        Some(n) => {
                            reverse.push((Rc::from(n.to_string()), V::str(key)));
                            next = n.checked_add(1);
                        }
                        // A computed number: some key maps back to this name.
                        None => {
                            open = Some(self.unk(Tag::Unknown, "", m.span));
                            next = None;
                        }
                    }
                    props.push((Rc::from(key), v));
                }
                props.extend(reverse);
                V::object(props, open)
            }
            AstKind::CatchParameter(_) => V::unknown(Tag::Unknown, "", &name),
            AstKind::FormalParameterRest(_) | AstKind::BindingRestElement(_) => {
                match self.rest_hole(decl, &name) {
                    Some(v) => v,
                    None => V::unknown(Tag::Unknown, "", &name),
                }
            }
            _ => V::unknown(Tag::Unknown, "", &name),
        }
    }

    /// The module specifier of the import declaration containing the specifier
    /// `decl` (`None` for type-only imports).
    fn import_source(&self, decl: NodeId) -> Option<&'a str> {
        match self.nodes.parent_kind(decl) {
            AstKind::ImportDeclaration(d) if d.import_kind == ImportOrExportKind::Value => {
                d.source.value.as_str()
            }
            _ => None,
        }
    }

    /// The value of a module's namespace (for the builtin modules, the module
    /// object).
    pub(super) fn module_value(&self, spec: &str) -> V<'a> {
        match spec.strip_prefix("node:").unwrap_or(spec) {
            "path" | "path/posix" => V::Js(JsVal::Obj(Obj::Path)),
            "url" => V::Js(JsVal::Obj(Obj::UrlModule)),
            "process" => V::Js(JsVal::Obj(Obj::Process)),
            "os" => V::Builtin(Builtin::Os),
            "fs" | "fs-extra" | "fs-extra/esm" | "graceful-fs" => V::Builtin(Builtin::Fs),
            "fs/promises" => V::Builtin(Builtin::FsPromises),
            "module" => V::Builtin(Builtin::Module),
            "resolve-from" => V::Js(JsVal::Func(Func::ResolveFrom)),
            "tmp" | "tmp-promise" | "tempy" | "temp-dir" | "temp" | "tempfile" => {
                V::Builtin(Builtin::Tmp)
            }
            m => V::Import(Rc::new(ImportRef {
                module: m.into(),
                name: "*".into(),
            })),
        }
    }

    /// Destructures `v` with `pattern` into `out`.
    fn destructure(
        &mut self,
        pattern: &'a BindingPattern<'a>,
        v: V<'a>,
        out: &mut Vec<(SymbolId, V<'a>)>,
    ) {
        if !self.tick() {
            return;
        }
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                if let Some(s) = id.symbol_id.get() {
                    out.push((s, v));
                }
            }
            BindingPattern::ObjectPattern(o) => {
                for p in &o.properties {
                    let sub = match self.prop_key(&p.key, p.computed) {
                        Some(k) => self.member(v.clone(), &k, p.span),
                        None => self.any_member(v.clone(), p.span),
                    };
                    self.destructure(&p.value, sub, out);
                }
                if let Some(r) = &o.rest {
                    // The remaining properties: a subset of the object.
                    self.destructure(&r.argument, v, out);
                }
            }
            BindingPattern::ArrayPattern(a) => {
                for (i, el) in a.elements.iter().enumerate() {
                    if let Some(el) = el {
                        let sub = self.member(v.clone(), &i.to_string(), a.span);
                        self.destructure(el, sub, out);
                    }
                }
                if let Some(r) = &a.rest {
                    let els = self.elements(v, a.span);
                    self.destructure(&r.argument, V::array(Vec::new(), Some(els)), out);
                }
            }
            BindingPattern::AssignmentPattern(ap) => {
                let v = self.with_default(v, &ap.right);
                self.destructure(&ap.left, v, out);
            }
        }
    }

    /// The part of `v` that `pattern` binds to `sym`.
    fn pick(&mut self, pattern: &'a BindingPattern<'a>, v: V<'a>, sym: SymbolId) -> V<'a> {
        if let BindingPattern::BindingIdentifier(_) = pattern {
            return v;
        }
        let mut out = Vec::new();
        self.destructure(pattern, v, &mut out);
        match out.into_iter().find(|(s, _)| *s == sym) {
            Some((_, v)) => v,
            None => V::unknown(
                Tag::Unknown,
                "",
                &origin::clean(self.scoping.symbol_name(sym)),
            ),
        }
    }

    /// `v`, or the default `init` where `v` may be `undefined`.
    fn with_default(&mut self, v: V<'a>, init: &'a Expression<'a>) -> V<'a> {
        let mut out = Vec::new();
        let mut need = false;
        for l in v.into_leaves() {
            match l {
                V::Void => need = true,
                V::Js(JsVal::Unknown(_)) | V::Import(_) => {
                    need = true;
                    out.push(l);
                }
                l => out.push(l),
            }
        }
        if need {
            out.push(self.eval(init));
        }
        join(out)
    }

    // ---- parameters ----

    /// The value of the parameter `p` (declaration node `decl`) outside an
    /// inlined call: the join of the arguments at every call the analysis
    /// sees, plus an unknown with origin `Param` when there can be others.
    fn param(&mut self, p: &'a FormalParameter<'a>, decl: NodeId, sym: SymbolId) -> V<'a> {
        let name = origin::clean(self.scoping.symbol_name(sym));
        let list = self.nodes.parent_id(decl);
        let fn_id = self.nodes.parent_id(list);
        let (f, items) = match self.nodes.kind(fn_id) {
            AstKind::Function(f) => (FnRef::Function(f), &f.params.items),
            AstKind::ArrowFunctionExpression(f) => (FnRef::Arrow(f), &f.params.items),
            _ => return V::unknown(Tag::Unknown, "", &name),
        };
        let Some(index) = items.iter().position(|x| x.span == p.span) else {
            return V::unknown(Tag::Unknown, "", &name);
        };
        let reach = self.reach(f, fn_id);
        // The call sites are other activations: the frames of inlined calls do not
        // apply.
        let frames = std::mem::take(&mut self.frames);
        let mut vals = Vec::new();
        match &*reach {
            Reach::Named {
                calls,
                escapes,
                name: fname,
            } => {
                for c in calls.iter().take(MAX_CALL_SITES) {
                    let args = self.args(&c.arguments);
                    vals.push(args.get(index));
                }
                if calls.len() > MAX_CALL_SITES {
                    vals.push(V::unknown(Tag::Unknown, "", &name));
                }
                if *escapes {
                    vals.push(self.hole(f, index, &format!("{fname}#{index}"), &name));
                }
            }
            Reach::Callback { call, arg } => {
                let v = self.callback_param(call, *arg, index, &name);
                vals.push(self.holes_in(v, f, index));
            }
            Reach::Unseen { name: fname } => {
                vals.push(self.hole(f, index, &format!("{fname}#{index}"), &name));
            }
        }
        self.frames = frames;
        let mut v = join(vals);
        if let Some(init) = &p.initializer {
            v = self.with_default(v, init);
        }
        if self.facts().uses_arguments(f.span().start) {
            // `arguments[i] = x` aliases the parameter.
            v = join([v, V::unknown(Tag::Unknown, "", &name)]);
        }
        // A parameter is an alias of whatever the callers pass.
        self.pick(&p.pattern, v.shared(), sym)
    }

    /// How the function `f` (node `fn_id`) can be called.
    pub(super) fn reach(&mut self, f: FnRef<'a>, fn_id: NodeId) -> Rc<Reach<'a>> {
        let key = f.span().start;
        if let Some(r) = self.reach.get(&key) {
            return r.clone();
        }
        let r = Rc::new(self.compute_reach(f, fn_id));
        self.reach.insert(key, r.clone());
        r
    }

    fn compute_reach(&mut self, f: FnRef<'a>, fn_id: NodeId) -> Reach<'a> {
        let scoping = self.scoping;
        let mut symbols: Vec<SymbolId> = Vec::new();
        let mut exported = false;
        let mut name: Box<str> = "function".into();
        let own = match f {
            FnRef::Function(func) => func.id.as_ref(),
            FnRef::Arrow(_) => None,
        };
        if let Some(id) = own {
            name = id.name.as_str().into();
            symbols.extend(id.symbol_id.get());
        }
        let declaration = matches!(f, FnRef::Function(func) if func.is_declaration());
        let (pid, pk) = self.parent(fn_id);
        if declaration {
            exported = matches!(
                pk,
                AstKind::ExportDeclaration(_)
                    | AstKind::ExportNamedDeclaration(_)
                    | AstKind::ExportDefaultDeclaration(_)
            );
        } else {
            match pk {
                AstKind::VariableDeclarator(d) => {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        return Reach::Unseen { name };
                    };
                    name = id.name.as_str().into();
                    symbols.extend(id.symbol_id.get());
                    let list = self.nodes.parent_id(pid);
                    exported = matches!(
                        self.nodes.parent_kind(list),
                        AstKind::ExportDeclaration(_)
                            | AstKind::ExportNamedDeclaration(_)
                            | AstKind::ExportDefaultDeclaration(_)
                    );
                }
                AstKind::AssignmentExpression(a) => {
                    // `name = function () {}`
                    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                        return Reach::Unseen {
                            name: self.text(a.left.span()).into(),
                        };
                    };
                    name = id.name.as_str().into();
                    match id
                        .reference_id
                        .get()
                        .and_then(|r| scoping.get_reference(r).symbol_id())
                    {
                        Some(s) => symbols.push(s),
                        None => return Reach::Unseen { name },
                    }
                }
                AstKind::CallExpression(c) => {
                    let fspan = f.span();
                    if c.callee.get_inner_expression().span() == fspan {
                        // An immediately invoked function. Its own name can call it again
                        // with other arguments.
                        let recursive = symbols
                            .iter()
                            .any(|s| scoping.get_resolved_references(*s).next().is_some());
                        return Reach::Named {
                            calls: vec![c],
                            escapes: recursive,
                            name,
                        };
                    }
                    return match c.arguments.iter().position(|a| {
                        a.as_expression()
                            .is_some_and(|e| e.get_inner_expression().span() == fspan)
                    }) {
                        Some(arg) => Reach::Callback { call: c, arg },
                        None => Reach::Unseen { name },
                    };
                }
                AstKind::ObjectProperty(p) => {
                    return Reach::Unseen {
                        name: self.text(p.key.span()).into(),
                    };
                }
                AstKind::MethodDefinition(m) => {
                    return Reach::Unseen {
                        name: self.text(m.key.span()).into(),
                    };
                }
                AstKind::PropertyDefinition(p) => {
                    return Reach::Unseen {
                        name: self.text(p.key.span()).into(),
                    };
                }
                _ if symbols.is_empty() => return Reach::Unseen { name },
                // A named function expression used as a value: its own name only sees the
                // recursive calls.
                _ => exported = true,
            }
        }
        if symbols.is_empty() {
            return Reach::Unseen { name };
        }
        let mut calls = Vec::new();
        let mut escapes = exported;
        for sym in symbols {
            for r in scoping.get_resolved_references(sym) {
                if !self.tick() {
                    escapes = true;
                    break;
                }
                if r.is_write() && !r.is_read() {
                    continue;
                }
                let node = r.node_id();
                let span = self.nodes.kind(node).span();
                match self.parent(node) {
                    (_, AstKind::CallExpression(c))
                        if c.callee.get_inner_expression().span() == span =>
                    {
                        calls.push(c);
                    }
                    _ => escapes = true,
                }
            }
        }
        // A function nothing in the file refers to is still reachable where top-level
        // declarations are globals (classic scripts): no call site bounds its
        // parameters.
        if calls.is_empty() {
            escapes = true;
        }
        Reach::Named {
            calls,
            escapes,
            name,
        }
    }

    /// The value the callee of `call` passes as parameter `param` of its
    /// callback argument number `arg`.
    fn callback_param(
        &mut self,
        call: &'a CallExpression<'a>,
        arg: usize,
        param: usize,
        name: &str,
    ) -> V<'a> {
        let callee = call.callee.get_inner_expression();
        let unseen = |this: &Self| {
            let c = this.text(callee.span());
            V::unknown(Tag::Param, &format!("callback of {c}#{param}"), name)
        };
        match callee {
            Expression::StaticMemberExpression(m) => {
                let method = m.property.name.as_str();
                let iterates = matches!(
                    method,
                    "forEach"
                        | "map"
                        | "filter"
                        | "find"
                        | "findLast"
                        | "findIndex"
                        | "findLastIndex"
                        | "some"
                        | "every"
                        | "flatMap"
                );
                match (method, arg, param) {
                    (_, 0, 0) if iterates => {
                        let recv = self.eval(&m.object);
                        // Only an array has these methods: a string or a plain object
                        // in the receiver's place has no elements to hand out.
                        let recv = self.sym_as_array(recv, m.object.span());
                        self.elements(recv, m.object.span())
                    }
                    // The index.
                    (_, 0, 1) if iterates => V::unknown(Tag::Listing, "", name),
                    (_, 0, 2) if iterates => self.eval(&m.object),
                    ("reduce" | "reduceRight", 0, 1) => {
                        let recv = self.eval(&m.object);
                        self.elements(recv, m.object.span())
                    }
                    ("sort" | "toSorted", 0, 0 | 1) => {
                        let recv = self.eval(&m.object);
                        self.elements(recv, m.object.span())
                    }
                    ("then", 0, 0) => self.eval(&m.object),
                    _ => unseen(self),
                }
            }
            // `describe.each(table)(title, (a, b) => ...)`
            Expression::CallExpression(inner)
                if matches!(
                    inner.callee.get_inner_expression(),
                    Expression::StaticMemberExpression(m) if m.property.name == "each"
                ) && arg >= 1 =>
            {
                let Some(table) = inner.arguments.first().and_then(|a| a.as_expression()) else {
                    return unseen(self);
                };
                let table = self.eval(table);
                let rows = self.elements(table, inner.span);
                let mut out = Vec::new();
                for row in rows.into_leaves() {
                    match row {
                        V::Array(_) => out.push(self.member(row, &param.to_string(), inner.span)),
                        // A row that is not known (an imported table, a call result) can be
                        // an array: any of its elements.
                        V::Js(j @ JsVal::Unknown(_)) => out.push(self.derived(&j, inner.span)),
                        V::Import(_) | V::Builtin(_) | V::Method(..) => {
                            out.push(self.elements(row, inner.span))
                        }
                        // A row that is not an array is the only argument.
                        other if param == 0 => out.push(other),
                        _ => out.push(V::Void),
                    }
                }
                join(out)
            }
            _ => unseen(self),
        }
    }

    // ---- calls ----

    pub(super) fn args(&mut self, args: &'a [Argument<'a>]) -> Args<'a> {
        let mut out = Args {
            vals: Vec::with_capacity(args.len()),
            rest: None,
        };
        for a in args {
            match a {
                Argument::SpreadElement(s) => {
                    let v = self.eval(&s.argument);
                    match &v {
                        V::Array(arr)
                            if arr.rest.is_none()
                                && out.rest.is_none()
                                && arr.trust != Trust::Shared =>
                        {
                            out.vals.extend(arr.elems.iter().cloned());
                        }
                        _ => {
                            let els = self.elements(v, s.span);
                            out.rest = Some(match out.rest.take() {
                                Some(r) => join([r, els]),
                                None => els,
                            });
                        }
                    }
                }
                other => {
                    let Some(e) = other.as_expression() else {
                        continue;
                    };
                    let v = self.eval(e);
                    match out.rest.take() {
                        Some(r) => out.rest = Some(join([r, v])),
                        None => out.vals.push(v),
                    }
                }
            }
        }
        out
    }

    fn call(&mut self, c: &'a CallExpression<'a>) -> V<'a> {
        match c.callee.get_inner_expression() {
            Expression::StaticMemberExpression(m) => {
                let recv = self.eval(&m.object);
                self.method(recv, m.property.name.as_str(), &c.arguments, c.span)
            }
            Expression::ComputedMemberExpression(m) => {
                let recv = self.eval(&m.object);
                match self.eval(&m.expression) {
                    V::Js(JsVal::Str(Value::Str(name))) => {
                        self.method(recv, &name, &c.arguments, c.span)
                    }
                    _ => {
                        let f = self.any_member(recv, m.span);
                        self.call_value(f, &c.arguments, c.span)
                    }
                }
            }
            callee => {
                let f = self.eval(callee);
                self.call_value(f, &c.arguments, c.span)
            }
        }
    }

    /// Calls the value `f`.
    pub(super) fn call_value(&mut self, f: V<'a>, args: &'a [Argument<'a>], span: Span) -> V<'a> {
        let leaves = f.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match l {
                V::Func(fr) => {
                    let a = self.args(args);
                    self.inline(fr, &a, span)
                }
                V::Js(JsVal::Func(func)) => {
                    let a = self.args(args);
                    self.apply(func, &a, span)
                }
                V::Js(JsVal::Obj(Obj::Require)) => {
                    let a = self.args(args);
                    match a.get(0) {
                        V::Js(JsVal::Str(Value::Str(m))) => self.module_value(&m),
                        v => match self.sym_require(&v, span) {
                            Some(x) => x,
                            None => self.unk(Tag::Call, "require", span),
                        },
                    }
                }
                // `String(x)`
                V::Js(JsVal::Obj(Obj::StringCtor)) => {
                    let v = self.args(args).get(0);
                    self.stringified(v, span)
                }
                V::Method(b, name) => self.builtin(b, &name, args, span),
                // The module a `require` made by `createRequire(elsewhere)` loads: not a
                // value of the file.
                V::Loader(_) => self.unk(Tag::Call, "require", span),
                V::Builtin(Builtin::Tmp) => V::Builtin(Builtin::Tmp),
                V::Import(_) | V::Js(JsVal::Unknown(_)) if self.sym.is_some() => {
                    match self.sym_call(&l, args, span) {
                        Some(v) => v,
                        None => match &l {
                            V::Js(j) => self.derived_call(j, span),
                            _ => self.unk(Tag::Call, "", span),
                        },
                    }
                }
                V::Import(i) => {
                    let a = self.args(args);
                    self.import_call(&i.module, &i.name, &a, span)
                }
                V::Js(j @ JsVal::Unknown(_)) => self.derived_call(&j, span),
                V::Void => V::Void,
                _ => self.unk(Tag::Call, "", span),
            });
        }
        join(out)
    }

    /// The returns of a function.
    pub(super) fn fn_info(&mut self, f: FnRef<'a>) -> Rc<FnInfo<'a>> {
        let key = f.span().start;
        if let Some(i) = self.fns.get(&key) {
            return i.clone();
        }
        let mut info = FnInfo {
            returns: Vec::new(),
            generator: false,
        };
        match f {
            FnRef::Function(func) => {
                info.generator = func.generator;
                if let Some(b) = &func.body {
                    for s in &b.statements {
                        collect_returns(s, &mut info.returns, 0, &mut info.generator);
                    }
                }
            }
            FnRef::Arrow(a) => match &a.body {
                ArrowFunctionBody::FunctionBody(b) => {
                    for s in &b.statements {
                        collect_returns(s, &mut info.returns, 0, &mut info.generator);
                    }
                }
                other => info.returns.extend(other.as_expression()),
            },
        }
        let info = Rc::new(info);
        self.fns.insert(key, info.clone());
        info
    }

    /// Evaluates a call of the file's function `f` with `args`: the join of its
    /// returns.
    pub(super) fn inline(&mut self, f: FnRef<'a>, args: &Args<'a>, span: Span) -> V<'a> {
        let key = f.span().start;
        if self.inlining.len() >= MAX_INLINE || self.inlining.contains(&key) || !self.tick() {
            return self.unk(Tag::Unknown, "", span);
        }
        let info = self.fn_info(f);
        if info.generator {
            return self.unk(Tag::Unknown, "", span);
        }
        if info.returns.is_empty() {
            return V::Void;
        }
        let params = match f {
            FnRef::Function(func) => &func.params,
            FnRef::Arrow(a) => &a.params,
        };
        // Bind the parameters. Defaults are evaluated in the callee, where earlier
        // parameters are already bound. Whether the callee reads its own frame says
        // nothing about the caller's (the arguments were evaluated there).
        let outer_used = self.frame_used;
        // `arguments[i] = x` in the body writes the parameter (sloppy code).
        let aliased = self.facts().uses_arguments(key);
        self.inlining.push(key);
        self.frames.push(Vec::new());
        for (i, p) in params.items.iter().enumerate() {
            let mut v = args.get(i);
            if let Some(init) = &p.initializer {
                v = self.with_default(v, init);
            }
            if aliased {
                v = join([v, self.unk(Tag::Unknown, "", p.span)]);
            }
            let mut bound = Vec::new();
            // An argument is an alias of the caller's value.
            self.destructure(&p.pattern, v.shared(), &mut bound);
            if let Some(frame) = self.frames.last_mut() {
                frame.extend(bound);
            }
        }
        if let Some(r) = &params.rest {
            let mut bound = Vec::new();
            self.destructure(&r.rest.argument, args.tail(params.items.len()), &mut bound);
            if let Some(frame) = self.frames.last_mut() {
                frame.extend(bound);
            }
        }
        let mut out = Vec::with_capacity(info.returns.len());
        for e in &info.returns {
            out.push(self.eval(e));
        }
        self.frames.pop();
        self.inlining.pop();
        self.frame_used = outer_used;
        join(out)
    }

    /// Applies a path builtin of the token evaluator to every combination of
    /// argument leaves.
    pub(super) fn apply(&mut self, f: Func, args: &Args<'a>, span: Span) -> V<'a> {
        let mut vals = args.vals.clone();
        if let Some(r) = &args.rest {
            // `path.join(dir, ...parts)`: some unknown number of further segments.
            let mut tails = Vec::new();
            for l in r.leaves() {
                tails.push(match l {
                    V::Js(j @ JsVal::Unknown(_)) => self.derived(j, span),
                    V::Void => V::Void,
                    _ => self.unk(Tag::Listing, "", span),
                });
            }
            vals.push(join(tails));
        }
        let mut lists: Vec<Vec<Option<JsVal>>> = Vec::with_capacity(vals.len());
        let mut total = 1usize;
        for v in &vals {
            let l: Vec<Option<JsVal>> = v
                .leaves()
                .iter()
                .map(|l| match l {
                    // `path.join(undefined)`, `path.join(3)` throw.
                    V::Void => None,
                    l if l.is_scalar() => None,
                    l => self.operand(l, true, span).map(|j| match (l, j) {
                        (V::Js(orig), _) => orig.clone(),
                        (_, j) => j,
                    }),
                })
                .collect();
            total = total.saturating_mul(l.len().max(1));
            lists.push(l);
        }
        if total > MAX_COMBOS {
            return self.unk(Tag::Unknown, "", span);
        }
        let mut out = Vec::with_capacity(total);
        let mut idx = vec![0usize; lists.len()];
        'combos: loop {
            let mut combo: Vec<JsVal> = Vec::with_capacity(lists.len());
            let mut void = false;
            for (l, &i) in lists.iter().zip(&idx) {
                match l.get(i) {
                    Some(Some(j)) => combo.push(j.clone()),
                    // `path.join(undefined)` throws: no value.
                    Some(None) | None => void = true,
                }
            }
            // `path.basename(p, undefined)` is `path.basename(p)`.
            if void
                && f == Func::PathBasename
                && combo.len() == 1
                && matches!(lists.first().zip(idx.first()), Some((l, &i)) if matches!(l.get(i), Some(Some(_))))
            {
                void = false;
            }
            if !void {
                out.push(match self.sym_apply(f, &combo, span) {
                    Some(j) => V::Js(j),
                    None => match evaluator::apply(f, &combo, &self.scope()) {
                        Some(j) => V::Js(j),
                        None => match combo.iter().find(|j| matches!(j, JsVal::Unknown(_))) {
                            Some(j) => self.derived(j, span),
                            None => self.unk(Tag::Unknown, "", span),
                        },
                    },
                });
            }
            let mut k = 0;
            loop {
                if k == idx.len() {
                    break 'combos;
                }
                idx[k] += 1;
                if idx[k] < lists[k].len() {
                    break;
                }
                idx[k] = 0;
                k += 1;
            }
        }
        join(out)
    }

    /// `createRequire(filename)`: a `require` that resolves as a module at
    /// `filename` would. For the file itself, or any file in its directory,
    /// that is the file's own `require`; for a known path elsewhere a
    /// [`V::Loader`]; otherwise unknown, so that nothing loaded through it
    /// is resolved from the wrong place.
    fn create_require(&mut self, args: &Args<'a>, span: Span) -> V<'a> {
        let mut out = Vec::new();
        for l in args.get(0).into_leaves() {
            out.push(match &l {
                V::Js(JsVal::FileUrl | JsVal::File) => V::Js(JsVal::Obj(Obj::Require)),
                V::Js(j) => match evaluator::start_of_file(j) {
                    Some(evaluator::Start::Dir(d)) if d.is_empty() => {
                        V::Js(JsVal::Obj(Obj::Require))
                    }
                    Some(start) => V::Loader(Rc::new(start)),
                    None => match j {
                        JsVal::Unknown(_) => self.derived(j, span),
                        _ => self.unk(Tag::Call, "", span),
                    },
                },
                // `createRequire(undefined)` throws.
                V::Void => V::Void,
                _ => self.unk(Tag::Call, "", span),
            });
        }
        join(out)
    }

    /// `require.resolve(request, options)`: with `{ paths: [p, ...] }` the
    /// search starts at each `p` in turn instead of at the file (one
    /// alternative per entry: the first that finds the module wins at
    /// runtime). Node reads no other option. Options that are
    /// not understood (not an object literal's value, an unknown spread,
    /// entries that are not known paths) make the result unknown.
    fn resolve_with(&mut self, args: &Args<'a>, span: Span) -> V<'a> {
        if args.has_rest() {
            return self.unk(Tag::Call, "", span);
        }
        let request = args.get(0);
        let mut out = Vec::new();
        for opts in args.get(1).into_leaves() {
            match opts {
                // No options: from the file.
                V::Void => {
                    out.push(self.apply(Func::Resolve, &Args::of(vec![request.clone()]), span))
                }
                // Node reads only `options.paths`.
                V::Object(o) if !o.map => {
                    let paths = self.obj_get(&o, "paths");
                    for p in paths.into_leaves() {
                        match p {
                            // `paths: undefined`: from the file.
                            V::Void => out.push(self.apply(
                                Func::Resolve,
                                &Args::of(vec![request.clone()]),
                                span,
                            )),
                            V::Array(_) => {
                                let entries = self.elements(p, span);
                                for e in entries.into_leaves() {
                                    out.push(match &e {
                                        V::Js(j) => match evaluator::start_at(j) {
                                            Some(start) => self.resolve_at(&request, &start, span),
                                            None => match j {
                                                JsVal::Unknown(_) => self.derived(j, span),
                                                _ => self.unk(Tag::Call, "", span),
                                            },
                                        },
                                        // An entry that is no string throws.
                                        V::Void => V::Void,
                                        _ => self.unk(Tag::Call, "", span),
                                    });
                                }
                            }
                            _ => out.push(self.unk(Tag::Call, "", span)),
                        }
                    }
                }
                _ => out.push(self.unk(Tag::Call, "", span)),
            }
        }
        join(out)
    }

    /// `require.resolve(request)` searching from `start`
    /// ([`evaluator::resolve_from`]), for every alternative of `request`.
    fn resolve_at(&mut self, request: &V<'a>, start: &evaluator::Start, span: Span) -> V<'a> {
        let mut out = Vec::new();
        for r in request.leaves() {
            out.push(match r {
                r if same_everywhere(r) => r.clone(),
                V::Js(j) => match evaluator::resolve_from(j, start) {
                    Some(v) => V::Js(v),
                    None => match j {
                        JsVal::Unknown(_) => self.derived(j, span),
                        _ => self.unk(Tag::Call, "", span),
                    },
                },
                // `resolve(undefined)` throws.
                V::Void => V::Void,
                _ => self.unk(Tag::Call, "", span),
            });
        }
        join(out)
    }

    /// The path a call of a `require` function resolves (a site of the
    /// recognizer whose callee is not the file's own `require`:
    /// [`super::Ask::Load`]): what the call's first argument names, from
    /// where the function searches. `req.resolve(x)` is its own value.
    ///
    /// A request that is exactly an environment variable (`req(process.env.X)`)
    /// has no value here whatever function is asked, so it stays its own
    /// value, and the place of the function is said next to it
    /// ([`Engine::take_from`]): the file, a known directory elsewhere, or
    /// unknown. The environment layer resolves the values the repository
    /// assigns; only a relative one depends on the place, and with an unknown
    /// place it resolves nothing. Any other request asked of a function for
    /// another place is resolved there or is unknown, as the place decides.
    pub(super) fn site_load(&mut self, c: &'a CallExpression<'a>) -> Option<V<'a>> {
        self.begin();
        self.asked_from = None;
        let budget = self.site_steps;
        let v = match c.callee.get_inner_expression() {
            Expression::StaticMemberExpression(m) if m.property.name == "resolve" => {
                // An absolute path or a builtin module is its own answer, whatever
                // function resolves it.
                let fixed = match c.arguments.as_slice() {
                    [_] => {
                        let a = self.args(&c.arguments);
                        let request = a.get(0);
                        (!a.has_rest() && request.leaves().iter().all(same_everywhere))
                            .then_some(request)
                    }
                    _ => None,
                };
                match fixed {
                    Some(request) => request,
                    None => self.call(c),
                }
            }
            callee => {
                let f = self.eval(callee);
                let a = self.args(&c.arguments);
                // A builtin module or an absolute path is the same for every `require`
                // function.
                let (fixed, other): (Vec<V<'a>>, Vec<V<'a>>) = a
                    .get(0)
                    .into_leaves()
                    .into_iter()
                    .partition(same_everywhere);
                let mut out = fixed;
                let request = join(other.iter().cloned());
                // The alternatives that are exactly an environment variable, and the rest.
                let (named, rest): (Vec<V<'a>>, Vec<V<'a>>) = other.iter().cloned().partition(
                    |l| matches!(l, V::Js(JsVal::Unknown(r)) if super::env::exact_env(r).is_some()),
                );
                let rest_request = join(rest.iter().cloned());
                let mut from: Option<EnvFrom> = None;
                for l in f.into_leaves() {
                    if other.is_empty() {
                        break;
                    }
                    let place = match l {
                        // The file's own `require`: the request, from the file.
                        V::Js(JsVal::Obj(Obj::Require)) => {
                            out.push(request.clone());
                            EnvFrom::File
                        }
                        V::Loader(start) => {
                            if !rest.is_empty() {
                                out.push(self.resolve_at(&rest_request, &start, c.span));
                            }
                            out.extend(named.iter().cloned());
                            match &*start {
                                evaluator::Start::Dir(d) => EnvFrom::Dir(d.to_string()),
                                evaluator::Start::Next(..) => EnvFrom::Unknown,
                            }
                        }
                        // Not a function: the call throws.
                        V::Void => {
                            out.push(V::Void);
                            continue;
                        }
                        l => {
                            if !rest.is_empty() {
                                out.push(match &l {
                                    V::Js(j @ JsVal::Unknown(_)) => self.derived(j, c.span),
                                    _ => self.unk(Tag::Call, "", c.span),
                                });
                            }
                            out.extend(named.iter().cloned());
                            EnvFrom::Unknown
                        }
                    };
                    if !named.is_empty() {
                        // Functions for different places: no one place can be said.
                        from = Some(match from {
                            Some(f) if f != place => EnvFrom::Unknown,
                            _ => place,
                        });
                    }
                }
                self.asked_from = from;
                join(out)
            }
        };
        self.end(budget);
        Some(v)
    }

    fn new_expr(&mut self, n: &'a NewExpression<'a>) -> V<'a> {
        let callee = self.eval(&n.callee);
        match callee {
            V::Js(JsVal::Obj(Obj::UrlCtor)) => {
                let args = self.args(&n.arguments);
                let (input, base) = (args.get(0), args.vals.get(1).cloned());
                let mut out = Vec::new();
                let bases: Vec<Option<V<'a>>> = match &base {
                    Some(b) => b.leaves().iter().cloned().map(Some).collect(),
                    None => vec![None],
                };
                if input.leaves().len() * bases.len() > MAX_COMBOS {
                    return self.unk(Tag::Unknown, "", n.span);
                }
                for i in input.leaves() {
                    for b in &bases {
                        let (Some(i), b) = (
                            self.operand(i, true, n.span).map(|j| match i {
                                V::Js(o) => o.clone(),
                                _ => j,
                            }),
                            match b {
                                Some(V::Void) | None => None,
                                Some(V::Js(j)) => Some(j.clone()),
                                Some(other) => self.operand(other, true, n.span),
                            },
                        ) else {
                            continue;
                        };
                        let known =
                            matches!(&i, JsVal::Str(Value::Str(s)) if !s.starts_with("file:"));
                        // An unknown input against a file URL: taken as a relative
                        // reference (like an unknown segment of `path.resolve`), so the
                        // result is something below the base's directory.
                        if let (
                            JsVal::Unknown(r),
                            Some(base @ (JsVal::FileUrl | JsVal::Url(..) | JsVal::Resolved(..))),
                        ) = (&i, &b)
                            && let Some(dir) = evaluator::url_ctor(
                                &[JsVal::Str(Value::str("./")), base.clone()],
                                &self.scope(),
                            )
                        {
                            let rest = Value::Unknown { reason: r.clone() };
                            match dir {
                                JsVal::Url(anchor, dir) => {
                                    out.push(V::Js(JsVal::Url(anchor, dir.concat(rest))));
                                    continue;
                                }
                                JsVal::Resolved(m, dir) => {
                                    out.push(V::Js(JsVal::Resolved(m, dir.concat(rest))));
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        let mut a = vec![i];
                        a.extend(b);
                        out.push(match evaluator::url_ctor(&a, &self.scope()) {
                            Some(j) => V::Js(j),
                            // A known input that is no file path against a base that is a
                            // file URL (or no base): another scheme (`https:`, `data:`).
                            None if known
                                && a.get(1).is_none_or(|b| {
                                    matches!(
                                        b,
                                        JsVal::FileUrl | JsVal::Url(..) | JsVal::Resolved(..)
                                    )
                                }) =>
                            {
                                V::Void
                            }
                            None => match a.iter().find(|j| matches!(j, JsVal::Unknown(_))) {
                                Some(j) => self.derived(j, n.span),
                                None => self.unk(Tag::Unknown, "", n.span),
                            },
                        });
                    }
                }
                join(out)
            }
            V::Builtin(Builtin::SetCtor) | V::Builtin(Builtin::Array) => {
                match n.arguments.first().and_then(|a| a.as_expression()) {
                    Some(e) if matches!(callee, V::Builtin(Builtin::SetCtor)) => {
                        let v = self.eval(e);
                        let els = self.elements(v, e.span());
                        V::array(Vec::new(), Some(els))
                    }
                    Some(_) => self.unk(Tag::Unknown, "", n.span),
                    None => V::array(Vec::new(), None),
                }
            }
            V::Builtin(Builtin::MapCtor) => {
                match n.arguments.first().and_then(|a| a.as_expression()) {
                    Some(e) => {
                        let v = self.eval(e);
                        self.entries_object(v, e.span(), true)
                    }
                    None => V::Object(Rc::new(ObjV {
                        props: Vec::new(),
                        open: None,
                        exact: Vec::new(),
                        late: None,
                        trust: Trust::Fresh,
                        map: true,
                    })),
                }
            }
            V::Builtin(Builtin::Tmp) => V::Builtin(Builtin::Tmp),
            _ => self.unk(Tag::Unknown, "", n.span),
        }
    }

    /// An object from `[key, value]` pairs (`new Map(pairs)`,
    /// `Object.fromEntries`).
    fn entries_object(&mut self, pairs: V<'a>, span: Span, map: bool) -> V<'a> {
        let mut obj = ObjV {
            props: Vec::new(),
            open: None,
            exact: Vec::new(),
            late: None,
            trust: Trust::Fresh,
            map,
        };
        let rows = self.elements(pairs, span);
        for row in rows.into_leaves() {
            match &row {
                V::Array(a) if a.rest.is_none() && a.elems.len() >= 2 => match &a.elems[0] {
                    V::Js(JsVal::Str(Value::Str(k))) => obj.set(k, a.elems[1].clone(), true),
                    _ => obj.widen(a.elems[1].clone()),
                },
                V::Void => {}
                V::Js(j @ JsVal::Unknown(_)) => obj.widen(self.derived(j, span)),
                _ => obj.widen(self.unk(Tag::Unknown, "", span)),
            }
        }
        V::Object(Rc::new(obj))
    }

    // ---- methods ----

    fn method(&mut self, recv: V<'a>, name: &str, args: &'a [Argument<'a>], span: Span) -> V<'a> {
        // Promises are their settled values: `p.then(f)` is `f(p)`, `p.catch(f)` is `p`
        // or what the handler returns, `p.finally(f)` is `p`.
        match name {
            "then" | "catch" => {
                let Some(cb) = args.first().and_then(|a| a.as_expression()) else {
                    return recv;
                };
                let f = self.eval(cb);
                let arg = if name == "then" {
                    recv.clone()
                } else {
                    self.unk(Tag::Unknown, "", span)
                };
                let a = Args {
                    vals: vec![arg],
                    rest: None,
                };
                let mut r = self.call_with(f, &a, span);
                // `p.then(onFulfilled, onRejected)`: also what the second one returns.
                if name == "then"
                    && let Some(cb) = args.get(1).and_then(|a| a.as_expression())
                {
                    let f = self.eval(cb);
                    let a = Args {
                        vals: vec![self.unk(Tag::Unknown, "", span)],
                        rest: None,
                    };
                    let rejected = self.call_with(f, &a, span);
                    r = join([r, rejected]);
                }
                return if name == "then" { r } else { join([recv, r]) };
            }
            "finally" => return recv,
            _ => {}
        }
        let leaves = recv.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            if !self.tick() {
                out.push(self.unk(Tag::Unknown, "", span));
                break;
            }
            out.push(match l {
                V::Js(JsVal::Obj(o)) => match evaluator::obj_member(o, name, &self.scope()) {
                    // `require.resolve(request, options)`.
                    Some(JsVal::Func(Func::Resolve)) if args.len() > 1 => {
                        let a = self.args(args);
                        self.resolve_with(&a, span)
                    }
                    // `import.meta.resolve(request, parent)`: a parent is not followed.
                    Some(JsVal::Func(Func::MetaResolve)) if args.len() > 1 => {
                        let a = self.args(args);
                        match a.get(1) {
                            V::Void if !a.has_rest() => {
                                self.apply(Func::MetaResolve, &Args::of(vec![a.get(0)]), span)
                            }
                            _ => self.unk(Tag::Call, "", span),
                        }
                    }
                    Some(JsVal::Func(f)) => {
                        let a = self.args(args);
                        self.apply(f, &a, span)
                    }
                    Some(other) => self.call_value(V::Js(other), args, span),
                    None => match (o, name) {
                        (Obj::Path, "isAbsolute") => V::scalar(),
                        (Obj::Path, "toNamespacedPath") => self.args(args).get(0),
                        (Obj::Require, _) | (Obj::ImportMeta, _) => self.unk(Tag::Call, "", span),
                        _ => self.unk(Tag::Unknown, "", span),
                    },
                },
                V::Js(j) => self.string_method(j, name, args, span),
                V::Void => V::Void,
                V::Array(a) => self.array_method(&a, name, args, span),
                V::Object(o) => self.object_method(&o, name, args, span),
                V::Builtin(b) => self.builtin(b, name, args, span),
                V::Func(f) if name == "call" => {
                    let a = self.args(args);
                    let a = Args {
                        vals: a.vals.into_iter().skip(1).collect(),
                        rest: a.rest,
                    };
                    self.inline(f, &a, span)
                }
                // `req.resolve(request)` of a `require` made by `createRequire(elsewhere)`.
                V::Loader(start) if name == "resolve" => {
                    let a = self.args(args);
                    match a.get(1) {
                        V::Void if !a.has_rest() => self.resolve_at(&a.get(0), &start, span),
                        // Options on top of another start: not followed.
                        _ => self.unk(Tag::Call, "", span),
                    }
                }
                V::Func(_) | V::Method(..) | V::Loader(_) => self.unk(Tag::Call, "", span),
                V::Import(i) => {
                    let callee = if &*i.name == "*" {
                        name.to_owned()
                    } else {
                        format!("{}.{name}", i.name)
                    };
                    let member = V::Import(Rc::new(ImportRef {
                        module: i.module.clone(),
                        name: callee.as_str().into(),
                    }));
                    match self.sym_call(&member, args, span) {
                        Some(v) => v,
                        None => {
                            let a = self.args(args);
                            self.import_call(&i.module, &callee, &a, span)
                        }
                    }
                }
                V::Alts(_) => V::Void,
            });
        }
        join(out)
    }

    fn string_method(
        &mut self,
        j: JsVal,
        name: &str,
        args: &'a [Argument<'a>],
        span: Span,
    ) -> V<'a> {
        let known = match &j {
            JsVal::Str(Value::Str(s)) => Some(s.clone()),
            _ => None,
        };
        if V::Js(j.clone()).is_scalar() {
            // `n.toString()`, `n.toFixed(2)`, `flag.valueOf()`: some text.
            return self.unk(Tag::Unknown, "", span);
        }
        match name {
            "toString" | "valueOf" | "normalize" | "toWellFormed" => V::Js(j),
            "trim" | "trimEnd" | "trimStart" => match known {
                Some(s) => V::str(match name {
                    "trim" => s.trim_matches(js_space),
                    "trimEnd" => s.trim_end_matches(js_space),
                    _ => s.trim_start_matches(js_space),
                }),
                None => V::Js(j),
            },
            "concat" => {
                let a = self.args(args);
                let mut acc = V::Js(j);
                for v in a.vals {
                    acc = self.concat(acc, v, span);
                }
                match a.rest {
                    Some(_) => self.unk(Tag::Unknown, "", span),
                    None => acc,
                }
            }
            "replace" | "replaceAll" => {
                let a = self.args(args);
                match (known, a.get(0), a.get(1)) {
                    (
                        Some(s),
                        V::Js(JsVal::Str(Value::Str(from))),
                        V::Js(JsVal::Str(Value::Str(to))),
                        // `$&`, `$1`, `$$` in the replacement are patterns.
                    ) if !from.is_empty() && !to.contains('$') => V::str(if name == "replace" {
                        s.replacen(&from, &to, 1)
                    } else {
                        s.replace(&from, &to)
                    }),
                    _ => self.derived(&j, span),
                }
            }
            "toLowerCase" | "toUpperCase" => match known {
                Some(s) if name == "toLowerCase" => V::str(s.to_lowercase()),
                Some(s) => V::str(s.to_uppercase()),
                None => self.derived(&j, span),
            },
            "split" | "match" | "matchAll" => V::array(Vec::new(), Some(self.derived(&j, span))),
            // Booleans and numbers, for a receiver known to be a string. (An unknown
            // receiver can be any object with a method of that name.)
            "startsWith" | "endsWith" | "includes" | "indexOf" | "lastIndexOf"
            | "localeCompare" | "search" | "charCodeAt" | "codePointAt"
                if !matches!(j, JsVal::Unknown(_)) =>
            {
                V::scalar()
            }
            // `<unknown>.join(<anchored path>, ...)`: taken as `path.join`, as the token
            // evaluator does (bundler namespace objects), with a record.
            "join" | "resolve" if matches!(j, JsVal::Unknown(_)) => {
                let a = self.args(args);
                let anchored = matches!(a.vals.first(), Some(V::Js(JsVal::Path(..))));
                if anchored {
                    let f = if name == "join" {
                        Func::PathJoin
                    } else {
                        Func::PathResolve
                    };
                    if self.guess.is_none() {
                        let callee = origin::strip(&self.text(span));
                        self.guess = Some(format!("`{callee}` assumed to be `path.{name}`").into());
                    }
                    self.apply(f, &a, span)
                } else {
                    self.derived_call(&j, span)
                }
            }
            _ => match &j {
                JsVal::Unknown(_) => self.derived_call(&j, span),
                _ => self.derived(&j, span),
            },
        }
    }

    fn array_method(
        &mut self,
        a: &Rc<ArrV<'a>>,
        name: &str,
        args: &'a [Argument<'a>],
        span: Span,
    ) -> V<'a> {
        let this = V::Array(a.clone());
        match name {
            "join" => {
                let sep = match args.first().and_then(|x| x.as_expression()) {
                    None => Some(",".to_owned()),
                    Some(e) => match self.eval(e) {
                        V::Js(JsVal::Str(Value::Str(s))) => Some(s),
                        _ => None,
                    },
                };
                match sep {
                    Some(sep) if a.rest.is_none() && a.trust != Trust::Shared => {
                        let mut acc: Option<V<'a>> = None;
                        for e in &a.elems {
                            // `join` writes `undefined` and `null` as the empty string.
                            let e = &join(e.leaves().iter().map(|l| match l {
                                V::Void => V::str(""),
                                l => l.clone(),
                            }));
                            acc = Some(match acc {
                                None => self.concat(e.clone(), V::str(""), span),
                                Some(x) => {
                                    let x = self.concat(x, V::str(sep.clone()), span);
                                    self.concat(x, e.clone(), span)
                                }
                            });
                        }
                        acc.unwrap_or_else(|| V::str(""))
                    }
                    _ => {
                        let els = self.elements(this, span);
                        // Starts with some element.
                        let tail = V::unknown(Tag::Listing, "", &self.text(span));
                        let joined = self.concat(els.clone(), tail, span);
                        join([els, joined])
                    }
                }
            }
            "map" | "flatMap" => {
                let els = self.elements(this, span);
                let mapped = match args.first().and_then(|x| x.as_expression()) {
                    Some(cb) => {
                        let f = self.eval(cb);
                        let a = Args {
                            vals: vec![els, V::unknown(Tag::Listing, "", "index")],
                            rest: None,
                        };
                        self.call_with(f, &a, span)
                    }
                    None => self.unk(Tag::Unknown, "", span),
                };
                let mapped = if name == "flatMap" {
                    let mut out = Vec::new();
                    for l in mapped.into_leaves() {
                        match l {
                            V::Array(_) => out.push(self.elements(l, span)),
                            other => out.push(other),
                        }
                    }
                    join(out)
                } else {
                    mapped
                };
                V::array(Vec::new(), Some(mapped))
            }
            // A copy with new elements: `xs.with(i, x)`, `xs.toSpliced(i, n, ...items)`.
            "with" | "toSpliced" => {
                let a = self.args(args);
                let skip = if name == "with" { 1 } else { 2 };
                let mut els = vec![self.elements(this, span)];
                els.extend(a.vals.into_iter().skip(skip));
                els.extend(a.rest);
                V::array(Vec::new(), Some(join(els)))
            }
            "filter" | "slice" | "sort" | "reverse" | "toSorted" | "toReversed" | "flat"
            | "values" | "splice" => {
                let els = self.elements(this, span);
                let els = if name == "flat" {
                    let mut out = Vec::new();
                    for l in els.into_leaves() {
                        match l {
                            V::Array(_) => out.push(self.elements(l, span)),
                            other => out.push(other),
                        }
                    }
                    join(out)
                } else {
                    els
                };
                V::array(Vec::new(), Some(els))
            }
            "concat" => {
                let mut els = vec![self.elements(this, span)];
                let a = self.args(args);
                for v in a.vals.into_iter().chain(a.rest) {
                    for l in v.into_leaves() {
                        match l {
                            V::Array(_) => els.push(self.elements(l, span)),
                            other => els.push(other),
                        }
                    }
                }
                V::array(Vec::new(), Some(join(els)))
            }
            "find" | "findLast" | "at" | "pop" | "shift" => {
                let els = self.elements(this, span);
                join([els, V::Void])
            }
            "entries" => {
                let els = self.elements(this, span);
                V::array(
                    Vec::new(),
                    Some(V::array(
                        vec![V::unknown(Tag::Listing, "", "index"), els],
                        None,
                    )),
                )
            }
            "keys" => V::array(Vec::new(), Some(V::unknown(Tag::Listing, "", "index"))),
            "forEach" | "clear" => V::Void,
            "includes" | "indexOf" | "lastIndexOf" | "some" | "every" | "findIndex"
            | "findLastIndex" | "push" | "unshift" | "has" | "delete" => V::scalar(),
            // Chainable: the set or array itself, with the new element.
            "add" | "fill" => {
                let v = self.args(args).get(0);
                let mut arr = (**a).clone();
                arr.add(v);
                V::Array(Rc::new(arr))
            }
            "toString" => self.unk(Tag::Unknown, "", span),
            _ => self.unk(Tag::Unknown, "", span),
        }
    }

    /// Calls `f` with already evaluated arguments.
    fn call_with(&mut self, f: V<'a>, args: &Args<'a>, span: Span) -> V<'a> {
        let leaves = f.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match l {
                V::Func(fr) => self.inline(fr, args, span),
                V::Js(JsVal::Func(func)) => {
                    // `xs.map(path.basename)`: only the element is meaningful.
                    let a = Args {
                        vals: args.vals.iter().take(1).cloned().collect(),
                        rest: None,
                    };
                    self.apply(func, &a, span)
                }
                V::Js(JsVal::Obj(Obj::StringCtor)) => self.stringified(args.get(0), span),
                V::Import(i) => self.unk(Tag::Call, &format!("{}#{}", i.module, i.name), span),
                V::Js(j @ JsVal::Unknown(_)) => self.derived_call(&j, span),
                V::Void => V::Void,
                _ => self.unk(Tag::Call, "", span),
            });
        }
        join(out)
    }

    /// `String(v)`, `v.toString()`: the text of a number or boolean is a string
    /// like any other (unknown); other values are themselves.
    fn stringified(&mut self, v: V<'a>, span: Span) -> V<'a> {
        if !v.leaves().iter().any(V::is_scalar) {
            return v;
        }
        let leaves = v.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(if l.is_scalar() {
                self.unk(Tag::Unknown, "", span)
            } else {
                l
            });
        }
        join(out)
    }

    fn object_method(
        &mut self,
        o: &Rc<ObjV<'a>>,
        name: &str,
        args: &'a [Argument<'a>],
        span: Span,
    ) -> V<'a> {
        if o.get(name).is_some() {
            let f = self.obj_get(o, name);
            return self.call_value(f, args, span);
        }
        let this = V::Object(o.clone());
        match name {
            // `Map` methods.
            "get" => {
                let a = self.args(args);
                self.member_by(this, a.get(0), span)
            }
            "values" => {
                let v = self.any_member(this, span);
                V::array(Vec::new(), Some(v))
            }
            "keys" => {
                let k = self.keys(this, span);
                V::array(Vec::new(), Some(k))
            }
            "entries" => self.entries(this, span),
            "clear" | "forEach" => V::Void,
            "has" | "delete" | "hasOwnProperty" => V::scalar(),
            // Chainable: the map itself, with the new entry.
            "set" => {
                let a = self.args(args);
                let mut obj = (**o).clone();
                match a.get(0) {
                    V::Js(JsVal::Str(Value::Str(k))) => obj.set(&k, a.get(1), true),
                    _ => obj.widen(a.get(1)),
                }
                V::Object(Rc::new(obj))
            }
            _ if o.open.is_some() => {
                let f = self.obj_get(o, name);
                self.call_value(f, args, span)
            }
            _ => self.unk(Tag::Unknown, "", span),
        }
    }

    /// `[key, value]` pairs of an object.
    fn entries(&mut self, o: V<'a>, span: Span) -> V<'a> {
        let mut rows = Vec::new();
        for l in o.into_leaves() {
            match l {
                V::Object(obj) => {
                    let shared = obj.trust == Trust::Shared;
                    for (k, v) in &obj.props {
                        let v = if shared {
                            self.obj_get(&obj, k)
                        } else {
                            v.clone().shared()
                        };
                        rows.push(V::array(vec![V::str(&**k), v], None));
                    }
                    if obj.open.is_some() || any_written(&obj, &self.facts()) {
                        let k = self.unk(Tag::Unknown, "", span);
                        let v = self.any_member(V::Object(obj.clone()), span);
                        rows.push(V::array(vec![k, v], None));
                    }
                }
                V::Void => {}
                V::Js(j @ JsVal::Unknown(_)) => {
                    let d = self.derived(&j, span);
                    rows.push(V::array(vec![d.clone(), d], None));
                }
                _ => rows.push(self.unk(Tag::Unknown, "", span)),
            }
        }
        V::array(Vec::new(), Some(join(rows)))
    }

    fn builtin(&mut self, b: Builtin, name: &str, args: &'a [Argument<'a>], span: Span) -> V<'a> {
        match (b, name) {
            (Builtin::Tmp, _) => V::Builtin(Builtin::Tmp),
            (Builtin::Os, "tmpdir") => V::Js(JsVal::Path(Anchor::Tmp, Value::str(""))),
            (Builtin::Os, _) => self.unk(Tag::Unknown, "", span),
            (Builtin::Fs | Builtin::FsPromises, "mkdtempSync" | "mkdtemp") => {
                let prefix = self.args(args).get(0);
                let suffix = V::unknown(Tag::Listing, "", "mkdtemp suffix");
                self.concat(prefix, suffix, span)
            }
            (Builtin::Fs | Builtin::FsPromises, "realpathSync" | "realpath") => {
                self.args(args).get(0)
            }
            (Builtin::Fs | Builtin::FsPromises, "readdirSync" | "readdir") => {
                let dir = self.args(args).get(0);
                let entry = V::unknown(Tag::Listing, "", &self.text(span));
                let dirent = args.get(1).is_some_and(|o| match o.as_expression() {
                    Some(Expression::ObjectExpression(o)) => o.properties.iter().any(|p| {
                        matches!(p, ObjectPropertyKind::ObjectProperty(p)
                            if matches!(&p.key, PropertyKey::StaticIdentifier(k) if k.name == "withFileTypes")
                                && matches!(&p.value, Expression::BooleanLiteral(b) if b.value))
                    }),
                    _ => false,
                });
                let el = if dirent {
                    V::object(
                        vec![
                            ("name".into(), entry),
                            ("path".into(), dir.clone()),
                            ("parentPath".into(), dir),
                        ],
                        None,
                    )
                } else {
                    entry
                };
                V::array(Vec::new(), Some(el))
            }
            (
                Builtin::Fs | Builtin::FsPromises,
                "readFileSync" | "readFile" | "readJsonSync" | "readJson" | "readJSON"
                | "readJSONSync",
            ) => self.unk(Tag::Data, "", span),
            // A boolean, a file descriptor, a file handle or a `Stats` object: not a path,
            // but what is read from it is some number or text (`stat.size`, `fh.readFile()`).
            (
                Builtin::Fs | Builtin::FsPromises,
                "existsSync" | "exists" | "statSync" | "stat" | "lstatSync" | "lstat" | "openSync"
                | "open" | "pathExists" | "pathExistsSync",
            ) => V::scalar(),
            (
                Builtin::Fs | Builtin::FsPromises,
                "accessSync" | "access" | "writeFileSync" | "writeFile" | "appendFileSync"
                | "appendFile" | "rmSync" | "rm" | "rmdirSync" | "rmdir" | "unlinkSync" | "unlink"
                | "copyFileSync" | "copyFile" | "cpSync" | "cp" | "renameSync" | "rename"
                | "chmodSync" | "chmod" | "closeSync" | "close" | "outputFile" | "outputFileSync"
                | "remove" | "removeSync" | "emptyDir" | "emptyDirSync" | "symlinkSync" | "symlink",
            ) => V::Void,
            (Builtin::Fs | Builtin::FsPromises, _) => self.unk(Tag::Call, "", span),
            (Builtin::Module, "createRequire") => {
                let a = self.args(args);
                self.create_require(&a, span)
            }
            (Builtin::Module, _) => self.unk(Tag::Call, "", span),
            (Builtin::Object, "keys") => {
                let o = self.args(args).get(0);
                let k = self.keys(o, span);
                V::array(Vec::new(), Some(k))
            }
            (Builtin::Object, "values") => {
                let o = self.args(args).get(0);
                let v = self.any_member(o, span);
                V::array(Vec::new(), Some(v))
            }
            (Builtin::Object, "entries") => {
                let o = self.args(args).get(0);
                self.entries(o, span)
            }
            (Builtin::Object, "freeze" | "seal") => self.args(args).get(0),
            (Builtin::Object, "fromEntries") => {
                let p = self.args(args).get(0);
                self.entries_object(p, span, false)
            }
            (Builtin::Object, "assign") => {
                let a = self.args(args);
                let mut obj = ObjV {
                    props: Vec::new(),
                    open: None,
                    exact: Vec::new(),
                    late: None,
                    trust: Trust::Fresh,
                    map: false,
                };
                for v in a.vals.iter().chain(a.rest.iter()) {
                    let weak = v.leaves().len() > 1 || a.rest.is_some();
                    for l in v.leaves() {
                        match l {
                            V::Object(src) => {
                                for (k, _) in &src.props {
                                    let pv = self.obj_get(src, k);
                                    obj.set(k, pv, weak);
                                }
                                if let Some(op) = &src.open {
                                    obj.widen(op.clone());
                                }
                            }
                            V::Void => {}
                            V::Js(j @ JsVal::Unknown(_)) => obj.widen(self.derived(j, span)),
                            _ => obj.widen(self.unk(Tag::Unknown, "", span)),
                        }
                    }
                }
                V::Object(Rc::new(obj))
            }
            (Builtin::Object, _) => self.unk(Tag::Unknown, "", span),
            (Builtin::Array, "from") => {
                let a = self.args(args);
                let els = self.elements(a.get(0), span);
                let els = match a.vals.get(1) {
                    Some(f) => {
                        let a = Args {
                            vals: vec![els, V::unknown(Tag::Listing, "", "index")],
                            rest: None,
                        };
                        self.call_with(f.clone(), &a, span)
                    }
                    None => els,
                };
                V::array(Vec::new(), Some(els))
            }
            (Builtin::Array, "of") => {
                let a = self.args(args);
                V::array(a.vals, a.rest)
            }
            (Builtin::Array, "isArray") => V::scalar(),
            (Builtin::Array, _) => self.unk(Tag::Unknown, "", span),
            (Builtin::Json, "parse") => self.unk(Tag::Data, "", span),
            (Builtin::Json, _) => self.unk(Tag::Unknown, "", span),
            (Builtin::Promise, "all" | "allSettled" | "resolve") => self.args(args).get(0),
            (Builtin::Promise, "race" | "any") => {
                let v = self.args(args).get(0);
                self.elements(v, span)
            }
            (Builtin::Promise, "reject") => V::Void,
            (Builtin::Promise, _) => self.unk(Tag::Unknown, "", span),
            (Builtin::SetCtor | Builtin::MapCtor, _) => self.unk(Tag::Unknown, "", span),
        }
    }
}

/// Whether a request resolves alike from every `require` function, wherever it
/// searches from: a builtin module by its `node:` name, or an absolute path (a
/// path is not searched: one anchored at the file, the working directory or a
/// resolved module, or written out).
fn same_everywhere(request: &V<'_>) -> bool {
    match request {
        V::Js(JsVal::Str(Value::Str(s))) => s.starts_with("node:") || s.starts_with('/'),
        V::Js(
            JsVal::Path(..) | JsVal::Url(..) | JsVal::Resolved(..) | JsVal::File | JsVal::FileUrl,
        ) => true,
        _ => false,
    }
}

/// Whether every leaf is a non-empty known string or path (so `a || b` is `a`).
/// The digit `0` can be the number (numeric literals are kept as their text).
fn always_truthy(v: &V<'_>) -> bool {
    v.leaves().iter().all(|l| match l {
        V::Js(JsVal::Str(Value::Str(s))) => !s.is_empty() && s != "0",
        V::Js(JsVal::Path(..) | JsVal::Url(..) | JsVal::FileUrl | JsVal::File) => true,
        V::Object(_) | V::Array(_) | V::Func(_) => true,
        _ => false,
    })
}

/// What `String.prototype.trim` removes: ECMAScript white space and line
/// terminators (not Unicode `White_Space`: U+FEFF is trimmed, U+0085 is not).
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// What `a && b` evaluates to when `a` is falsy: `None` when `a` is always
/// truthy; else the empty string, `undefined` / `null` ([`V::Void`]) or a
/// number or boolean (`0`, `false`, `NaN`).
fn falsy_part<'a>(v: &V<'a>) -> Option<V<'a>> {
    let mut out = Vec::new();
    for l in v.leaves() {
        match l {
            V::Js(JsVal::Str(Value::Str(s))) if s.is_empty() => out.push(V::str("")),
            V::Js(JsVal::Str(Value::Str(s))) if s == "0" => out.push(V::scalar()),
            V::Js(JsVal::Str(Value::Str(_)))
            | V::Js(JsVal::Path(..) | JsVal::Url(..) | JsVal::FileUrl | JsVal::File)
            | V::Object(_)
            | V::Array(_)
            | V::Func(_) => {}
            V::Void => out.push(V::Void),
            _ => out.push(V::scalar()),
        }
    }
    (!out.is_empty()).then(|| join(out))
}

/// The `return` arguments of a function body (not of nested functions). Sets
/// `opaque` when the body is too deep or has too many returns to collect them
/// all.
fn collect_returns<'a>(
    s: &'a Statement<'a>,
    out: &mut Vec<&'a Expression<'a>>,
    depth: u32,
    opaque: &mut bool,
) {
    if depth > 64 || out.len() >= 64 {
        *opaque = true;
        return;
    }
    let d = depth + 1;
    match s {
        Statement::ReturnStatement(r) => out.extend(r.argument.as_ref()),
        Statement::BlockStatement(b) => b
            .body
            .iter()
            .for_each(|s| collect_returns(s, out, d, opaque)),
        Statement::IfStatement(i) => {
            collect_returns(&i.consequent, out, d, opaque);
            if let Some(a) = &i.alternate {
                collect_returns(a, out, d, opaque);
            }
        }
        Statement::ForStatement(f) => collect_returns(&f.body, out, d, opaque),
        Statement::ForInStatement(f) => collect_returns(&f.body, out, d, opaque),
        Statement::ForOfStatement(f) => collect_returns(&f.body, out, d, opaque),
        Statement::WhileStatement(w) => collect_returns(&w.body, out, d, opaque),
        Statement::DoWhileStatement(w) => collect_returns(&w.body, out, d, opaque),
        Statement::LabeledStatement(l) => collect_returns(&l.body, out, d, opaque),
        Statement::TryStatement(t) => {
            t.block
                .body
                .iter()
                .for_each(|s| collect_returns(s, out, d, opaque));
            if let Some(h) = &t.handler {
                h.body
                    .body
                    .iter()
                    .for_each(|s| collect_returns(s, out, d, opaque));
            }
            if let Some(f) = &t.finalizer {
                f.body
                    .iter()
                    .for_each(|s| collect_returns(s, out, d, opaque));
            }
        }
        Statement::SwitchStatement(sw) => {
            for c in &sw.cases {
                c.consequent
                    .iter()
                    .for_each(|s| collect_returns(s, out, d, opaque));
            }
        }
        _ => {}
    }
}
