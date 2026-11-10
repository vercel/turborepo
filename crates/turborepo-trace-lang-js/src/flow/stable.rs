//! Mutable state: how far container values and `this` fields can be trusted.
//!
//! - [`Facts`]: one scan of the file for writes the engine does not attribute
//!   to a tracked binding (property assignments, array mutations, `Map.set`),
//!   the functions that use `arguments`, and assignments to `this.x`.
//! - [`Engine::stabilize`]: the value of a binding initialized with a container
//!   literal, after checking every use of the binding: reads are fine, `.push`
//!   / `.add` / `.set` / property assignments become part of the value, an
//!   alias must itself only be read, and any other use (passed to code that is
//!   not evaluated, exported unfrozen, returned) makes the container open
//!   (every property or element may also be unknown).
//! - [`Engine::this_field`]: `this.x` inside a class.

use std::{collections::HashSet, rc::Rc};

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{NodeId, SymbolId};
use oxc_span::GetSpan;

use super::{
    engine::Engine,
    value::{ArrV, ObjV, Trust, V, join},
};
use crate::extract::origin::Tag;

/// Array and set methods that add elements.
const ADDERS: &[&str] = &["push", "unshift", "add", "splice", "fill", "copyWithin"];
/// Array methods that move elements to other positions in place.
const REORDERERS: &[&str] = &[
    "sort",
    "reverse",
    "shift",
    "unshift",
    "splice",
    "copyWithin",
    "fill",
];
/// Methods whose result is a new array (reordering it changes no other array).
const COPIES: &[&str] = &[
    "slice",
    "map",
    "filter",
    "concat",
    "flat",
    "flatMap",
    "keys",
    "values",
    "entries",
    "from",
    "split",
    "toSorted",
    "toReversed",
    "toSpliced",
    "with",
    "readdirSync",
    "match",
];
/// Methods of arrays, sets, maps and objects that never add a value.
pub(super) const READERS: &[&str] = &[
    "map",
    "filter",
    "forEach",
    "join",
    "includes",
    "has",
    "get",
    "find",
    "findLast",
    "findIndex",
    "findLastIndex",
    "some",
    "every",
    "slice",
    "concat",
    "indexOf",
    "lastIndexOf",
    "flat",
    "flatMap",
    "entries",
    "keys",
    "values",
    "reduce",
    "reduceRight",
    "at",
    "toString",
    "sort",
    "reverse",
    "pop",
    "shift",
    "delete",
    "clear",
    "hasOwnProperty",
    "toSorted",
    "toReversed",
    "toSpliced",
    "with",
];

/// An assignment to a field of `this`.
pub(super) struct ThisWrite<'a> {
    /// Start offset of the class.
    class: u32,
    /// The field; `None` for a write that can hit any field.
    name: Option<Box<str>>,
    /// The assigned value; `None` when it is not a plain `=`.
    value: Option<&'a Expression<'a>>,
}

/// Facts about the whole file.
#[derive(Default)]
pub(super) struct Facts<'a> {
    prop_writes: HashSet<&'a str>,
    any_prop_write: bool,
    array_mutation: bool,
    array_reorder: bool,
    map_mutation: bool,
    args_fns: HashSet<u32>,
    this_fns: HashSet<u32>,
    this_writes: Vec<ThisWrite<'a>>,
}

impl Facts<'_> {
    /// Whether property `name` of some untracked object is assigned in the
    /// file.
    pub(super) fn writes_prop(&self, name: &str) -> bool {
        self.any_prop_write || self.prop_writes.contains(name)
    }

    /// Whether property `name` of some untracked object is assigned by name
    /// (`other.name = …`): `other` can be an instance of the class that
    /// declares `name`.
    fn names_prop(&self, name: &str) -> bool {
        self.prop_writes.contains(name)
    }

    /// Whether some untracked object is written with a computed key.
    pub(super) fn any_prop_write(&self) -> bool {
        self.any_prop_write || !self.prop_writes.is_empty()
    }

    /// Whether some untracked array or set gets elements added.
    pub(super) fn array_mutation(&self) -> bool {
        self.array_mutation
    }

    /// Whether some untracked array is reordered in place (`sort`, `reverse`,
    /// `shift`, …): the positions of a shared array's elements are then not
    /// known.
    pub(super) fn array_reorder(&self) -> bool {
        self.array_reorder
    }

    /// Whether some untracked map gets entries set.
    fn map_mutation(&self) -> bool {
        self.map_mutation
    }

    /// Whether the function starting at `start` uses `arguments`.
    pub(super) fn uses_arguments(&self, start: u32) -> bool {
        self.args_fns.contains(&start)
    }

    /// Whether the (non-arrow) function starting at `start` uses `this`.
    pub(super) fn uses_this(&self, start: u32) -> bool {
        self.this_fns.contains(&start)
    }

    /// The property names some untracked object is assigned by name, sorted.
    pub(super) fn written_props(&self) -> Vec<String> {
        let mut v: Vec<String> = self.prop_writes.iter().map(|s| (*s).to_owned()).collect();
        v.sort_unstable();
        v
    }

    /// Whether some untracked object is written with a key that is not a
    /// literal name (`o[k] = v`, `Object.assign(o, …)`, a destructuring
    /// into members).
    pub(super) fn writes_any_prop(&self) -> bool {
        self.any_prop_write
    }
}

/// How a binding that holds a container is used at one reference.
enum Use<'a> {
    Read,
    /// A property (or map entry) is set; `None` key: not known.
    Set(Option<String>, Option<&'a Expression<'a>>),
    /// Bound to another name, which must only be read.
    Alias(SymbolId),
    /// A method is called on the container: a builtin (`push`, `add`, `set`, a
    /// reader) or a method of the object literal itself
    /// ([`Engine::stabilize`] decides, it knows the value).
    Method(String, &'a [Argument<'a>]),
    /// Two uses at once: `binding.sort().push(x)` (the call returns the
    /// container).
    Both(Box<Use<'a>>, Box<Use<'a>>),
    /// The value of a property (`None`: of any property or element) is handed
    /// to code that is not evaluated: a container stored there can be
    /// changed.
    Escapes(Option<String>),
    /// Anything can happen to the container.
    Taint,
}

