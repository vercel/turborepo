//! Symbolic values: the engine's mode for cross-file facts ([`crate::xfile`]).
//!
//! In the normal mode a parameter that callers outside the file can bind is an
//! unknown with origin `Param`, and everything computed from it degrades to a
//! pattern or an unknown. For cross-file binding the computation itself is
//! needed: `path.join(__dirname, 'fixtures', name)` must be applied again to
//! each caller's `name`. In symbolic mode ([`Engine::symbolic`]) such a
//! parameter is a *hole*, and every operation with a hole among its operands is
//! recorded as a node of an expression DAG instead of being evaluated:
//!
//! - a property or an element of a hole ([`Sym::Field`], [`Sym::Elem`],
//!   [`Sym::AnyField`]);
//! - `+` / template literals ([`Sym::Concat`]) and the path builtins
//!   ([`Sym::Apply`]);
//! - a call of a hole or of an imported binding ([`Sym::Call`]);
//! - any other derivation ([`Sym::Opaque`]: some function of its operands).
//!
//! A node is carried through the engine as an ordinary unknown leaf whose
//! reason is the mark `@param[$<id>]` (see [`crate::extract::origin`]), so
//! containers, joins, inlining and bindings work unchanged. The normal mode
//! never creates such a payload, and the values of this mode are never lowered
//! to references.

use std::rc::Rc;

use oxc_ast::{
    AstKind,
    ast::{Argument, BindingPattern},
};
use oxc_semantic::{NodeId, SymbolId};
use oxc_span::Span;

use super::{
    engine::{Engine, Reach},
    value::{FnRef, ImportRef, V, join},
};
use crate::{
    extract::{
        evaluator::{self, Func, JsVal},
        origin::{self, Tag},
    },
    xfile::facts::REQUIRE,
};

/// Maximum nodes per file. Beyond it holes degrade to plain unknowns (which
/// keeps every record that depends on them open).
const MAX_NODES: usize = 20_000;

/// How far back a new node is looked for among the existing ones.
const RECENT: usize = 64;

/// Where `node` is among the last [`RECENT`] of `nodes` (the last place, if
/// several): the node a new one is taken to be. Only those are compared.
/// Comparing a new node with every node of the file (to use a match among the
/// last ones only, as this did) is quadratic in a file with thousands of nodes:
/// 0.3 s for one 26 KB test file with a table of routes.
fn recent<T: PartialEq>(nodes: &[T], node: &T) -> Option<usize> {
    let from = nodes.len().saturating_sub(RECENT);
    nodes[from..]
        .iter()
        .rposition(|n| n == node)
        .map(|i| from + i)
}

/// An operand of a symbolic operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    /// Another node.
    Sym(u32),
    /// A value without holes.
    Const(JsVal),
}

/// What is called.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Callee {
    /// A node (a hole, a property of one, the result of another call).
    Sym(u32),
    /// An imported binding.
    Import(Rc<ImportRef>),
}

/// A node of the expression DAG.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sym<'a> {
    /// Parameter `index` of the function starting at byte `func`, as bound by
    /// callers the file cannot see.
    Param { func: u32, index: u32 },
    /// `base.name`.
    Field(u32, Box<str>),
    /// Any element of `base`.
    Elem(u32),
    /// Any property of `base`.
    AnyField(u32),
    /// `a + b`.
    Concat(Op, Op),
    /// A path builtin applied to operands.
    Apply(Func, Vec<Op>),
    /// Some function of the nodes (a string method, an operation the engine
    /// does not model): not computable, but it has no other inputs.
    Opaque(Vec<u32>),
    /// `obj[key]` for an object of the file and a key that is a node: the
    /// properties the key selects.
    Index { obj: V<'a>, key: u32 },
    /// The result of calling `callee` with `args` (`rest`: further arguments, a
    /// spread).
    Call {
        callee: Callee,
        args: Vec<V<'a>>,
        rest: Option<V<'a>>,
    },
    /// `base` and everything reached from it by any sequence of the reads
    /// `steps`: the value of a binding that a function fills by calling
    /// itself with parts of its own argument.
    Reach { base: u32, steps: Vec<ReachStep> },
}

