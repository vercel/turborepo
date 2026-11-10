//! Guards: what the tests around a use of a binding say about its value.
//!
//! The engine is flow-insensitive: a binding is the join of its definitions,
//! and a parameter of a function that escapes is unknown. A load of such a
//! value is open (`computed_specifier`). Code often checks the value first:
//!
//! ```js
//! function load(name) {
//!   if (!name.startsWith('./handlers/')) throw new Error('outside');
//!   return require(name);          // ./handlers/*
//! }
//! const ALLOWED = ['./a', './b'];
//! function pick(name) {
//!   if (ALLOWED.includes(name)) return require(name);   // ./a | ./b
//! }
//! ```
//!
//! A guard is recognized by what it does, never by a name: a test on the
//! binding that holds wherever the use runs. The unknown part of the binding's
//! value at that use is then replaced by what the test allows: a pattern (the
//! prefix and anything after it) or the members of the list. Everything else
//! about the site is unchanged: the pattern is lowered, globbed and capped like
//! a template with a literal prefix ([`crate::extract::evaluator::lower`]), so
//! a prefix that names too many files stays open.
//!
//! # What is a test
//!
//! With `x` the binding ([`Cover::atom`]):
//!
//! | Test | Says |
//! |---|---|
//! | `x.startsWith(p)` (one argument) | `x` is `p` followed by anything, for every value `p` can have |
//! | `list.includes(x)`, `set.has(x)` where the receiver is an array or `Set` of the file | `x` is one of its elements |
//! | `/^literal/.test(x)`, `/^(a\|b)\/…/.test(x)` (a regular expression literal, no `i` / `m` / `g` / `y` flag) | `x` starts with the literal start of the expression |
//!
//! combined with `!`, `&&` and `||`. Anything else says nothing.
//!
//! # Where a test holds
//!
//! At a use of `x` ([`Engine::guarded`]), going outward:
//!
//! - inside the consequent of `if (test)` / `test ? … : …`, and to the right of
//!   `test &&`: the test is true; inside the alternate, and to the right of
//!   `test ||`: it is false;
//! - after `if (test) <exit>` earlier in the same statement list, or in a list
//!   around it: the test is false (`<exit>` is `throw`, `return`, `continue` or
//!   `break`, or a block that ends in one); after `if (test) … else <exit>`: it
//!   is true.
//!
//! # Soundness
//!
//! - The binding holds one value for as long as it lives (a parameter or a
//!   `const`, never assigned; a parameter of a function that uses `arguments`
//!   can be assigned through it). So a test that held once holds whenever the
//!   use runs, also inside a function created behind the test.
//! - A function *declaration* is callable before the statements in front of it
//!   run, so an exit in front of a declaration says nothing about the uses
//!   inside it. Arrow functions, function expressions, methods and classes
//!   exist only once control reached them.
//! - Lists of `switch` cases are not followed (control enters them at any
//!   label).
//! - The receiver of `startsWith` is a string (or a `String` object), and a
//!   value used as a specifier converts to the same string each time: an object
//!   that defines its own `startsWith` is not considered. A wildcard after a
//!   known directory names something below it (the existing rule for partial
//!   specifiers).
//! - Only the unknown leaves of the value are replaced; known alternatives
//!   stay.
//! - A cover that is not at least a pattern with a literal start (the prefix is
//!   unknown, the list was changed by code that is not evaluated) is dropped:
//!   the use stays as it was.
//!
//! Symbolic evaluation (cross-file facts, [`super::sym`]) is not narrowed:
//! there the parameter stays a hole that callers fill.

use std::collections::HashMap;

use oxc_ast::{AstKind, ast::*};
use oxc_semantic::{NodeId, SymbolId};
use oxc_span::{GetSpan, Span};
use turborepo_trace_core::eval::{PatPart, Value};

use super::{
    engine::Engine,
    value::{V, join},
};
use crate::extract::{evaluator::JsVal, origin::Tag};

/// Maximum literal starts taken from one regular expression.
const MAX_REGEX_STARTS: usize = 16;

/// The guard state of one file.
#[derive(Default)]
pub(super) struct Guards<'a> {
    /// Whether the file can contain a test at all (a cheap text check), once
    /// known.
    possible: Option<bool>,
    /// A cover is being evaluated: uses inside it are not narrowed (no
    /// recursion).
    busy: bool,
    /// The cover at each use looked at (by the node of the reference).
    covers: HashMap<NodeId, Option<V<'a>>>,
    /// Whether a symbol is a fixed binding.
    fixed: HashMap<SymbolId, bool>,
}