/// What a builtin method of an array, set or map does to its receiver.
enum Effect<'a> {
    Read,
    /// Elements are added.
    Add(&'a [Argument<'a>]),
    /// An entry is set.
    Set(Option<String>, Option<&'a Expression<'a>>),
    Unknown,
}

/// Array, set and map methods that return their receiver.
const SELF_RETURNING: &[&str] = &["add", "set", "sort", "reverse", "fill", "copyWithin"];

fn builtin_effect<'a>(name: &str, args: &'a [Argument<'a>]) -> Effect<'a> {
    match name {
        "push" | "unshift" | "add" => Effect::Add(args),
        "splice" => Effect::Add(args.get(2..).unwrap_or(&[])),
        "fill" => Effect::Add(args.get(..1).unwrap_or(&[])),
        "set" if args.len() == 2 => {
            let k = match args[0].as_expression().map(|e| e.get_inner_expression()) {
                Some(Expression::StringLiteral(s)) => s.value.as_str().map(str::to_owned),
                _ => None,
            };
            Effect::Set(k, args[1].as_expression())
        }
        m if READERS.contains(&m) => Effect::Read,
        _ => Effect::Unknown,
    }
}

impl<'a, 's> Engine<'a, 's> {
    pub(super) fn facts(&mut self) -> Rc<Facts<'a>> {
        if let Some(f) = &self.facts {
            return f.clone();
        }
        let f = Rc::new(self.build_facts());
        self.facts = Some(f.clone());
        f
    }

    fn build_facts(&self) -> Facts<'a> {
        let mut f = Facts::default();
        for node in self.nodes.iter() {
            match node.kind() {
                AstKind::AssignmentExpression(a) => {
                    let (object, name): (&Expression<'a>, Option<&'a str>) = match &a.left {
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
                        AssignmentTarget::PrivateFieldExpression(m) => {
                            let this = matches!(
                                m.object.get_inner_expression(),
                                Expression::ThisExpression(_)
                            );
                            // `other.#x = v` writes the field of another instance (of any
                            // class with a field of that name, as far as this scan knows).
                            f.this_writes.push(ThisWrite {
                                class: if this {
                                    self.enclosing_class(node.id())
                                        .map_or(u32::MAX, |c| c.span.start)
                                } else {
                                    u32::MAX
                                },
                                name: Some(format!("#{}", m.field.name).into()),
                                value: (this && a.operator == AssignmentOperator::Assign)
                                    .then_some(&a.right),
                            });
                            continue;
                        }
                        AssignmentTarget::ArrayAssignmentTarget(_)
                        | AssignmentTarget::ObjectAssignmentTarget(_) => {
                            // Destructuring into members: not analyzed.
                            let text = self
                                .src
                                .get(a.left.span().start as usize..a.left.span().end as usize)
                                .unwrap_or("");
                            if text.contains('.') || text.get(1..).is_some_and(|t| t.contains('['))
                            {
                                f.any_prop_write = true;
                                f.this_writes.push(ThisWrite {
                                    class: u32::MAX,
                                    name: None,
                                    value: None,
                                });
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    let object = object.get_inner_expression();
                    match object {
                        Expression::ThisExpression(_) => f.this_writes.push(ThisWrite {
                            class: self
                                .enclosing_class(node.id())
                                .map_or(u32::MAX, |c| c.span.start),
                            name: name.map(Into::into),
                            value: (a.operator == AssignmentOperator::Assign).then_some(&a.right),
                        }),
                        Expression::Identifier(id) if self.tracked_root(id) => {}
                        Expression::Identifier(id)
                            if id.name == "exports" && self.is_global(id) => {}
                        // `module.exports = v` defines the exports, it changes no object
                        // (cross-file facts only: the per-file pass keeps counting it).
                        Expression::Identifier(id)
                            if self.sym.is_some()
                                && id.name == "module"
                                && name == Some("exports")
                                && self.is_global(id) => {}
                        Expression::StaticMemberExpression(m)
                            if m.property.name == "exports"
                                && matches!(&m.object, Expression::Identifier(i) if i.name == "module") =>
                            {}
                        _ => match name {
                            Some(n) => {
                                f.prop_writes.insert(n);
                            }
                            None => f.any_prop_write = true,
                        },
                    }
                }
                // `x.n++`, `this.n--`: a write of something that is not a plain value.
                AstKind::UpdateExpression(u) => {
                    let (object, name): (&Expression<'a>, Option<&'a str>) = match &u.argument {
                        SimpleAssignmentTarget::StaticMemberExpression(m) => {
                            (&m.object, Some(m.property.name.as_str()))
                        }
                        SimpleAssignmentTarget::ComputedMemberExpression(m) => (
                            &m.object,
                            match &m.expression {
                                Expression::StringLiteral(s) => s.value.as_str(),
                                _ => None,
                            },
                        ),
                        SimpleAssignmentTarget::PrivateFieldExpression(m) => {
                            let this = matches!(
                                m.object.get_inner_expression(),
                                Expression::ThisExpression(_)
                            );
                            f.this_writes.push(ThisWrite {
                                class: if this {
                                    self.enclosing_class(node.id())
                                        .map_or(u32::MAX, |c| c.span.start)
                                } else {
                                    u32::MAX
                                },
                                name: Some(format!("#{}", m.field.name).into()),
                                value: None,
                            });
                            continue;
                        }
                        _ => continue,
                    };
                    match object.get_inner_expression() {
                        Expression::ThisExpression(_) => f.this_writes.push(ThisWrite {
                            class: self
                                .enclosing_class(node.id())
                                .map_or(u32::MAX, |c| c.span.start),
                            name: name.map(Into::into),
                            value: None,
                        }),
                        Expression::Identifier(id) if self.tracked_root(id) => {}
                        _ => match name {
                            Some(n) => {
                                f.prop_writes.insert(n);
                            }
                            None => f.any_prop_write = true,
                        },
                    }
                }
                AstKind::CallExpression(c) => {
                    let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression()
                    else {
                        continue;
                    };
                    let method = m.property.name.as_str();
                    let recv = m.object.get_inner_expression();
                    // A tracked binding checks its own uses; a container created right there
                    // (`new Set().add(x)`) has no alias yet.
                    let tracked = matches!(recv, Expression::Identifier(id) if self.tracked_root(id))
                        || is_fresh_container(recv, 0);
                    let copy = matches!(recv, Expression::CallExpression(inner)
                        if matches!(inner.callee.get_inner_expression(),
                            Expression::StaticMemberExpression(im)
                                if COPIES.contains(&im.property.name.as_str())));
                    if REORDERERS.contains(&method) && !tracked && !copy {
                        f.array_reorder = true;
                    }
                    if ADDERS.contains(&method) && !tracked {
                        f.array_mutation = true;
                    } else if method == "set" && c.arguments.len() == 2 && !tracked {
                        f.map_mutation = true;
                    } else if matches!(recv, Expression::Identifier(id) if (id.name == "Object" || id.name == "Reflect") && self.is_global(id))
                        && matches!(
                            method,
                            "assign"
                                | "defineProperty"
                                | "defineProperties"
                                | "set"
                                | "setPrototypeOf"
                        )
                    {
                        match c
                            .arguments
                            .first()
                            .and_then(|a| a.as_expression())
                            .map(|e| e.get_inner_expression())
                        {
                            // Handled by the binding's own use check.
                            Some(Expression::Identifier(id)) if self.tracked_root(id) => {}
                            // A fresh object: `Object.assign({}, a, b)`.
                            Some(Expression::ObjectExpression(_)) => {}
                            Some(Expression::ThisExpression(_)) => f.this_writes.push(ThisWrite {
                                class: self
                                    .enclosing_class(node.id())
                                    .map_or(u32::MAX, |c| c.span.start),
                                name: None,
                                value: None,
                            }),
                            // Some other object: also an instance of any class.
                            _ => {
                                f.any_prop_write = true;
                                f.this_writes.push(ThisWrite {
                                    class: u32::MAX,
                                    name: None,
                                    value: None,
                                });
                            }
                        }
                    }
                }
                AstKind::ThisExpression(_) => {
                    for k in self.nodes.ancestor_kinds(node.id()) {
                        if let AstKind::Function(func) = k {
                            f.this_fns.insert(func.span.start);
                            break;
                        }
                    }
                }
                AstKind::IdentifierReference(id)
                    if id.name == "arguments" && self.is_global(id) =>
                {
                    for k in self.nodes.ancestor_kinds(node.id()) {
                        if let AstKind::Function(func) = k {
                            f.args_fns.insert(func.span.start);
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        f
    }

    fn is_global(&self, id: &IdentifierReference<'a>) -> bool {
        id.reference_id
            .get()
            .is_none_or(|r| self.scoping.get_reference(r).symbol_id().is_none())
    }

    /// Whether `id` names a binding whose containers are tracked by its own use
    /// check.
    fn tracked_root(&self, id: &IdentifierReference<'a>) -> bool {
        id.reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
            .is_some_and(|s| self.literal_binding(s))
    }

    /// Whether `sym` is declared once with a container literal (`{…}`, `[…]`,
    /// `new Set(…)`, `new Map(…)`, optionally frozen or cast) and never
    /// assigned again.
    pub(super) fn literal_binding(&self, sym: SymbolId) -> bool {
        let decl = self.scoping.symbol_declaration(sym);
        let AstKind::VariableDeclarator(d) = self.nodes.kind(decl) else {
            return false;
        };
        if !matches!(d.id, BindingPattern::BindingIdentifier(_))
            || !self.scoping.symbol_redeclarations(sym).is_empty()
            || self
                .scoping
                .get_resolved_references(sym)
                .any(|r| r.is_write())
        {
            return false;
        }
        let Some(init) = &d.init else { return false };
        is_container_literal(init.get_inner_expression())
    }

    /// Whether the binding is declared in an `export`, and whether importers
    /// cannot change its container: `Some(true)` for `as const` (deeply
    /// read-only), `Some(false)` for `Object.freeze` (only the container
    /// itself is frozen), `None` otherwise.
    fn exported_frozen(&self, sym: SymbolId) -> (bool, Option<bool>) {
        let decl = self.scoping.symbol_declaration(sym);
        let AstKind::VariableDeclarator(d) = self.nodes.kind(decl) else {
            return (false, None);
        };
        let frozen = d.init.as_ref().and_then(|init| {
            let mut e = init;
            loop {
                match e {
                    Expression::ParenthesizedExpression(p) => e = &p.expression,
                    Expression::TSSatisfiesExpression(x) => e = &x.expression,
                    Expression::TSAsExpression(x) => {
                        let t = x.type_annotation.span();
                        if self.src.get(t.start as usize..t.end as usize) == Some("const") {
                            return Some(true);
                        }
                        e = &x.expression;
                    }
                    Expression::CallExpression(c) => {
                        return matches!(
                            c.callee.get_inner_expression(),
                            Expression::StaticMemberExpression(m)
                                if m.property.name == "freeze"
                                    && matches!(&m.object, Expression::Identifier(i) if i.name == "Object")
                        )
                        .then_some(false);
                    }
                    _ => return None,
                }
            }
        });
        let list = self.nodes.parent_id(decl);
        let exported = matches!(
            self.nodes.parent_kind(list),
            AstKind::ExportDeclaration(_)
                | AstKind::ExportNamedDeclaration(_)
                | AstKind::ExportDefaultDeclaration(_)
        );
        (exported, frozen)
    }

    /// The value `v` of the literal binding `sym` after checking its uses (see
    /// the module docs).
    pub(super) fn stabilize(&mut self, sym: SymbolId, v: V<'a>) -> V<'a> {
        if self.checking.contains(&sym) || self.checking.len() >= 4 {
            // The uses of this binding are not checked here (it is being checked further
            // up, or the nesting bound is reached), so its own `push` / `set` calls are not
            // part of the value: it can hold anything. The file-wide facts do not help,
            // they leave out the mutations of tracked bindings.
            let open = V::unknown(Tag::Unknown, "", "container not evaluated");
            return join(v.into_leaves().into_iter().map(|l| match l {
                V::Object(o) => {
                    let mut o: ObjV<'a> = (*o).clone();
                    o.widen(open.clone());
                    o.trust = Trust::Shared;
                    V::Object(Rc::new(o))
                }
                V::Array(a) => {
                    let mut a: ArrV<'a> = (*a).clone();
                    a.add(open.clone());
                    a.trust = Trust::Shared;
                    V::Array(Rc::new(a))
                }
                l => l,
            }));
        }
        self.checking.push(sym);
        let scoping = self.scoping;
        let (exported, frozen) = self.exported_frozen(sym);
        // `Object.freeze` is shallow: importers can still change a nested container.
        let nested = |v: &V<'a>| {
            v.leaves().iter().any(|l| match l {
                V::Object(o) => o.props.iter().any(|(_, x)| x.has_container()),
                V::Array(a) => {
                    a.elems.iter().any(V::has_container)
                        || a.rest.as_ref().is_some_and(V::has_container)
                }
                _ => false,
            })
        };
        let frozen = match frozen {
            Some(true) => true,
            Some(false) => !nested(&v),
            None => false,
        };
        let mut taint = exported && !frozen;
        // `binding.sort()`, `binding.reverse()`, `binding.shift()`: positions change.
        let mut reordered = false;
        // Properties whose value is handed to code that is not evaluated.
        let mut escapes: Vec<Option<String>> = Vec::new();
        let mut adds: Vec<V<'a>> = Vec::new();
        let mut sets: Vec<(Option<String>, V<'a>)> = Vec::new();
        for r in scoping.get_resolved_references(sym) {
            if taint {
                break;
            }
            if !self.tick() {
                taint = true;
                break;
            }
            let mut uses = vec![self.use_of(r.node_id(), frozen)];
            while let Some(u) = uses.pop() {
                match u {
                    Use::Read => {}
                    Use::Both(a, b) => {
                        uses.push(*a);
                        uses.push(*b);
                    }
                    Use::Set(key, value) => {
                        let val = match value {
                            Some(e) => self.eval(e),
                            None => V::unknown(Tag::Unknown, "", "compound assignment"),
                        };
                        sets.push((key, val));
                    }
                    Use::Alias(other) => {
                        if !self.only_read(other, 0) {
                            taint = true;
                        }
                    }
                    Use::Method(name, args) => {
                        // A method of the object literal itself (whatever its name: `set`,
                        // `add`, `push` included): fine unless it can reach the object
                        // through `this`.
                        let own = v
                            .leaves()
                            .iter()
                            .any(|l| matches!(l, V::Object(o) if !o.map && o.get(&name).is_some()));
                        if own {
                            let facts = self.facts();
                            let safe = v.leaves().iter().all(|l| match l {
                                V::Object(o) => o.get(&name).is_some_and(|f| {
                                    f.leaves().iter().all(|f| match f {
                                        V::Func(fr) => !facts.uses_this(fr.span().start),
                                        _ => false,
                                    })
                                }),
                                _ => false,
                            });
                            if !safe {
                                taint = true;
                            }
                            continue;
                        }
                        reordered |= REORDERERS.contains(&name.as_str());
                        match builtin_effect(&name, args) {
                            Effect::Read => {}
                            Effect::Add(args) => {
                                let a = self.args(args);
                                adds.push(a.tail(0));
                            }
                            Effect::Set(key, value) => {
                                let val = match value {
                                    Some(e) => self.eval(e),
                                    None => V::unknown(Tag::Unknown, "", "entry"),
                                };
                                sets.push((key, val));
                            }
                            Effect::Unknown => taint = true,
                        }
                    }
                    Use::Escapes(key) => escapes.push(key),
                    Use::Taint => taint = true,
                }
            }
        }
        self.checking.pop();
        // An escaping property matters when it can hold a container: in the literal or
        // assigned later (the assignments are known only now).
        for key in &escapes {
            let named = |k: &str| key.as_deref().is_none_or(|want| want == k);
            let in_literal = v.leaves().iter().any(|l| match l {
                V::Object(o) => {
                    o.props.iter().any(|(k, x)| named(k) && x.has_container())
                        || o.open.as_ref().is_some_and(V::has_container)
                }
                V::Array(a) => {
                    a.elems.iter().any(V::has_container)
                        || a.rest.as_ref().is_some_and(V::has_container)
                }
                _ => false,
            });
            let assigned = sets
                .iter()
                .any(|(k, x)| k.as_deref().is_none_or(named) && x.has_container())
                || adds.iter().any(|a| match a {
                    V::Array(arr) => {
                        arr.elems.iter().any(V::has_container)
                            || arr.rest.as_ref().is_some_and(V::has_container)
                    }
                    _ => false,
                });
            if in_literal || assigned {
                taint = true;
            }
        }
        let mut add_vals = Vec::new();
        for a in adds {
            add_vals.push(self.elements(a, Span::default()));
        }
        let open = taint.then(|| V::unknown(Tag::Unknown, "", "container used elsewhere"));
        join(v.into_leaves().into_iter().map(|l| match l {
            V::Object(o) => {
                let mut o: ObjV<'a> = (*o).clone();
                for (k, val) in &sets {
                    match k {
                        // A new prototype: any property can come from it.
                        Some(k) if k == "__proto__" => {
                            o.widen(V::unknown(Tag::Unknown, "", "inherited property"))
                        }
                        Some(k) => o.set(k, val.clone(), true),
                        None => o.widen(val.clone()),
                    }
                }
                if let Some(u) = &open {
                    o.widen(u.clone());
                }
                o.trust = Trust::Tracked;
                V::Object(Rc::new(o))
            }
            V::Array(a) => {
                let mut a: ArrV<'a> = (*a).clone();
                for val in add_vals.iter().cloned() {
                    a.add(val);
                }
                // Index assignments.
                for (_, val) in &sets {
                    a.add(val.clone());
                }
                if let Some(u) = &open {
                    a.add(u.clone());
                }
                if reordered {
                    for e in std::mem::take(&mut a.elems) {
                        a.add(e);
                    }
                }
                a.trust = Trust::Tracked;
                V::Array(Rc::new(a))
            }
            l => l,
        }))
    }

    /// Whether the alias `sym` of a tracked container is only read (and at most
    /// aliased to names that are only read).
    pub(super) fn only_read(&mut self, sym: SymbolId, depth: u32) -> bool {
        if depth > 2
            || self
                .scoping
                .get_resolved_references(sym)
                .any(|r| r.is_write())
        {
            return false;
        }
        let scoping = self.scoping;
        for r in scoping.get_resolved_references(sym) {
            if !r.is_read() && self.sym.is_some() {
                // A type position (cross-file facts only: the per-file pass judges it
                // like any other use).
                continue;
            }
            if !self.use_only_reads(r.node_id(), depth) {
                return false;
            }
        }
        true
    }

    /// Whether the reference `node` to a binding that can hold a container only
    /// reads it (see [`Engine::only_read`]).
    pub(super) fn use_only_reads(&mut self, node: NodeId, depth: u32) -> bool {
        if !self.tick() {
            return false;
        }
        let mut uses = vec![self.use_of(node, false)];
        while let Some(u) = uses.pop() {
            match u {
                Use::Read => {}
                Use::Both(a, b) => {
                    uses.push(*a);
                    uses.push(*b);
                }
                Use::Alias(other) => {
                    if !self.only_read(other, depth + 1) {
                        return false;
                    }
                }
                Use::Method(name, _)
                    if READERS.contains(&name.as_str()) && !REORDERERS.contains(&name.as_str()) => {
                }
                Use::Method(..) | Use::Set(..) | Use::Escapes(_) | Use::Taint => return false,
            }
        }
        true
    }

    /// How the result of the call `call` is used when the call returns the
    /// container itself (`binding.add(x)`, `binding.sort()`): dropped, or
    /// used like the binding.
    fn result_use(&mut self, call: NodeId) -> Use<'a> {
        let (pid, pk) = self.parent(call);
        match pk {
            AstKind::ExpressionStatement(_) => Use::Read,
            // `xs.forEach((x) => set.add(x))`: `forEach` drops what its callback returns.
            AstKind::ArrowFunctionExpression(f)
                if !matches!(f.body, ArrowFunctionBody::FunctionBody(_)) =>
            {
                let fspan = f.span;
                match self.parent(pid) {
                    (_, AstKind::CallExpression(c))
                        if matches!(
                            c.callee.get_inner_expression(),
                            Expression::StaticMemberExpression(m) if m.property.name == "forEach"
                        ) && c.arguments.first().is_some_and(|a| {
                            a.as_expression()
                                .is_some_and(|e| e.get_inner_expression().span() == fspan)
                        }) =>
                    {
                        Use::Read
                    }
                    _ => Use::Taint,
                }
            }
            _ => self.use_of(call, false),
        }
    }

    /// Classifies the reference `node` (an identifier) to a binding holding a
    /// container.
    fn use_of(&mut self, node: NodeId, frozen: bool) -> Use<'a> {
        let span = self.nodes.kind(node).span();
        let (pid, pk) = self.parent(node);
        match pk {
            AstKind::StaticMemberExpression(m)
                if m.object.get_inner_expression().span() == span =>
            {
                self.member_use(pid, m.span, Some(m.property.name.as_str().to_owned()))
            }
            AstKind::ComputedMemberExpression(m)
                if m.object.get_inner_expression().span() == span =>
            {
                let key = match m.expression.get_inner_expression() {
                    Expression::StringLiteral(s) => s.value.as_str().map(str::to_owned),
                    _ => None,
                };
                self.member_use(pid, m.span, key)
            }
            AstKind::SpreadElement(_) => Use::Read,
            AstKind::ForOfStatement(f) if f.right.get_inner_expression().span() == span => {
                Use::Read
            }
            AstKind::ForInStatement(f) if f.right.get_inner_expression().span() == span => {
                Use::Read
            }
            AstKind::CallExpression(c) => {
                if c.callee.get_inner_expression().span() == span {
                    return Use::Read;
                }
                let Some(index) = c.arguments.iter().position(|a| {
                    a.as_expression()
                        .is_some_and(|e| e.get_inner_expression().span() == span)
                }) else {
                    return Use::Taint;
                };
                if self.pure_callee(c, index) {
                    return Use::Read;
                }
                // A function of the file: its parameter is an alias.
                match self.param_symbol(c, index) {
                    Some(Some(s)) => Use::Alias(s),
                    // Destructured: only members are read.
                    Some(None) => Use::Read,
                    None => Use::Taint,
                }
            }
            AstKind::NewExpression(n) => match n.callee.get_inner_expression() {
                Expression::Identifier(id)
                    if matches!(
                        id.name.as_str(),
                        "Set" | "Map" | "Array" | "Error" | "WeakSet"
                    ) =>
                {
                    Use::Read
                }
                _ => Use::Taint,
            },
            AstKind::VariableDeclarator(d) => match &d.id {
                BindingPattern::BindingIdentifier(id) => match id.symbol_id.get() {
                    Some(s) => Use::Alias(s),
                    None => Use::Taint,
                },
                // Destructuring reads members.
                _ => Use::Read,
            },
            AstKind::ConditionalExpression(c) if c.test.get_inner_expression().span() == span => {
                Use::Read
            }
            AstKind::LogicalExpression(l)
                if l.operator == LogicalOperator::And
                    && l.left.get_inner_expression().span() == span =>
            {
                Use::Read
            }
            // A tag function gets the container itself.
            AstKind::TemplateLiteral(_)
                if matches!(
                    self.nodes.parent_kind(pid),
                    AstKind::TaggedTemplateExpression(_)
                ) =>
            {
                Use::Taint
            }
            AstKind::IfStatement(_)
            | AstKind::WhileStatement(_)
            | AstKind::DoWhileStatement(_)
            | AstKind::UnaryExpression(_)
            | AstKind::BinaryExpression(_)
            | AstKind::TemplateLiteral(_)
            | AstKind::ExpressionStatement(_)
            | AstKind::TSTypeQuery(_)
            | AstKind::SwitchStatement(_) => Use::Read,
            AstKind::ExportSpecifier(_) | AstKind::ExportDefaultDeclaration(_) if frozen => {
                Use::Read
            }
            _ => Use::Taint,
        }
    }

    /// The use of `binding.key` (the member expression `member`, spanning
    /// `mspan`).
    fn member_use(&mut self, member: NodeId, mspan: Span, key: Option<String>) -> Use<'a> {
        let (gid, gk) = self.parent(member);
        match gk {
            AstKind::CallExpression(c) if c.callee.get_inner_expression().span() == mspan => {
                let Some(m) = key else {
                    return Use::Taint;
                };
                // `binding.add(a).add(b)`, `const s = binding.sort()`: the result is the
                // container again.
                let result = if SELF_RETURNING.contains(&m.as_str()) {
                    self.result_use(gid)
                } else {
                    Use::Read
                };
                // `binding.forEach((x, i, all) => all.push(y))`: an iteration callback gets
                // the container as its last parameter.
                let position = match m.as_str() {
                    "forEach" | "map" | "filter" | "find" | "findLast" | "findIndex"
                    | "findLastIndex" | "some" | "every" | "flatMap" => Some(2),
                    "reduce" | "reduceRight" => Some(3),
                    _ => None,
                };
                let params = match c.arguments.first().and_then(|a| a.as_expression()) {
                    Some(Expression::ArrowFunctionExpression(f)) => Some(&f.params),
                    Some(Expression::FunctionExpression(f)) => Some(&f.params),
                    _ => None,
                };
                let passed = match (position, params) {
                    (Some(i), Some(ps)) => match ps.items.get(i).map(|p| &p.pattern) {
                        Some(BindingPattern::BindingIdentifier(id)) => match id.symbol_id.get() {
                            Some(s) => Use::Alias(s),
                            None => Use::Taint,
                        },
                        // Destructured or absent: only members are read. A rest parameter
                        // can hold it.
                        _ if ps.rest.is_some() => Use::Taint,
                        _ => Use::Read,
                    },
                    _ => Use::Read,
                };
                let mut all = Use::Method(m, &c.arguments);
                for other in [result, passed] {
                    if !matches!(other, Use::Read) {
                        all = Use::Both(Box::new(all), Box::new(other));
                    }
                }
                all
            }
            AstKind::AssignmentExpression(a) if a.left.span() == mspan => Use::Set(
                key,
                (a.operator == AssignmentOperator::Assign).then_some(&a.right),
            ),
            // `binding.n++`: the property becomes another number.
            AstKind::UpdateExpression(_) => Use::Set(key, None),
            // `binding.a.b`: a nested container. Writing to it or calling a method that
            // changes it is not tracked.
            AstKind::StaticMemberExpression(_) | AstKind::ComputedMemberExpression(_) => {
                let mut id = gid;
                let mut cur = self.nodes.kind(gid).span();
                for _ in 0..16 {
                    let (p, k) = self.parent(id);
                    match k {
                        AstKind::StaticMemberExpression(m)
                            if m.object.get_inner_expression().span() == cur =>
                        {
                            id = p;
                            cur = m.span;
                        }
                        AstKind::ComputedMemberExpression(m)
                            if m.object.get_inner_expression().span() == cur =>
                        {
                            id = p;
                            cur = m.span;
                        }
                        AstKind::AssignmentExpression(a) if a.left.span() == cur => {
                            return Use::Taint;
                        }
                        AstKind::CallExpression(c)
                            if c.callee.get_inner_expression().span() == cur =>
                        {
                            let name = match self.nodes.kind(id) {
                                AstKind::StaticMemberExpression(m) => m.property.name.as_str(),
                                _ => "",
                            };
                            return if READERS.contains(&name) {
                                Use::Read
                            } else {
                                Use::Taint
                            };
                        }
                        AstKind::ArrayAssignmentTarget(_)
                        | AstKind::ObjectAssignmentTarget(_)
                        | AstKind::AssignmentTargetWithDefault(_)
                        | AstKind::AssignmentTargetPropertyProperty(_)
                        | AstKind::AssignmentTargetRest(_) => return Use::Taint,
                        // `binding.a.b` as a value: see the last arm below.
                        _ => {
                            return match self.use_of(id, false) {
                                Use::Taint => Use::Escapes(key),
                                Use::Alias(s) if !self.only_read(s, 1) => Use::Escapes(key),
                                _ => Use::Read,
                            };
                        }
                    }
                }
                Use::Taint
            }
            AstKind::ArrayAssignmentTarget(_)
            | AstKind::ObjectAssignmentTarget(_)
            | AstKind::AssignmentTargetWithDefault(_)
            | AstKind::AssignmentTargetPropertyProperty(_)
            | AstKind::AssignmentTargetRest(_) => Use::Taint,
            AstKind::ForOfStatement(f) if f.left.span() == mspan => Use::Set(key, None),
            AstKind::ForInStatement(f) if f.left.span() == mspan => Use::Set(key, None),
            // `binding.key` as a value: an alias of a container stored there. Handing it to
            // code that is not evaluated (an unknown callee, a `return`, an export) lets
            // that code change the nested container, as for the binding itself.
            _ => match self.use_of(member, false) {
                Use::Taint => Use::Escapes(key),
                Use::Alias(s) if !self.only_read(s, 1) => Use::Escapes(key),
                _ => Use::Read,
            },
        }
    }

    /// The binding that receives argument `index` of the call `c` when the
    /// callee is a function of the file: `Some(Some(symbol))` for a plain
    /// parameter, `Some(None)` for a destructured one, `None` when the
    /// callee is not known.
    fn param_symbol(
        &mut self,
        c: &'a CallExpression<'a>,
        index: usize,
    ) -> Option<Option<SymbolId>> {
        let Expression::Identifier(id) = c.callee.get_inner_expression() else {
            return None;
        };
        let sym = id
            .reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())?;
        if self
            .scoping
            .get_resolved_references(sym)
            .any(|r| r.is_write())
        {
            return None;
        }
        let params = match self.nodes.kind(self.scoping.symbol_declaration(sym)) {
            AstKind::Function(f) if f.body.is_some() => &f.params,
            AstKind::VariableDeclarator(d) => match d.init.as_ref()?.get_inner_expression() {
                Expression::ArrowFunctionExpression(f) => &f.params,
                Expression::FunctionExpression(f) => &f.params,
                _ => return None,
            },
            _ => return None,
        };
        if c.arguments.iter().take(index + 1).any(|a| a.is_spread()) {
            return None;
        }
        let p = params.items.get(index)?;
        Some(match &p.pattern {
            BindingPattern::BindingIdentifier(id) => Some(id.symbol_id.get()?),
            _ => None,
        })
    }

    // ---- classes ----

    /// The class whose instance `this` is at `at` (through arrow functions,
    /// methods and field initializers).
    pub(super) fn enclosing_class(&self, at: NodeId) -> Option<&'a Class<'a>> {
        let mut prev_function = false;
        for id in self.nodes.ancestor_ids(at) {
            match self.nodes.kind(id) {
                AstKind::Function(_) => {
                    if prev_function {
                        return None;
                    }
                    prev_function = true;
                }
                AstKind::MethodDefinition(m) => {
                    if m.r#static {
                        return None;
                    }
                    prev_function = false;
                }
                AstKind::PropertyDefinition(p) => {
                    if p.r#static {
                        return None;
                    }
                }
                AstKind::ObjectProperty(_) if prev_function => return None,
                AstKind::StaticBlock(_) | AstKind::Program(_) => return None,
                AstKind::Class(c) => return (!prev_function).then_some(c),
                _ => {
                    if prev_function {
                        // A plain function rebinds `this`.
                        return None;
                    }
                }
            }
        }
        None
    }

    /// `this.name` at `at`: known when the field cannot be written from outside
    /// the class and cannot be redeclared by a subclass (`#name`,
    /// TypeScript `private`): the join of its initializer, the constructor
    /// parameter property, and every `this.name = …` in the class. A
    /// `readonly` field that is not private is not enough: a subclass can
    /// declare it again with another value. A write through another reference
    /// in the file (`other.name = …`, `self.name = …`, `other.#name = …`)
    /// makes the field unknown.
    pub(super) fn this_field(&mut self, at: NodeId, name: &str, span: Span) -> V<'a> {
        let Some(class) = self.enclosing_class(at) else {
            return self.unk(Tag::Unknown, "", span);
        };
        let mut protected = name.starts_with('#');
        let mut inits: Vec<&'a Expression<'a>> = Vec::new();
        let mut params: Vec<SymbolId> = Vec::new();
        for el in &class.body.body {
            match el {
                ClassElement::PropertyDefinition(p) if key_is(&p.key, name) => {
                    if p.r#static || !p.decorators.is_empty() || p.declare {
                        return self.unk(Tag::Unknown, "", span);
                    }
                    protected |= p.accessibility == Some(TSAccessibility::Private);
                    inits.extend(p.value.as_ref());
                }
                ClassElement::MethodDefinition(m)
                    if m.kind == MethodDefinitionKind::Constructor =>
                {
                    for p in &m.value.params.items {
                        if !(p.accessibility.is_some() || p.readonly) {
                            continue;
                        }
                        let BindingPattern::BindingIdentifier(id) = &p.pattern else {
                            continue;
                        };
                        if id.name != name {
                            continue;
                        }
                        protected |= p.accessibility == Some(TSAccessibility::Private);
                        params.extend(id.symbol_id.get());
                    }
                }
                // A method or accessor of that name.
                ClassElement::MethodDefinition(m) if key_is(&m.key, name) => {
                    return self.unk(Tag::Unknown, "", span);
                }
                ClassElement::AccessorProperty(p) if key_is(&p.key, name) => {
                    return self.unk(Tag::Unknown, "", span);
                }
                _ => {}
            }
        }
        if !protected {
            return self.unk(Tag::Unknown, "", span);
        }
        let facts = self.facts();
        if !name.starts_with('#') && facts.names_prop(name) {
            return self.unk(Tag::Unknown, "", span);
        }
        for w in &facts.this_writes {
            if w.class != class.span.start && w.class != u32::MAX {
                continue;
            }
            match (&w.name, w.value) {
                (Some(n), Some(e)) if &**n == name => inits.push(e),
                (Some(n), None) if &**n == name => return self.unk(Tag::Unknown, "", span),
                (None, _) => return self.unk(Tag::Unknown, "", span),
                _ => {}
            }
        }
        if (inits.is_empty() && params.is_empty()) || inits.len() > 32 {
            // Never assigned in the class (injected by a decorator or a framework), or too
            // many assignments to join.
            return self.unk(Tag::Unknown, "", span);
        }
        // Other activations: the frames of inlined calls do not apply.
        let frames = std::mem::take(&mut self.frames);
        let mut vals = Vec::new();
        for e in inits {
            vals.push(self.eval(e));
        }
        for s in params {
            vals.push(self.symbol(s));
        }
        self.frames = frames;
        join(vals).shared()
    }
}