/// A read that [`Sym::Reach`] repeats.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReachStep {
    /// `.name`.
    Field(Box<str>),
    /// Any element.
    Elem,
    /// Any property.
    Any,
}

/// The pseudo-property that stands for "as the receiver of an array method": an
/// array is itself, a string or a plain object nothing.
pub(crate) const AS_ARRAY: &str = "\u{0}array";

/// The pseudo-property that stands for "an element, as an array method hands it
/// out" (`AS_ARRAY` followed by an element read, as one step of a
/// [`Sym::Reach`]).
pub(crate) const ARRAY_ELEMENT: &str = "\u{0}array[]";

/// The nodes of one file.
#[derive(Default)]
pub(crate) struct SymState<'a> {
    pub nodes: Vec<Sym<'a>>,
}

/// The node a reason stands for.
pub(crate) fn sym_of_reason(reason: &str) -> Option<u32> {
    let rest = reason.strip_prefix("@param[$")?;
    let end = rest.find(']')?;
    rest[..end].parse().ok()
}

/// The node an unknown leaf stands for.
pub(crate) fn sym_of(j: &JsVal) -> Option<u32> {
    match j {
        JsVal::Unknown(r) => sym_of_reason(r),
        _ => None,
    }
}

/// Every node mentioned in the unknown parts of `j` (a node embedded in a
/// pattern by an operation without a hook).
pub(crate) fn syms_in(j: &JsVal, out: &mut Vec<u32>) {
    fn reason(r: &str, out: &mut Vec<u32>) {
        let mut rest = r;
        while let Some(at) = rest.find("@param[$") {
            let tail = &rest[at + 8..];
            let Some(end) = tail.find(']') else { break };
            if let Ok(id) = tail[..end].parse::<u32>()
                && !out.contains(&id)
            {
                out.push(id);
            }
            rest = &tail[end..];
        }
    }
    match j {
        JsVal::Unknown(r) => reason(r, out),
        JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v) | JsVal::Resolved(_, v) => {
            let mut rs = Vec::new();
            evaluator::wild_reasons(v, &mut rs);
            for r in rs {
                reason(&r, out);
            }
        }
        JsVal::Alts(vs) | JsVal::Arr(vs) => vs.iter().for_each(|v| syms_in(v, out)),
        _ => {}
    }
}