/// What is known about a binding from tests: `None` is nothing, `Some(v)` says
/// its value is one of the strings `v` covers.
type Cover<'a> = Option<V<'a>>;

/// Both hold: the tighter one (exact values before patterns), the first when
/// equal.
fn both<'a>(a: Cover<'a>, b: Cover<'a>) -> Cover<'a> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if !exact(&a) && exact(&b) { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// One of the two holds.
fn either<'a>(a: Cover<'a>, b: Cover<'a>) -> Cover<'a> {
    Some(join([a?, b?]))
}

/// Whether every leaf is a known string.
fn exact(v: &V<'_>) -> bool {
    v.leaves().iter().all(|l| {
        matches!(
            l,
            V::Js(JsVal::Str(Value::Str(_)) | JsVal::Path(_, Value::Str(_)))
        )
    })
}

/// Whether `v` bounds anything: every leaf is a known string, a pattern with a
/// literal start, or a path below an anchor.
fn bounded(v: &V<'_>) -> bool {
    let starts = |v: &Value| match v {
        Value::Str(s) => !s.is_empty(),
        Value::Pattern(p) => matches!(p.first(), Some(PatPart::Lit(l)) if !l.is_empty()),
        _ => false,
    };
    let leaves = v.leaves();
    !leaves.is_empty()
        && leaves.iter().all(|l| match l {
            V::Js(JsVal::Str(v) | JsVal::Path(_, v) | JsVal::Url(_, v)) => starts(v),
            _ => false,
        })
}

/// Whether the leaf is an unknown string-like value (not a number or boolean).
fn open_leaf(l: &V<'_>) -> bool {
    matches!(l, V::Js(JsVal::Unknown(_))) && !l.is_scalar()
}

/// Whether the statement always leaves the statement list it is in.
fn exits(s: &Statement<'_>) -> bool {
    match s {
        Statement::ThrowStatement(_)
        | Statement::ReturnStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::BreakStatement(_) => true,
        Statement::BlockStatement(b) => b.body.last().is_some_and(exits),
        _ => false,
    }
}

fn within(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl<'a> Engine<'a, '_> {
    /// The value `v` of the binding `sym` at the use `id`, with its unknown
    /// leaves replaced by what the tests around the use allow.
    pub(super) fn guarded(
        &mut self,
        id: &'a IdentifierReference<'a>,
        sym: SymbolId,
        v: V<'a>,
    ) -> V<'a> {
        if self.guards.busy || self.sym.is_some() || !v.leaves().iter().any(open_leaf) {
            return v;
        }
        let src = self.src;
        let possible = *self.guards.possible.get_or_insert_with(|| {
            ["startsWith(", ".includes(", ".has(", ".test("]
                .iter()
                .any(|t| src.contains(t))
        });
        if !possible {
            return v;
        }
        let Some(node) = id
            .reference_id
            .get()
            .map(|r| self.scoping.get_reference(r).node_id())
        else {
            return v;
        };
        let cover = match self.guards.covers.get(&node) {
            Some(c) => c.clone(),
            None => {
                let c = if self.fixed_binding(sym) {
                    self.guards.busy = true;
                    let c = self.cover_at(node, sym, id.span).filter(bounded);
                    self.guards.busy = false;
                    c
                } else {
                    None
                };
                self.guards.covers.insert(node, c.clone());
                c
            }
        };
        let Some(cover) = cover else {
            return v;
        };
        join(
            v.into_leaves()
                .into_iter()
                .filter(|l| !open_leaf(l))
                .chain([cover]),
        )
    }

    /// Whether the binding `sym` holds one value for as long as it lives: a
    /// parameter or a `const`, declared once and never assigned.
    fn fixed_binding(&mut self, sym: SymbolId) -> bool {
        if let Some(&f) = self.guards.fixed.get(&sym) {
            return f;
        }
        let f = self.fixed_binding_uncached(sym);
        self.guards.fixed.insert(sym, f);
        f
    }

    fn fixed_binding_uncached(&mut self, sym: SymbolId) -> bool {
        let scoping = self.scoping;
        if !scoping.symbol_redeclarations(sym).is_empty()
            || scoping.get_resolved_references(sym).any(|r| r.is_write())
        {
            return false;
        }
        let nodes = self.nodes;
        let decl = scoping.symbol_declaration(sym);
        match nodes.kind(decl) {
            AstKind::VariableDeclarator(_) => matches!(
                nodes.parent_kind(decl),
                AstKind::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Const
            ),
            AstKind::FormalParameter(_) => {
                for k in nodes.ancestor_kinds(decl) {
                    match k {
                        // An arrow function has no `arguments` of its own.
                        AstKind::ArrowFunctionExpression(_) => return true,
                        AstKind::Function(f) => {
                            return !self.facts().uses_arguments(f.span.start);
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// What the tests around the use at `node` say about `sym`.
    fn cover_at(&mut self, node: NodeId, sym: SymbolId, name: Span) -> Cover<'a> {
        let nodes = self.nodes;
        let mut acc: Cover<'a> = None;
        let mut cur = node;
        loop {
            let p = nodes.parent_id(cur);
            if p == cur {
                break;
            }
            let at = nodes.kind(cur).span();
            match nodes.kind(p) {
                AstKind::IfStatement(i) => {
                    if within(i.consequent.span(), at) {
                        acc = both(acc, self.holds(&i.test, sym, name, true));
                    } else if i.alternate.as_ref().is_some_and(|a| within(a.span(), at)) {
                        acc = both(acc, self.holds(&i.test, sym, name, false));
                    }
                }
                AstKind::ConditionalExpression(c) => {
                    if within(c.consequent.span(), at) {
                        acc = both(acc, self.holds(&c.test, sym, name, true));
                    } else if within(c.alternate.span(), at) {
                        acc = both(acc, self.holds(&c.test, sym, name, false));
                    }
                }
                AstKind::LogicalExpression(l) if within(l.right.span(), at) => match l.operator {
                    LogicalOperator::And => {
                        acc = both(acc, self.holds(&l.left, sym, name, true));
                    }
                    LogicalOperator::Or => {
                        acc = both(acc, self.holds(&l.left, sym, name, false));
                    }
                    LogicalOperator::Coalesce => {}
                },
                AstKind::BlockStatement(b) => {
                    acc = both(acc, self.before(&b.body, at, sym, name));
                }
                AstKind::FunctionBody(b) => {
                    acc = both(acc, self.before(&b.statements, at, sym, name));
                }
                AstKind::Program(p) => {
                    acc = both(acc, self.before(&p.body, at, sym, name));
                }
                _ => {}
            }
            cur = p;
        }
        acc
    }

    /// What the statements of `list` in front of the one that holds the use (at
    /// `at`) say: an `if` that leaves the list makes its test false from
    /// there on.
    fn before(
        &mut self,
        list: &'a [Statement<'a>],
        at: Span,
        sym: SymbolId,
        name: Span,
    ) -> Cover<'a> {
        let i = list.iter().position(|s| within(s.span(), at))?;
        // A function declaration can run before the statements in front of it. An
        // `export`ed one is a declaration all the same.
        let hoisted = match &list[i] {
            Statement::FunctionDeclaration(_) => true,
            Statement::ExportDeclaration(e) => {
                matches!(e.declaration, Declaration::FunctionDeclaration(_))
            }
            Statement::ExportDefaultDeclaration(e) => matches!(
                e.declaration,
                ExportDefaultDeclarationKind::FunctionDeclaration(_)
            ),
            _ => false,
        };
        if hoisted {
            return None;
        }
        let mut acc: Cover<'a> = None;
        for s in &list[..i] {
            let Statement::IfStatement(s) = s else {
                continue;
            };
            if exits(&s.consequent) {
                acc = both(acc, self.holds(&s.test, sym, name, false));
            } else if s.alternate.as_ref().is_some_and(exits) {
                acc = both(acc, self.holds(&s.test, sym, name, true));
            }
        }
        acc
    }

    /// What `e` being `truth` says about `sym`.
    fn holds(
        &mut self,
        e: &'a Expression<'a>,
        sym: SymbolId,
        name: Span,
        truth: bool,
    ) -> Cover<'a> {
        if !self.tick() {
            return None;
        }
        match e.get_inner_expression() {
            Expression::UnaryExpression(u) if u.operator == UnaryOperator::LogicalNot => {
                self.holds(&u.argument, sym, name, !truth)
            }
            Expression::LogicalExpression(l) => {
                let and = match l.operator {
                    LogicalOperator::And => true,
                    LogicalOperator::Or => false,
                    LogicalOperator::Coalesce => return None,
                };
                let a = self.holds(&l.left, sym, name, truth);
                let b = self.holds(&l.right, sym, name, truth);
                // `a && b` true: both; false: one of them. `a || b` the other way round.
                if and == truth {
                    both(a, b)
                } else {
                    either(a, b)
                }
            }
            Expression::CallExpression(c) if truth => self.atom(c, sym, name),
            Expression::ChainExpression(c) if truth => match &c.expression {
                // `x?.startsWith(p)` is true only when the call was made and returned true.
                ChainElement::CallExpression(c) => self.atom(c, sym, name),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether `e` is a plain use of `sym`.
    fn is_sym(&self, e: &Expression<'a>, sym: SymbolId) -> bool {
        matches!(
            e.get_inner_expression(),
            Expression::Identifier(i)
                if i.reference_id
                    .get()
                    .and_then(|r| self.scoping.get_reference(r).symbol_id())
                    == Some(sym)
        )
    }

    /// What the call `c` returning true says about `sym` (see the module
    /// documentation).
    fn atom(&mut self, c: &'a CallExpression<'a>, sym: SymbolId, name: Span) -> Cover<'a> {
        let [arg] = c.arguments.as_slice() else {
            return None;
        };
        let arg = arg.as_expression()?;
        let Expression::StaticMemberExpression(m) = c.callee.get_inner_expression() else {
            return None;
        };
        match m.property.name.as_str() {
            "startsWith" if self.is_sym(&m.object, sym) => {
                let p = self.eval(arg);
                let rest = self.unk(Tag::Unknown, "", name);
                Some(self.concat(p, rest, c.span))
            }
            "includes" | "has" if self.is_sym(arg, sym) => {
                let recv = self.eval(&m.object);
                // An array or a `Set` of the file. (`includes` of a string is a substring
                // test, and an object can define either method.)
                if !recv.leaves().iter().all(|l| matches!(l, V::Array(_))) {
                    return None;
                }
                let v = self.elements(recv, c.span);
                exact(&v).then_some(v)
            }
            "test" if self.is_sym(arg, sym) => {
                let Expression::RegExpLiteral(r) = m.object.get_inner_expression() else {
                    return None;
                };
                let text = self.src.get(r.span.start as usize..r.span.end as usize)?;
                let starts = regex_starts(text)?;
                let rest = self.unk(Tag::Unknown, "", name);
                let mut out = Vec::with_capacity(starts.len());
                for s in starts {
                    out.push(self.concat(V::str(s), rest.clone(), c.span));
                }
                Some(join(out))
            }
            _ => None,
        }
    }
}

/// The literal starts of the strings the regular expression literal `text`
/// (`/…/flags`) matches: every match begins with one of them. `None` when the
/// expression is not anchored at the start, has a flag that changes what `^` or
/// a letter means, or its start is not a literal.
pub(super) fn regex_starts(text: &str) -> Option<Vec<String>> {
    let end = text.rfind('/')?;
    let (pattern, flags) = (text.get(1..end)?, &text[end + 1..]);
    // `i`: other spellings match; `m`: `^` matches after any line break; `g` / `y`:
    // the match starts at `lastIndex`.
    if !flags.chars().all(|f| matches!(f, 'u' | 's' | 'd')) {
        return None;
    }
    let chars: Vec<char> = pattern.chars().collect();
    if chars.first() != Some(&'^') {
        return None;
    }
    let quantified = |i: usize| matches!(chars.get(i), Some('?' | '*' | '+' | '{'));
    // A literal character at `i`: the character and where the next one starts.
    let literal = |i: usize| -> Option<(char, usize)> {
        match *chars.get(i)? {
            '\\' => {
                let c = *chars.get(i + 1)?;
                // An escaped punctuation character is itself; a letter or digit is a class,
                // a back reference or a control character.
                (!c.is_alphanumeric() && c != '_').then_some((c, i + 2))
            }
            '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/' => {
                None
            }
            c => Some((c, i + 1)),
        }
    };
    let mut starts: Vec<String> = vec![String::new()];
    let mut i = 1;
    loop {
        if let Some((c, next)) = literal(i) {
            if quantified(next) {
                break;
            }
            for s in &mut starts {
                s.push(c);
            }
            i = next;
            continue;
        }
        if chars.get(i) != Some(&'(') {
            break;
        }
        // A group of literal alternatives: `(a|b)` or `(?:a|b)`.
        let mut j = i + 1;
        if chars.get(j) == Some(&'?') {
            if chars.get(j + 1) != Some(&':') {
                break;
            }
            j += 2;
        }
        let mut alts: Vec<String> = vec![String::new()];
        let closed = loop {
            match chars.get(j) {
                Some(')') => break true,
                Some('|') => {
                    alts.push(String::new());
                    j += 1;
                }
                _ => match literal(j) {
                    Some((c, next)) if !quantified(next) => {
                        if let Some(a) = alts.last_mut() {
                            a.push(c);
                        }
                        j = next;
                    }
                    _ => break false,
                },
            }
        };
        if !closed || quantified(j + 1) || starts.len() * alts.len() > MAX_REGEX_STARTS {
            break;
        }
        starts = starts
            .iter()
            .flat_map(|s| alts.iter().map(move |a| format!("{s}{a}")))
            .collect();
        i = j + 1;
    }
    // An alternation at the top level (`^a|b`) has branches the start does not
    // cover.
    let (mut depth, mut class, mut k) = (0usize, false, i);
    while let Some(&c) = chars.get(k) {
        match c {
            '\\' => k += 1,
            '[' if !class => class = true,
            ']' if class => class = false,
            '(' if !class => depth += 1,
            ')' if !class => depth = depth.saturating_sub(1),
            '|' if !class && depth == 0 => return None,
            _ => {}
        }
        k += 1;
    }
    starts.iter().all(|s| !s.is_empty()).then_some(starts)
}

#[cfg(test)]
mod tests {
    use super::regex_starts;

    fn starts(text: &str) -> Option<Vec<String>> {
        regex_starts(text)
    }

    #[test]
    fn a_literal_start_of_a_regular_expression() {
        let one = |s: &str| Some(vec![s.to_owned()]);
        assert_eq!(starts(r"/^\.\/handlers\//"), one("./handlers/"));
        assert_eq!(starts(r"/^services\/[^/]+\/src\//u"), one("services/"));
        assert_eq!(starts(r"/^abc$/"), one("abc"));
        // The quantified character is not part of the start.
        assert_eq!(starts(r"/^abc?d/"), one("ab"));
        assert_eq!(starts(r"/^ab+/"), one("a"));
        assert_eq!(
            starts(r"/^(services|apps)\/x/"),
            Some(vec!["services/x".to_owned(), "apps/x".to_owned()])
        );
        assert_eq!(
            starts(r"/^(?:a|b)-(c|d)/"),
            Some(vec![
                "a-c".to_owned(),
                "a-d".to_owned(),
                "b-c".to_owned(),
                "b-d".to_owned()
            ])
        );
        // A group that is not literal ends the start in front of it.
        assert_eq!(starts(r"/^lib\/(\w+)\//"), one("lib/"));
        assert_eq!(starts(r"/^lib\/(a|b)?x/"), one("lib/"));
    }

    #[test]
    fn expressions_without_a_literal_start() {
        // Not anchored, or anchored only in one branch.
        assert_eq!(starts(r"/handlers\//"), None);
        assert_eq!(starts(r"/^a|b/"), None);
        assert_eq!(starts(r"/^ab(c|d)e|f/"), None);
        // An alternation inside a group or a class is not one at the top level.
        assert_eq!(starts(r"/^ab(c|d)+[|]/"), Some(vec!["ab".to_owned()]));
        // Flags that change the meaning.
        assert_eq!(starts(r"/^abc/i"), None);
        assert_eq!(starts(r"/^abc/m"), None);
        assert_eq!(starts(r"/^abc/g"), None);
        assert_eq!(starts(r"/^abc/y"), None);
        // Nothing literal after the anchor.
        assert_eq!(starts(r"/^\w+\/x/"), None);
        assert_eq!(starts(r"/^(a|)/"), None);
        assert_eq!(starts(r"/^a?/"), None);
        assert_eq!(starts(r"/^/"), None);
    }
}