/// A container literal, or a chain of `.add` / `.set` / `.push`-style calls on
/// one.
fn is_fresh_container(e: &Expression<'_>, depth: u32) -> bool {
    if is_container_literal(e) {
        return true;
    }
    match e {
        Expression::CallExpression(c) if depth < 8 => match c.callee.get_inner_expression() {
            Expression::StaticMemberExpression(m) => {
                matches!(m.property.name.as_str(), "add" | "set" | "fill")
                    && is_fresh_container(m.object.get_inner_expression(), depth + 1)
            }
            _ => false,
        },
        _ => false,
    }
}

fn key_is(key: &PropertyKey<'_>, name: &str) -> bool {
    match key {
        PropertyKey::StaticIdentifier(id) => id.name == name,
        PropertyKey::PrivateIdentifier(id) => name.strip_prefix('#') == Some(id.name.as_str()),
        PropertyKey::StringLiteral(s) => s.value.as_str() == Some(name),
        _ => false,
    }
}

fn is_container_literal(e: &Expression<'_>) -> bool {
    match e {
        Expression::ObjectExpression(_) | Expression::ArrayExpression(_) => true,
        Expression::NewExpression(n) => matches!(
            n.callee.get_inner_expression(),
            Expression::Identifier(id) if matches!(id.name.as_str(), "Set" | "Map" | "Array")
        ),
        // `Object.freeze({...})`
        Expression::CallExpression(c) => {
            matches!(
                c.callee.get_inner_expression(),
                Expression::StaticMemberExpression(m)
                    if m.property.name == "freeze"
                        && matches!(&m.object, Expression::Identifier(i) if i.name == "Object")
            ) && c
                .arguments
                .first()
                .and_then(|a| a.as_expression())
                .is_some_and(|a| is_container_literal(a.get_inner_expression()))
        }
        _ => false,
    }
}