impl<'a> Engine<'a, '_> {
    /// Switches the engine to symbolic mode (see the module documentation).
    pub(crate) fn symbolic(&mut self) {
        self.sym = Some(SymState::default());
    }

    pub(crate) fn sym_node(&self, id: u32) -> Option<&Sym<'a>> {
        self.sym.as_ref()?.nodes.get(id as usize)
    }

    /// A new node as a leaf, or a plain unknown when the file has too many.
    fn sym_new(&mut self, node: Sym<'a>, text: &str) -> JsVal {
        let Some(st) = self.sym.as_mut() else {
            return JsVal::Unknown(origin::mark(Tag::Unknown, "", text));
        };
        if st.nodes.len() >= MAX_NODES {
            return JsVal::Unknown(origin::mark(Tag::Unknown, "", text));
        }
        // Small files repeat the same node a lot (`dir` read at ten sites).
        let id = match recent(&st.nodes, &node) {
            Some(i) => i,
            None => {
                st.nodes.push(node);
                st.nodes.len() - 1
            }
        };
        JsVal::Unknown(origin::mark(Tag::Param, &format!("${id}"), text))
    }

    fn op(j: &JsVal) -> Result<Op, Vec<u32>> {
        if let Some(id) = sym_of(j) {
            return Ok(Op::Sym(id));
        }
        let mut inside = Vec::new();
        syms_in(j, &mut inside);
        if inside.is_empty() {
            Ok(Op::Const(j.clone()))
        } else {
            Err(inside)
        }
    }

    /// The value of a parameter bound by unseen callers: a hole in symbolic
    /// mode, else an unknown with origin `Param` and `payload`.
    pub(super) fn hole(&mut self, f: FnRef<'a>, index: usize, payload: &str, name: &str) -> V<'a> {
        if self.sym.is_none() {
            return V::unknown(Tag::Param, payload, name);
        }
        V::Js(self.sym_new(
            Sym::Param {
                func: f.span().start,
                index: index as u32,
            },
            name,
        ))
    }

    /// In symbolic mode, the plain `Param` unknowns of `v` (an unseen
    /// callback's parameter) as holes of `f`.
    pub(super) fn holes_in(&mut self, v: V<'a>, f: FnRef<'a>, index: usize) -> V<'a> {
        if self.sym.is_none() {
            return v;
        }
        let leaves = v.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match &l {
                V::Js(JsVal::Unknown(r))
                    if origin::has_tag(r, Tag::Param) && sym_of_reason(r).is_none() =>
                {
                    let text = origin::strip(r);
                    self.hole(f, index, "", &text)
                }
                _ => l,
            });
        }
        join(out)
    }

    /// A derivation of the unknown leaf `j` by `how`: a node when `j` is one.
    fn sym_step(
        &mut self,
        j: &JsVal,
        how: impl FnOnce(u32) -> Sym<'a>,
        span: Span,
    ) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let text = self.text(span);
        match Self::op(j) {
            Ok(Op::Sym(id)) => Some(V::Js(self.sym_new(how(id), &text))),
            Ok(Op::Const(_)) => None,
            Err(ids) => Some(V::Js(self.sym_new(Sym::Opaque(ids), &text))),
        }
    }

    /// `j.name` for an unknown leaf `j`.
    pub(super) fn derived_member(&mut self, j: &JsVal, name: &str, span: Span) -> V<'a> {
        match self.sym_step(j, |id| Sym::Field(id, name.into()), span) {
            Some(v) => v,
            None => self.derived(j, span),
        }
    }

    /// Any element of the unknown leaf `j`.
    pub(super) fn derived_elem(&mut self, j: &JsVal, span: Span) -> V<'a> {
        match self.sym_step(j, Sym::Elem, span) {
            Some(v) => v,
            None => self.derived(j, span),
        }
    }

    /// Any property of the unknown leaf `j`.
    pub(super) fn derived_any(&mut self, j: &JsVal, span: Span) -> V<'a> {
        match self.sym_step(j, Sym::AnyField, span) {
            Some(v) => v,
            None => self.derived(j, span),
        }
    }

    /// In symbolic mode, the value derived from `j` when `j` contains nodes.
    pub(super) fn sym_derived(&mut self, j: &JsVal, span: Span) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let mut ids = Vec::new();
        syms_in(j, &mut ids);
        if ids.is_empty() {
            return None;
        }
        let text = self.text(span);
        Some(V::Js(self.sym_new(Sym::Opaque(ids), &text)))
    }

    /// In symbolic mode, `x + y` when an operand is or contains a node.
    pub(super) fn sym_concat(&mut self, x: &JsVal, y: &JsVal, span: Span) -> Option<JsVal> {
        self.sym.as_ref()?;
        let (a, b) = (Self::op(x), Self::op(y));
        if matches!((&a, &b), (Ok(Op::Const(_)), Ok(Op::Const(_)))) {
            return None;
        }
        let text = self.text(span);
        Some(match (a, b) {
            (Ok(a), Ok(b)) => self.sym_new(Sym::Concat(a, b), &text),
            (a, b) => {
                let mut ids = Vec::new();
                for x in [a, b] {
                    match x {
                        Ok(Op::Sym(id)) => ids.push(id),
                        Ok(Op::Const(_)) => {}
                        Err(more) => ids.extend(more),
                    }
                }
                self.sym_new(Sym::Opaque(ids), &text)
            }
        })
    }

    /// In symbolic mode, `f(combo)` when an argument is or contains a node.
    pub(super) fn sym_apply(&mut self, f: Func, combo: &[JsVal], span: Span) -> Option<JsVal> {
        self.sym.as_ref()?;
        let ops: Vec<Result<Op, Vec<u32>>> = combo.iter().map(Self::op).collect();
        if ops.iter().all(|o| matches!(o, Ok(Op::Const(_)))) {
            return None;
        }
        let text = self.text(span);
        let exact = matches!(
            f,
            Func::PathJoin
                | Func::PathResolve
                | Func::PathDirname
                | Func::PathNormalize
                | Func::FileUrlToPath
        );
        if exact && ops.iter().all(Result::is_ok) {
            let ops = ops.into_iter().filter_map(Result::ok).collect();
            return Some(self.sym_new(Sym::Apply(f, ops), &text));
        }
        let mut ids = Vec::new();
        for o in ops {
            match o {
                Ok(Op::Sym(id)) => ids.push(id),
                Ok(Op::Const(_)) => {}
                Err(more) => ids.extend(more),
            }
        }
        Some(self.sym_new(Sym::Opaque(ids), &text))
    }

    /// In symbolic mode, `o[key]` when `o` is one or more objects of the file
    /// and `key` is a node.
    pub(super) fn sym_index(&mut self, o: &V<'a>, key: &V<'a>, span: Span) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let V::Js(j) = key else { return None };
        let id = sym_of(j)?;
        if !o.leaves().iter().all(|l| matches!(l, V::Object(_))) {
            return None;
        }
        let text = self.text(span);
        Some(V::Js(self.sym_new(
            Sym::Index {
                obj: o.clone(),
                key: id,
            },
            &text,
        )))
    }

    /// In symbolic mode, the nodes of `recv` as receivers of an array method
    /// ([`AS_ARRAY`]).
    pub(super) fn sym_as_array(&mut self, recv: V<'a>, span: Span) -> V<'a> {
        if self.sym.is_none() {
            return recv;
        }
        let leaves = recv.into_leaves();
        let mut out = Vec::with_capacity(leaves.len());
        for l in leaves {
            out.push(match &l {
                V::Js(j) if sym_of(j).is_some() => self.derived_member(j, AS_ARRAY, span),
                _ => l,
            });
        }
        join(out)
    }

    /// The node `id` as reads of a root: the node the reads start from, and the
    /// reads. The reads of a [`Sym::Reach`] on the way count as reads of
    /// its base.
    pub(crate) fn sym_chain(&self, mut id: u32) -> (u32, Vec<ReachStep>) {
        let mut steps: Vec<ReachStep> = Vec::new();
        let mut add = |s: ReachStep| {
            if !steps.contains(&s) {
                steps.push(s);
            }
        };
        for _ in 0..256 {
            match self.sym_node(id) {
                Some(Sym::Field(b, n)) => {
                    add(ReachStep::Field(n.clone()));
                    id = *b;
                }
                Some(Sym::Elem(b)) => {
                    // An element handed out by an array method is one step: only an
                    // array has it.
                    match self.sym_node(*b) {
                        Some(Sym::Field(bb, n)) if &**n == AS_ARRAY => {
                            add(ReachStep::Field(ARRAY_ELEMENT.into()));
                            id = *bb;
                        }
                        _ => {
                            add(ReachStep::Elem);
                            id = *b;
                        }
                    }
                }
                Some(Sym::AnyField(b)) => {
                    add(ReachStep::Any);
                    id = *b;
                }
                Some(Sym::Reach { base, steps: more }) => {
                    more.iter().cloned().for_each(&mut add);
                    id = *base;
                }
                _ => break,
            }
        }
        steps.sort();
        (id, steps)
    }

    /// In symbolic mode, the widening of the binding `sym` (at position `k` of
    /// the bindings being evaluated) whose value `v` did not settle: when
    /// every alternative is a chain of reads from a few roots, the roots
    /// with everything those reads can reach ([`Sym::Reach`]). The result
    /// is checked: evaluating the binding's definitions once more with it
    /// must give nothing outside it. `None`: no such description (the
    /// caller falls back to an unknown).
    pub(super) fn sym_widen(&mut self, sym: SymbolId, k: usize, v: &V<'a>) -> Option<V<'a>> {
        self.sym.as_ref()?;
        // Roots with the reads applied to them, and the alternatives without nodes.
        let mut roots: Vec<(u32, Vec<ReachStep>)> = Vec::new();
        let mut plain: Vec<V<'a>> = Vec::new();
        for l in v.leaves() {
            match l {
                V::Js(j) => match sym_of(j) {
                    Some(id) => {
                        let (root, steps) = self.sym_chain(id);
                        match roots.iter_mut().find(|(r, _)| *r == root) {
                            Some((_, have)) => {
                                for s in steps {
                                    if !have.contains(&s) {
                                        have.push(s);
                                    }
                                }
                                have.sort();
                            }
                            None => roots.push((root, steps)),
                        }
                    }
                    None => {
                        let mut inside = Vec::new();
                        syms_in(j, &mut inside);
                        if !inside.is_empty() {
                            return None;
                        }
                        plain.push(l.clone());
                    }
                },
                V::Void => plain.push(V::Void),
                // A container or a function built on the way: not a chain of reads.
                _ => return None,
            }
        }
        if roots.iter().all(|(_, steps)| steps.is_empty()) || roots.len() > 8 {
            return None;
        }
        let name = origin::clean(self.scoping.symbol_name(sym));
        let mut out = plain.clone();
        let mut reach: Vec<(u32, Vec<ReachStep>)> = Vec::new();
        for (root, steps) in &roots {
            if steps.is_empty() {
                let text = name.clone();
                let node = self.sym_node(*root)?.clone();
                out.push(V::Js(self.sym_new(node, &text)));
                reach.push((*root, Vec::new()));
                continue;
            }
            let j = self.sym_new(
                Sym::Reach {
                    base: *root,
                    steps: steps.clone(),
                },
                &name,
            );
            sym_of(&j)?;
            out.push(V::Js(j));
            reach.push((*root, steps.clone()));
        }
        let w = join(out);
        // The check: one more evaluation with the widened value must stay inside it.
        self.busy_partial(k, w.clone());
        let next = self.definitions_of(sym);
        for l in next.leaves() {
            let ok = match l {
                V::Void => true,
                V::Js(j) => match sym_of(j) {
                    Some(id) => {
                        let (root, steps) = self.sym_chain(id);
                        reach
                            .iter()
                            .any(|(r, have)| *r == root && steps.iter().all(|s| have.contains(s)))
                    }
                    None => plain.contains(l),
                },
                _ => false,
            };
            if !ok {
                return None;
            }
        }
        Some(w)
    }

    /// In symbolic mode, the value of the rest parameter declared at `decl`
    /// (`...args`): a hole with the index of the first argument it takes,
    /// which stands for the array of the remaining arguments. Calls in the
    /// file are not followed for rest parameters, so a function with
    /// visible calls also gets an unknown.
    pub(super) fn rest_hole(&mut self, decl: oxc_semantic::NodeId, name: &str) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let mut id = decl;
        let mut found = false;
        for _ in 0..3 {
            if let AstKind::FormalParameterRest(r) = self.nodes.kind(id) {
                found = matches!(r.rest.argument, BindingPattern::BindingIdentifier(_));
                break;
            }
            id = self.nodes.parent_id(id);
        }
        if !found {
            return None;
        }
        let list = self.nodes.parent_id(id);
        let AstKind::FormalParameters(ps) = self.nodes.kind(list) else {
            return None;
        };
        let fn_id = self.nodes.parent_id(list);
        let f = match self.nodes.kind(fn_id) {
            AstKind::Function(f) => FnRef::Function(f),
            AstKind::ArrowFunctionExpression(f) => FnRef::Arrow(f),
            _ => return None,
        };
        let index = ps.items.len();
        let local_calls = match &*self.reach(f, fn_id) {
            Reach::Named { calls, .. } => !calls.is_empty(),
            _ => false,
        };
        let hole = self.hole(f, index, "", name);
        Some(if local_calls {
            join([hole, V::unknown(Tag::Unknown, "", name)])
        } else {
            hole
        })
    }

    /// In symbolic mode, the module loaded by `require(arg)` / `import(arg)`
    /// with a specifier that is not a literal.
    pub(super) fn sym_require(&mut self, arg: &V<'a>, span: Span) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let text = self.text(span);
        Some(V::Js(self.sym_new(
            Sym::Call {
                callee: Callee::Import(Rc::new(ImportRef {
                    module: REQUIRE.into(),
                    name: "*".into(),
                })),
                args: vec![arg.clone()],
                rest: None,
            },
            &text,
        )))
    }

    /// In symbolic mode, the result of calling `callee` (a node or an imported
    /// binding) with `args`.
    pub(super) fn sym_call(
        &mut self,
        callee: &V<'a>,
        args: &'a [Argument<'a>],
        span: Span,
    ) -> Option<V<'a>> {
        self.sym.as_ref()?;
        let callee = match callee {
            V::Js(j) => Callee::Sym(sym_of(j)?),
            V::Import(i) => Callee::Import(i.clone()),
            _ => return None,
        };
        let a = self.args(args);
        let (vals, rest) = a.into_parts();
        let text = self.text(span);
        Some(V::Js(self.sym_new(
            Sym::Call {
                callee,
                args: vals,
                rest,
            },
            &text,
        )))
    }
}