impl<'a> Engine<'a, '_> {
    /// Whether `id` names a global or a module binding (an import, a `require`
    /// result), not a value made in this file: only then does its name say
    /// what it is.
    fn builtin_like(&self, id: &IdentifierReference<'a>) -> bool {
        let Some(sym) = id
            .reference_id
            .get()
            .and_then(|r| self.scoping.get_reference(r).symbol_id())
        else {
            return true;
        };
        match self.nodes.kind(self.scoping.symbol_declaration(sym)) {
            AstKind::ImportSpecifier(_)
            | AstKind::ImportDefaultSpecifier(_)
            | AstKind::ImportNamespaceSpecifier(_)
            | AstKind::TSImportEqualsDeclaration(_) => true,
            AstKind::VariableDeclarator(d) => {
                let mut e = d.init.as_ref().map(|e| e.get_inner_expression());
                while let Some(Expression::StaticMemberExpression(m)) = e {
                    e = Some(m.object.get_inner_expression());
                }
                matches!(e, Some(Expression::CallExpression(c))
                    if matches!(c.callee.get_inner_expression(), Expression::Identifier(r) if r.name == "require"))
            }
            _ => false,
        }
    }

    /// Whether the callee of `c` is known not to keep or change its argument
    /// number `index`: `Object.*` readers, `JSON.stringify`, `Array.from`,
    /// `console.*`, `path.*`, `String`, `expect(...)` chains,
    /// `.each(table)`, and the iteration helpers of promises.
    pub(super) fn pure_callee(&self, c: &CallExpression<'a>, index: usize) -> bool {
        match c.callee.get_inner_expression() {
            Expression::Identifier(id) => {
                matches!(
                    id.name.as_str(),
                    "String"
                        | "Boolean"
                        | "Number"
                        | "expect"
                        | "structuredClone"
                        | "join"
                        | "resolve"
                ) && self.builtin_like(id)
            }
            Expression::StaticMemberExpression(m) => self.pure_member_callee(m, index),
            _ => false,
        }
    }

    fn pure_member_callee(&self, m: &StaticMemberExpression<'a>, index: usize) -> bool {
        {
            let method = m.property.name.as_str();
            match m.object.get_inner_expression() {
                Expression::Identifier(o) if !self.builtin_like(o) => method == "each",
                Expression::Identifier(o) => match o.name.as_str() {
                    "Object" => {
                        matches!(
                            method,
                            "keys" | "values" | "entries" | "freeze" | "fromEntries"
                        ) || (method == "assign" && index > 0)
                    }
                    "JSON" | "console" | "path" | "Math" | "util" => true,
                    "Array" => matches!(method, "from" | "isArray"),
                    "Promise" => matches!(method, "all" | "allSettled" | "race" | "any"),
                    _ => method == "each",
                },
                // `expect(x).toEqual(binding)`
                Expression::CallExpression(inner) => {
                    matches!(inner.callee.get_inner_expression(), Expression::Identifier(i) if i.name == "expect")
                        || method == "each"
                }
                Expression::StaticMemberExpression(_) => method == "each",
                _ => false,
            }
        }
    }
}

/// Whether property `name` of the shared object `o` may have been written by
/// code the engine does not attribute to a tracked binding.
pub(super) fn written(o: &ObjV<'_>, name: &str, facts: &Facts<'_>) -> bool {
    o.trust == Trust::Shared && (facts.writes_prop(name) || (o.map && facts.map_mutation()))
}

/// Whether some property of the shared object `o` may have been written (see
/// [`written`]).
pub(super) fn any_written(o: &ObjV<'_>, facts: &Facts<'_>) -> bool {
    o.trust == Trust::Shared && (facts.any_prop_write() || (o.map && facts.map_mutation()))
}