impl<'a> Engine<'a, '_> {
    /// The value of the binding `sym`, as a site of its own.
    pub(crate) fn site_symbol(&mut self, sym: SymbolId) -> V<'a> {
        self.begin();
        let budget = self.budget();
        let v = self.symbol(sym);
        self.end(budget);
        v
    }

    /// The values of call arguments, as a site of their own.
    pub(crate) fn site_args(&mut self, args: &'a [Argument<'a>]) -> (Vec<V<'a>>, Option<V<'a>>) {
        self.begin();
        let budget = self.budget();
        let a = self.args(args);
        self.end(budget);
        a.into_parts()
    }

    /// Whether the reference `node` to a binding only reads it: nothing the
    /// file does through it there can change a container the binding holds
    /// ([`Engine::use_only_reads`]), as a site of its own.
    pub(crate) fn site_use_only_reads(&mut self, node: NodeId) -> bool {
        self.begin();
        let budget = self.budget();
        let r = self.use_only_reads(node, 0);
        self.end(budget);
        r
    }

    /// How the function `f` can be called, as a site of its own.
    pub(crate) fn site_reach(&mut self, f: FnRef<'a>, fn_id: NodeId) -> Rc<Reach<'a>> {
        self.begin();
        let budget = self.budget();
        let r = self.reach(f, fn_id);
        self.end(budget);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::{RECENT, recent};

    /// The node found is the one a search of every node finds, when that one is
    /// among the last `RECENT`; a match further back is none (a new node is
    /// made).
    #[test]
    fn a_node_is_looked_for_among_the_last_ones() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        for len in [0, 1, 2, RECENT - 1, RECENT, RECENT + 1, 3 * RECENT, 1000] {
            for kinds in [1, 3, 40, 500] {
                let nodes: Vec<u64> = (0..len).map(|_| next(kinds)).collect();
                for node in 0..kinds.min(60) {
                    let everywhere = nodes
                        .iter()
                        .rposition(|n| *n == node)
                        .filter(|i| nodes.len() - i <= RECENT);
                    assert_eq!(recent(&nodes, &node), everywhere, "{len} {kinds} {node}");
                }
            }
        }
        // The last of several, and nothing beyond the window.
        let mut nodes = vec![7u64; 3];
        assert_eq!(recent(&nodes, &7), Some(2));
        nodes.extend(std::iter::repeat_n(1, RECENT));
        assert_eq!(recent(&nodes, &7), None);
        assert_eq!(recent(&nodes, &1), Some(nodes.len() - 1));
    }
}
