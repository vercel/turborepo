//! Turbopack chunks as sets of modules with canonical identifiers.
//!
//! A production chunk is one statement holding a flat list of module ids and
//! factories:
//!
//! ```js
//! module.exports=[34468,a=>{...},47560,(a,b,c)=>{...}]                    // server
//! (globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push([script,34468,a=>{...}])   // client
//! ```
//!
//! An element of the list can be a list of the same shape (a group of modules).
//! Tools wrap the chunk (Sentry puts a debug-id statement in front and the
//! chunk into an immediately invoked function), so the list is looked for
//! inside such wrappers too.
//!
//! [`split`] returns one [`ChunkModule`] per module: its id and the canonical
//! text of its factory. Canonical means that what differs between two builds of
//! the same source is gone:
//!
//! | In the chunk | In the canonical text |
//! |---|---|
//! | identifiers bound inside the chunk (the minifier picks them per chunk) | `$<n>`, by order of first appearance in the factory |
//! | integer arguments of calls on a factory's context parameter (`a.i(123)`, `a.s([...], 123)`: module ids) | marked, so that [`resolve`] can replace the ones that are module ids |
//! | the export list `x.s([name, getter, ...])` | its entries sorted by name: the module that lists an entry's server actions sorts them by action id, which is salted per build |
//! | the re-export list `x.S([123, "name,local,name,local", 0, 456, "..."])` | the integer before a string is marked as a module id; the pairs of a string are sorted by name, for the same reason |
//! | a factory's leading `"use strict"` directive | dropped: a module has it when it stands alone and not inside a group whose function has it |
//! | a list of chunk files (`["static/chunks/a.js","static/chunks/b.css"]`, what a lazy import loads) | one placeholder: which chunks, and how many, is how the bundler grouped the modules; the files count through the closure of the route |
//!
//! A group of modules can also be a function called where it is written, as an
//! element of the list: `(()=>{"use strict";return[1,a=>{..},2,b=>{..}]})()`.
//! Its modules are read like those of a list; the statements before the
//! `return` (declarations the modules share) are a text of their own.
//!
//! Other numbers stay as they are: a constant that happens to equal a module id
//! must not tie a module to an unrelated one. Property names, globals and
//! strings stay too. A shorthand property (`{a}`) is written as `a:<n>`, the
//! same as its expanded form, because its key is not renamed with the local.
//!
//! Why module ids are marked: Turbopack derives an id from what identifies the
//! module, and that is not the module's content. A generated module (the proxy
//! of a server action, `createServerReference("<action id>")`) is identified by
//! text that holds an id salted per build; an alias module by a path that can
//! be absolute; a module of a dependency by its path in `node_modules`, which
//! with pnpm changes when an unrelated package is bumped. An id then differs
//! between two builds although the module is the same, and so does every
//! `a.i(<id>)` in its importers. The caller puts a hash of the target's text
//! where the id was ([`resolve`]).
//!
//! Needs the `oxc` feature (parser and scope analysis). Without it
//! [`AVAILABLE`] is false, [`split`] returns `None` and callers hash the chunk
//! as a whole.

/// Whether chunks can be split (the `oxc` feature).
pub const AVAILABLE: bool = cfg!(feature = "oxc");

/// Marks around an integer literal in a canonical text.
const NUM_OPEN: char = '\u{1}';
const NUM_CLOSE: char = '\u{2}';

/// Whether `text` may be a Turbopack production chunk (the list is near the
/// start; a wrapper may precede it).
pub fn is_turbopack_chunk(text: &str) -> bool {
    let mut n = text.len().min(4096);
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    let head = &text[..n];
    head.contains("module.exports=[") || head.contains("(globalThis.TURBOPACK")
}

/// One module of a chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkModule {
    /// The id as written (a number; a quoted string in development builds).
    pub id: String,
    /// The canonical text of the factory, with marked integer literals
    /// ([`resolve`]).
    pub text: String,
}

/// A chunk as modules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chunk {
    /// The modules, in list order.
    pub modules: Vec<ChunkModule>,
    /// The canonical texts of what is not a module: list elements without an
    /// id, and the code around the list (wrappers, the runtime of an entry
    /// chunk).
    pub other: Vec<String>,
}

/// `text` with every marked integer replaced: by `<...>` around what `module`
/// returns for it (it is a module id), or by its digits.
pub fn resolve<'a>(text: &str, module: impl Fn(&str) -> Option<&'a str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(NUM_OPEN) {
        out.push_str(&rest[..i]);
        let after = &rest[i + NUM_OPEN.len_utf8()..];
        let Some(j) = after.find(NUM_CLOSE) else {
            out.push_str(after);
            return out;
        };
        let digits = &after[..j];
        match module(digits) {
            Some(h) => {
                out.push('<');
                out.push_str(h);
                out.push('>');
            }
            None => out.push_str(digits),
        }
        rest = &after[j + NUM_CLOSE.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// The marked integers of a canonical text, in order.
pub fn integers(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(NUM_OPEN) {
        let after = &rest[i + NUM_OPEN.len_utf8()..];
        let Some(j) = after.find(NUM_CLOSE) else {
            break;
        };
        out.push(&after[..j]);
        rest = &after[j + NUM_CLOSE.len_utf8()..];
    }
    out
}

/// The modules of a chunk. `None` if the text does not parse as a chunk.
#[cfg(not(feature = "oxc"))]
pub fn split(_text: &str) -> Option<Chunk> {
    None
}

#[cfg(feature = "oxc")]
pub use imp::split;

#[cfg(feature = "oxc")]
mod imp {
    use std::collections::{HashMap, HashSet};

    use oxc_allocator::Allocator;
    use oxc_ast::ast::{
        Argument, ArrayExpression, ArrayExpressionElement, ArrowFunctionBody,
        AssignmentTargetPropertyIdentifier, BindingIdentifier, BindingProperty, CallExpression,
        Expression, IdentifierReference, ObjectProperty, Statement,
    };
    use oxc_ast_visit::{Visit, walk};
    use oxc_parser::Parser;
    use oxc_semantic::{Scoping, SemanticBuilder, SymbolId};
    use oxc_span::{GetSpan, SourceType};

    use super::{Chunk, ChunkModule, NUM_CLOSE, NUM_OPEN};

    /// An identifier occurrence bound to a symbol of the chunk.
    struct Occ {
        start: u32,
        end: u32,
        symbol: SymbolId,
    }

    /// An export list whose entries are compared in sorted order.
    struct ExportList {
        /// The array literal.
        start: u32,
        end: u32,
        /// The entries: `(start, end, name start, name end)`.
        entries: Vec<(u32, u32, u32, u32)>,
    }

    struct Collect<'s> {
        text: &'s str,
        scoping: &'s Scoping,
        occs: Vec<Occ>,
        /// Start offsets of identifiers that are also a property key
        /// (shorthand).
        shorthand: HashSet<u32>,
        /// Integer arguments of calls on a factory's context parameter (filled
        /// after the walk, from `calls`).
        nums: Vec<(u32, u32)>,
        /// Calls `x.y(..)` with integer arguments: the start of `x`, the
        /// arguments.
        calls: Vec<(u32, Vec<(u32, u32)>)>,
        lists: Vec<ExportList>,
        /// Text written in place of a range: a re-export string with its pairs
        /// sorted, a list of chunk names as one placeholder.
        subs: Vec<(u32, u32, String)>,
    }

    /// A string literal of `name,local` pairs (what `x.S([id,
    /// "name,local,..."])` re-exports from a module) with the pairs in
    /// sorted order: the bundler sorts them by name, and the
    /// name of a server action is its id, salted per build. `None` if the
    /// literal is not such a list (an odd number of fields, fewer than two
    /// pairs, an escape).
    fn sorted_pairs(lit: &str) -> Option<String> {
        let quote = lit.chars().next()?;
        if !(quote == '"' || quote == '\'')
            || lit.len() < 2
            || !lit.ends_with(quote)
            || lit.contains('\\')
        {
            return None;
        }
        let fields: Vec<&str> = lit[1..lit.len() - 1].split(',').collect();
        if fields.len() < 4 || !fields.len().is_multiple_of(2) {
            return None;
        }
        let mut pairs: Vec<(&str, &str)> = fields.chunks(2).map(|p| (p[0], p[1])).collect();
        pairs.sort_unstable();
        let mut out = String::with_capacity(lit.len());
        out.push(quote);
        for (i, (name, local)) in pairs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(name);
            out.push(',');
            out.push_str(local);
        }
        out.push(quote);
        Some(out)
    }

    impl<'a> Visit<'a> for Collect<'_> {
        fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
            if let Some(symbol) = it.symbol_id.get() {
                self.occs.push(Occ {
                    start: it.span.start,
                    end: it.span.end,
                    symbol,
                });
            }
        }

        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            if let Some(symbol) = it
                .reference_id
                .get()
                .and_then(|r| self.scoping.get_reference(r).symbol_id())
            {
                self.occs.push(Occ {
                    start: it.span.start,
                    end: it.span.end,
                    symbol,
                });
            }
        }

        fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
            if it.shorthand {
                self.shorthand.insert(it.key.span().start);
            }
            walk::walk_object_property(self, it);
        }

        fn visit_binding_property(&mut self, it: &BindingProperty<'a>) {
            if it.shorthand {
                self.shorthand.insert(it.key.span().start);
            }
            walk::walk_binding_property(self, it);
        }

        fn visit_assignment_target_property_identifier(
            &mut self,
            it: &AssignmentTargetPropertyIdentifier<'a>,
        ) {
            self.shorthand.insert(it.binding.span.start);
            walk::walk_assignment_target_property_identifier(self, it);
        }

        fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
            // `x.y(123, ...)`: a module id if `x` is a factory's context parameter.
            if let Expression::StaticMemberExpression(m) = &it.callee
                && let Expression::Identifier(object) = &m.object
            {
                let ints: Vec<(u32, u32)> = it
                    .arguments
                    .iter()
                    .filter_map(|a| match a {
                        Argument::NumericLiteral(n) => Some((n.span.start, n.span.end)),
                        _ => None,
                    })
                    .filter(|(a, b)| {
                        self.text[*a as usize..*b as usize]
                            .bytes()
                            .all(|c| c.is_ascii_digit())
                    })
                    .collect();
                if !ints.is_empty() {
                    self.calls.push((object.span.start, ints));
                }
            }
            // `x.s([name, getter, name, 0, value, ...], id)`: the exports of a module.
            if let Expression::StaticMemberExpression(m) = &it.callee
                && m.property.name.as_str() == "s"
                && let Some(Argument::ArrayExpression(arr)) = it.arguments.first()
                && let Some(entries) = export_entries(arr)
            {
                self.lists.push(ExportList {
                    start: arr.span.start,
                    end: arr.span.end,
                    entries,
                });
            }
            // `x.S([123, "name,local,name,local", 0, 456, "..."])`: what a module
            // re-exports, per source module. An integer before a string is a
            // module id; the pairs of a string are compared in sorted order.
            if let Expression::StaticMemberExpression(m) = &it.callee
                && m.property.name.as_str() == "S"
                && let Expression::Identifier(object) = &m.object
                && let Some(Argument::ArrayExpression(arr)) = it.arguments.first()
            {
                let els = &arr.elements;
                let mut ints: Vec<(u32, u32)> = Vec::new();
                for (i, el) in els.iter().enumerate() {
                    match el {
                        ArrayExpressionElement::NumericLiteral(n)
                            if matches!(
                                els.get(i + 1),
                                Some(ArrayExpressionElement::StringLiteral(_))
                            ) && self.text[n.span.start as usize..n.span.end as usize]
                                .bytes()
                                .all(|c| c.is_ascii_digit()) =>
                        {
                            ints.push((n.span.start, n.span.end));
                        }
                        ArrayExpressionElement::StringLiteral(s) => {
                            if let Some(sorted) =
                                sorted_pairs(&self.text[s.span.start as usize..s.span.end as usize])
                            {
                                self.subs.push((s.span.start, s.span.end, sorted));
                            }
                        }
                        _ => {}
                    }
                }
                if !ints.is_empty() {
                    self.calls.push((object.span.start, ints));
                }
            }
            walk::walk_call_expression(self, it);
        }

        fn visit_array_expression(&mut self, it: &ArrayExpression<'a>) {
            // A list of chunk files (`["static/chunks/a.js","static/chunks/b.css"]`: what a
            // lazy import loads). Which chunks, and how many, is how the bundler grouped
            // the modules; the files count through the closure of the route
            // that loads them.
            if !it.elements.is_empty()
                && it.elements.iter().all(|e| match e {
                    ArrayExpressionElement::StringLiteral(s) => {
                        // As written, after the opening quote.
                        self.text[s.span.start as usize..s.span.end as usize]
                            .get(1..)
                            .is_some_and(|v| {
                                v.starts_with("static/chunks/") || v.starts_with("server/chunks/")
                            })
                    }
                    _ => false,
                })
            {
                self.subs.push((
                    it.span.start + 1,
                    it.span.end - 1,
                    "\"<CHUNKS>\"".to_string(),
                ));
            }
            walk::walk_array_expression(self, it);
        }
    }

    /// The entries of an export list: a name, an optional `0`, a value. `None`
    /// if the array has another shape or fewer than two entries.
    fn export_entries(arr: &ArrayExpression<'_>) -> Option<Vec<(u32, u32, u32, u32)>> {
        let els = &arr.elements;
        let mut out = Vec::new();
        let mut i = 0;
        while i < els.len() {
            let ArrayExpressionElement::StringLiteral(name) = &els[i] else {
                return None;
            };
            let mut j = i + 1;
            if matches!(els.get(j), Some(ArrayExpressionElement::NumericLiteral(_)))
                && j + 1 < els.len()
            {
                j += 1;
            }
            let value = els.get(j)?;
            if matches!(
                value,
                ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_)
            ) {
                return None;
            }
            out.push((
                name.span.start,
                value.span().end,
                name.span.start,
                name.span.end,
            ));
            i = j + 1;
        }
        (out.len() >= 2).then_some(out)
    }

    /// Writes canonical text.
    struct Emit<'t> {
        text: &'t str,
        occs: &'t [Occ],
        shorthand: &'t HashSet<u32>,
        nums: &'t [(u32, u32)],
        lists: &'t [ExportList],
        subs: &'t [(u32, u32, String)],
        /// Symbol → its number in this text.
        index: HashMap<SymbolId, usize>,
        out: String,
    }

    impl Emit<'_> {
        fn range(&mut self, start: u32, end: u32) {
            // The marks outlive `self`: copies of the references keep `self.out` free.
            let (text, occs, nums, lists, subs, shorthand) = (
                self.text,
                self.occs,
                self.nums,
                self.lists,
                self.subs,
                self.shorthand,
            );
            let mut at = start;
            let mut oi = occs.partition_point(|o| o.start < start);
            let mut ni = nums.partition_point(|n| n.0 < start);
            let mut li = lists.partition_point(|l| l.start < start);
            let mut si = subs.partition_point(|s| s.0 < start);
            loop {
                let o = occs.get(oi).filter(|o| o.end <= end).map(|o| o.start);
                let n = nums.get(ni).filter(|n| n.1 <= end).map(|n| n.0);
                let l = lists.get(li).filter(|l| l.end <= end).map(|l| l.start);
                let s = subs.get(si).filter(|s| s.1 <= end).map(|s| s.0);
                let Some(next) = [o, n, l, s].into_iter().flatten().min() else {
                    break;
                };
                if s == Some(next) && l != Some(next) && o != Some(next) && n != Some(next) {
                    let sub = &subs[si];
                    si += 1;
                    // Inside a range that was written already: once.
                    if sub.0 < at {
                        continue;
                    }
                    self.out.push_str(&text[at as usize..sub.0 as usize]);
                    self.out.push_str(&sub.2);
                    at = sub.1;
                    continue;
                }
                if l == Some(next) {
                    let list = &lists[li];
                    li += 1;
                    if next < at {
                        continue;
                    }
                    // Up to and including the `[`.
                    self.out
                        .push_str(&text[at as usize..list.start as usize + 1]);
                    let mut order: Vec<&(u32, u32, u32, u32)> = list.entries.iter().collect();
                    order.sort_by_key(|e| &text[e.2 as usize..e.3 as usize]);
                    for (k, e) in order.into_iter().enumerate() {
                        if k > 0 {
                            self.out.push(',');
                        }
                        self.range(e.0, e.1);
                    }
                    // The `]` is written with what follows.
                    at = list.end - 1;
                    continue;
                }
                if o == Some(next) {
                    let occ = &occs[oi];
                    oi += 1;
                    // Inside a list that was written, or overlapping records (a shorthand
                    // key and its value): once.
                    if occ.start < at {
                        continue;
                    }
                    self.out.push_str(&text[at as usize..occ.start as usize]);
                    let fresh = self.index.len();
                    let n = *self.index.entry(occ.symbol).or_insert(fresh);
                    if shorthand.contains(&occ.start) {
                        self.out
                            .push_str(&text[occ.start as usize..occ.end as usize]);
                        self.out.push(':');
                    }
                    self.out.push('$');
                    self.out.push_str(&n.to_string());
                    at = occ.end;
                    continue;
                }
                let num = nums[ni];
                ni += 1;
                if num.0 < at {
                    continue;
                }
                self.out.push_str(&text[at as usize..num.0 as usize]);
                self.out.push(NUM_OPEN);
                self.out.push_str(&text[num.0 as usize..num.1 as usize]);
                self.out.push(NUM_CLOSE);
                at = num.1;
            }
            if at < end {
                self.out.push_str(&text[at as usize..end as usize]);
            }
        }
    }

    /// The array of an assignment to `module.exports` or of a `.push(...)`
    /// call.
    fn list<'a, 'b>(text: &str, stmt: &'b Statement<'a>) -> Option<&'b ArrayExpression<'a>> {
        let Statement::ExpressionStatement(es) = stmt else {
            return None;
        };
        let src = |s: oxc_span::Span| &text[s.start as usize..s.end as usize];
        match &es.expression {
            Expression::AssignmentExpression(a) if src(a.left.span()) == "module.exports" => {
                match &a.right {
                    Expression::ArrayExpression(arr) => Some(arr),
                    _ => None,
                }
            }
            Expression::CallExpression(c) if src(c.callee.span()).ends_with(".push") => {
                match c.arguments.first()? {
                    Argument::ArrayExpression(arr) => Some(arr),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The statements of a function that is called where it is written:
    /// `(()=>{...})()`, `!function(){...}()`.
    fn called_body<'a, 'b>(stmt: &'b Statement<'a>) -> Option<&'b [Statement<'a>]> {
        let Statement::ExpressionStatement(es) = stmt else {
            return None;
        };
        let mut e = &es.expression;
        loop {
            match e {
                Expression::ParenthesizedExpression(p) => e = &p.expression,
                Expression::UnaryExpression(u) => e = &u.argument,
                _ => break,
            }
        }
        let Expression::CallExpression(c) = e else {
            return None;
        };
        let mut callee = &c.callee;
        while let Expression::ParenthesizedExpression(p) = callee {
            callee = &p.expression;
        }
        match callee {
            Expression::ArrowFunctionExpression(f) => match &f.body {
                ArrowFunctionBody::FunctionBody(b) => Some(&b.statements[..]),
                _ => None,
            },
            Expression::FunctionExpression(f) => f.body.as_ref().map(|b| &b.statements[..]),
            _ => None,
        }
    }

    /// The module list: a top-level statement, or one inside a wrapper
    /// function.
    fn find<'a, 'b>(
        text: &str,
        stmts: &'b [Statement<'a>],
        depth: u8,
    ) -> Option<&'b ArrayExpression<'a>> {
        for s in stmts {
            if let Some(arr) = list(text, s) {
                return Some(arr);
            }
            if depth < 3
                && let Some(body) = called_body(s)
                && let Some(arr) = find(text, body, depth + 1)
            {
                return Some(arr);
            }
        }
        None
    }

    /// The list a function returns when it is called where it is written, as an
    /// element of a module list: `(()=>{"use
    /// strict";return[1,a=>{..},2,b=>{..}]})()`. The bundler writes a group
    /// of modules that share a scope this way. With the range of the statements
    /// before the `return` (the declarations the modules share), if there are
    /// any.
    fn returned_group<'a, 'b>(
        el: &'b ArrayExpressionElement<'a>,
    ) -> Option<(&'b ArrayExpression<'a>, Option<(u32, u32)>)> {
        let ArrayExpressionElement::CallExpression(c) = el else {
            return None;
        };
        if !c.arguments.is_empty() {
            return None;
        }
        let mut callee = &c.callee;
        while let Expression::ParenthesizedExpression(p) = callee {
            callee = &p.expression;
        }
        let stmts: &[Statement<'a>] = match callee {
            Expression::ArrowFunctionExpression(f) if f.params.items.is_empty() => match &f.body {
                ArrowFunctionBody::FunctionBody(b) => &b.statements[..],
                _ => return None,
            },
            Expression::FunctionExpression(f) if f.params.items.is_empty() => {
                &f.body.as_ref()?.statements[..]
            }
            _ => return None,
        };
        let (last, before) = stmts.split_last()?;
        let Statement::ReturnStatement(r) = last else {
            return None;
        };
        let mut value = r.argument.as_ref()?;
        while let Expression::ParenthesizedExpression(p) = value {
            value = &p.expression;
        }
        let Expression::ArrayExpression(arr) = value else {
            return None;
        };
        let prelude = match (before.first(), before.last()) {
            (Some(a), Some(b)) => Some((a.span().start, b.span().end)),
            _ => None,
        };
        Some((arr, prelude))
    }

    /// The source of a factory's leading `"use strict"` directive. A module has
    /// one when it stands alone and none inside a group whose function has
    /// it: the directive says where the bundler put the module, not what
    /// the module does.
    fn strict_directive<'t>(text: &'t str, el: &ArrayExpressionElement<'_>) -> Option<&'t str> {
        let directives = match el {
            ArrayExpressionElement::ArrowFunctionExpression(f) => match &f.body {
                ArrowFunctionBody::FunctionBody(b) => &b.directives,
                _ => return None,
            },
            ArrayExpressionElement::FunctionExpression(f) => &f.body.as_ref()?.directives,
            _ => return None,
        };
        let d = directives.first()?;
        let src = &text[d.span.start as usize..d.span.end as usize];
        matches!(src.trim_end_matches(';'), "\"use strict\"" | "'use strict'").then_some(src)
    }

    /// The start of the first parameter of every factory in `arr`, groups
    /// included.
    fn factory_params(arr: &ArrayExpression<'_>, out: &mut Vec<u32>) {
        for el in &arr.elements {
            if let Some((group, _)) = returned_group(el) {
                factory_params(group, out);
                continue;
            }
            let first = match el {
                ArrayExpressionElement::ArrowFunctionExpression(f) => f.params.items.first(),
                ArrayExpressionElement::FunctionExpression(f) => f.params.items.first(),
                ArrayExpressionElement::ArrayExpression(group) => {
                    factory_params(group, out);
                    None
                }
                _ => None,
            };
            out.extend(first.map(|p| p.span.start));
        }
    }

    /// Writes the canonical text of some ranges of the chunk.
    type Canonical<'c> = dyn Fn(&[(u32, u32)]) -> String + 'c;

    /// The modules of a list: ids followed by a factory. An element that is
    /// itself a list is a group of modules
    /// (`[[1,a=>{..},2,b=>{..}],3,c=>{..}]`) and is read the same way.
    fn modules_of(
        text: &str,
        arr: &ArrayExpression<'_>,
        canonical: &Canonical<'_>,
        out: &mut Chunk,
    ) {
        let mut ids: Vec<String> = Vec::new();
        for el in &arr.elements {
            // A group in a called function: its modules, and the declarations they share as
            // a text of their own.
            if ids.is_empty()
                && let Some((group, prelude)) = returned_group(el)
            {
                if let Some(range) = prelude {
                    out.other.push(canonical(&[range]));
                }
                modules_of(text, group, canonical, out);
                continue;
            }
            match el {
                ArrayExpressionElement::NumericLiteral(n) => {
                    ids.push(text[n.span.start as usize..n.span.end as usize].to_string());
                    continue;
                }
                ArrayExpressionElement::StringLiteral(s)
                    if !ids.is_empty() || (out.modules.is_empty() && out.other.is_empty()) =>
                {
                    // Development builds use string ids; a leading string is an id too.
                    ids.push(text[s.span.start as usize..s.span.end as usize].to_string());
                    continue;
                }
                ArrayExpressionElement::ArrayExpression(group) if ids.is_empty() => {
                    modules_of(text, group, canonical, out);
                    continue;
                }
                _ => {}
            }
            let span = el.span();
            let mut body = canonical(&[(span.start, span.end)]);
            if let Some(d) = strict_directive(text, el)
                && let Some(at) = body.find(d)
            {
                body.replace_range(at..at + d.len(), "");
                if !d.ends_with(';') && body[at..].starts_with(';') {
                    body.remove(at);
                }
            }
            if ids.is_empty() {
                out.other.push(body);
            } else {
                for id in ids.drain(..) {
                    out.modules.push(ChunkModule {
                        id,
                        text: body.clone(),
                    });
                }
            }
        }
        // Ids without a factory (should not happen) still count.
        for id in ids {
            out.modules.push(ChunkModule {
                id,
                text: String::new(),
            });
        }
    }

    /// See the module documentation.
    pub fn split(text: &str) -> Option<Chunk> {
        if u32::try_from(text.len()).is_err() {
            return None;
        }
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, text, SourceType::cjs()).parse();
        if ret.fatal_error || !ret.diagnostics.is_empty() {
            return None;
        }
        let program = ret.program;
        let arr = find(text, &program.body[..], 0)?;
        let sem = SemanticBuilder::new().build(&program);
        let mut c = Collect {
            text,
            scoping: sem.semantic.scoping(),
            occs: Vec::new(),
            shorthand: HashSet::new(),
            nums: Vec::new(),
            calls: Vec::new(),
            lists: Vec::new(),
            subs: Vec::new(),
        };
        c.visit_program(&program);
        c.subs.sort_by_key(|s| s.0);
        c.occs.sort_by_key(|o| (o.start, o.end));
        // The context parameters: the first parameter of every factory in the list.
        let symbol_at: HashMap<u32, SymbolId> =
            c.occs.iter().map(|o| (o.start, o.symbol)).collect();
        let mut params: Vec<u32> = Vec::new();
        factory_params(arr, &mut params);
        let contexts: HashSet<SymbolId> = params
            .iter()
            .filter_map(|start| symbol_at.get(start).copied())
            .collect();
        c.nums = c
            .calls
            .iter()
            .filter(|(object, _)| symbol_at.get(object).is_some_and(|s| contexts.contains(s)))
            .flat_map(|(_, ints)| ints.iter().copied())
            .collect();
        c.nums.sort_unstable();
        c.lists.sort_by_key(|l| l.start);
        let canonical = |ranges: &[(u32, u32)]| {
            let mut e = Emit {
                text,
                occs: &c.occs,
                shorthand: &c.shorthand,
                nums: &c.nums,
                lists: &c.lists,
                subs: &c.subs,
                index: HashMap::new(),
                out: String::new(),
            };
            for (i, (start, end)) in ranges.iter().enumerate() {
                if i > 0 {
                    e.out.push('\0');
                }
                e.range(*start, *end);
            }
            e.out
        };

        let mut out = Chunk::default();
        modules_of(text, arr, &canonical, &mut out);
        // The code around the list: wrappers, the runtime of an entry chunk.
        out.other.push(canonical(&[
            (0, arr.span.start),
            (arr.span.end, text.len() as u32),
        ]));
        Some(out)
    }

    #[cfg(test)]
    mod tests {
        use super::{super::resolve, *};

        const SERVER: &str = r#"module.exports=[34468,a=>{"use strict";var b=a.i(47560);a.S([b,"x"],34468)},47560,(a,b,c)=>{"use strict";let d=a.r(1);function e(f){return d.jsx("h1",{children:f.title})}a.s(["Hero",()=>e])}];"#;

        /// The texts with every integer as written.
        fn plain(c: &Chunk) -> Vec<String> {
            c.modules
                .iter()
                .map(|m| format!("{}:{}", m.id, resolve(&m.text, |_| None)))
                .collect()
        }

        #[test]
        fn one_entry_per_module() {
            let c = split(SERVER).unwrap();
            assert_eq!(c.modules.len(), 2);
            assert_eq!(c.modules[0].id, "34468");
            assert_eq!(c.modules[1].id, "47560");
            assert_eq!(c.other.len(), 1);
        }

        #[test]
        fn renamed_locals_are_equal() {
            // The same chunk with every local renamed, as a minifier would in another
            // build.
            let renamed = r#"module.exports=[34468,q=>{"use strict";var z=q.i(47560);q.S([z,"x"],34468)},47560,(r,s,t)=>{"use strict";let u=r.r(1);function v(w){return u.jsx("h1",{children:w.title})}r.s(["Hero",()=>v])}];"#;
            assert_eq!(split(SERVER), split(renamed));
        }

        #[test]
        fn content_changes_differ() {
            let base = plain(&split(SERVER).unwrap());
            // A string, a property name, a number and a global are content.
            for (from, to) in [
                (r#""h1""#, r#""h2""#),
                ("f.title", "f.label"),
                ("a.r(1)", "a.r(2)"),
                ("d.jsx(", "d.jsxs("),
            ] {
                let other = plain(&split(&SERVER.replace(from, to)).unwrap());
                assert_eq!(base[0], other[0], "{from}");
                assert_ne!(base[1], other[1], "{from}");
            }
            // Using a different local is a change, not a renaming.
            let swapped = SERVER.replace("children:f.title", "children:d.title");
            assert_ne!(base[1], plain(&split(&swapped).unwrap())[1]);
        }

        #[test]
        fn modules_do_not_depend_on_their_chunk() {
            // The second module alone in another chunk, with other names, is the same.
            let alone = r#"module.exports=[47560,(x,y,z)=>{"use strict";let k=x.r(1);function m(n){return k.jsx("h1",{children:n.title})}x.s(["Hero",()=>m])}];"#;
            assert_eq!(
                split(SERVER).unwrap().modules[1],
                split(alone).unwrap().modules[0]
            );
        }

        #[test]
        fn client_chunk_and_runtime_tail() {
            let client = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,42724,(e,t,r)=>{"use strict";var n=e.i(1);e.s(["Nav",()=>n])}]);var a=1;function b(c){return a+c}"#;
            let c = split(client).unwrap();
            assert_eq!(c.modules.len(), 1, "{c:?}");
            assert_eq!(c.modules[0].id, "42724");
            // The script element, and the code around the list (with the runtime after it).
            assert_eq!(c.other.len(), 2);
            let renamed = client.replace(
                "var a=1;function b(c){return a+c}",
                "var q=1;function w(e){return q+e}",
            );
            assert_eq!(c, split(&renamed).unwrap());
            let other_runtime = client.replace("return a+c", "return a-c");
            assert_ne!(c.other, split(&other_runtime).unwrap().other);
        }

        #[test]
        fn shorthand_keys_are_content() {
            let a = r#"module.exports=[1,a=>{let x=1;a.s({x})}];"#;
            let expanded = r#"module.exports=[1,a=>{let q=1;a.s({x:q})}];"#;
            let other_key = r#"module.exports=[1,a=>{let y=1;a.s({y})}];"#;
            assert_eq!(split(a), split(expanded));
            assert_ne!(split(a), split(other_key));
            // Destructuring: the key names the property read.
            let d1 = r#"module.exports=[1,a=>{let{x}=a;return x}];"#;
            let d2 = r#"module.exports=[1,a=>{let{x:q}=a;return q}];"#;
            let d3 = r#"module.exports=[1,a=>{let{y}=a;return y}];"#;
            assert_eq!(split(d1), split(d2));
            assert_ne!(split(d1), split(d3));
        }

        #[test]
        fn not_a_chunk() {
            assert!(split("var R=require('x');R.m(1)").is_none());
            assert!(split("module.exports=[1,a=>{").is_none());
            // An array passed to some other call is not a module list.
            assert!(split("foo([1,a=>a]);").is_none());
        }

        #[test]
        fn wrapped_chunks_are_split() {
            // A debug-id statement in front, the chunk inside a called function, comments
            // after.
            let wrapped = format!(
                ";!function(){{try{{var e=globalThis,n=(new \
                 e.Error).stack;n&&((e._debugIds||(e._debugIds={{}}))[n]=\"<DEBUG_ID>\"\
                 )}}catch(e){{}}}}();\n(()=>{{\"use strict\";{}}})();\n\n//# debugId=<DEBUG_ID>\n",
                SERVER.trim_end_matches(';')
            );
            assert!(super::super::is_turbopack_chunk(&wrapped));
            let w = split(&wrapped).unwrap();
            assert_eq!(w.modules, split(SERVER).unwrap().modules);
            // The wrapper is part of the chunk's other code.
            assert_ne!(w.other, split(SERVER).unwrap().other);
        }

        #[test]
        fn integers_are_marked_and_resolved() {
            let c = split(SERVER).unwrap();
            // Module 34468 names 47560 and itself.
            let named = resolve(&c.modules[0].text, |n| match n {
                "47560" => Some("hero"),
                "34468" => Some("self"),
                _ => None,
            });
            assert!(named.contains(".i(<hero>)"), "{named}");
            assert!(named.contains("],<self>)"), "{named}");
            // With another id for the same target, the importer is the same once resolved.
            let other = split(&SERVER.replace("47560", "99999")).unwrap();
            let again = resolve(&other.modules[0].text, |n| match n {
                "99999" => Some("hero"),
                "34468" => Some("self"),
                _ => None,
            });
            assert_eq!(named, again);
        }

        #[test]
        fn only_arguments_of_the_context_are_module_ids() {
            // 47560 as a constant and as an argument of another object's method is a
            // number.
            let c = split(
                r#"module.exports=[7,(a,b)=>{var x=47560,t=[];a.i(47560);t.slice(47560);b.i(47560);(()=>a.r(47560))()}];"#,
            )
            .unwrap();
            assert_eq!(
                super::super::integers(&c.modules[0].text),
                ["47560", "47560"]
            );
        }

        #[test]
        fn a_group_of_modules_is_read_as_modules() {
            // The first element is a list of its own.
            let grouped = r#"module.exports=[[34468,a=>{"use strict";var b=a.i(47560);a.S([b,"x"],34468)},47560,(a,b,c)=>{"use strict";let d=a.r(1);function e(f){return d.jsx("h1",{children:f.title})}a.s(["Hero",()=>e])}],9,(a,b,c)=>{b.exports=a.x("node:crypto",()=>require("node:crypto"))}];"#;
            let g = split(grouped).unwrap();
            assert_eq!(g.modules.len(), 3, "{g:?}");
            assert_eq!(g.modules[..2], split(SERVER).unwrap().modules[..]);
            assert_eq!(g.modules[2].id, "9");
            // Module ids inside the group are marked too.
            assert_eq!(
                super::super::integers(&g.modules[0].text),
                ["47560", "34468"]
            );
        }

        #[test]
        fn export_lists_are_sorted() {
            // The module that lists an entry's server actions sorts them by id.
            let a = r#"module.exports=[7,a=>{var e=a.i(11),f=a.i(12),g=a.i(13);a.s(["<ACTION x#a>",()=>f.n,"<ACTION x#b>",()=>e.P,"<ACTION y#c>",0,g])}];"#;
            let b = r#"module.exports=[7,a=>{var e=a.i(11),f=a.i(12),g=a.i(13);a.s(["<ACTION y#c>",0,g,"<ACTION x#b>",()=>e.P,"<ACTION x#a>",()=>f.n])}];"#;
            assert_eq!(split(a), split(b));
            // Another getter for a name is a change.
            let c = a.replace("()=>f.n", "()=>e.n");
            assert_ne!(split(a), split(&c));
        }

        #[test]
        fn a_group_in_a_called_function_is_read_as_modules() {
            // The modules alone, and as a group that shares a function and its directive.
            let alone = r#"module.exports=[500001,a=>{"use strict";var b=a.i(700001);a.s(["Card",0,()=>b.x])},500002,500003,c=>{"use strict";c.v(1)}];"#;
            let grouped = r#"module.exports=[(()=>{"use strict";return[500001,a=>{var b=a.i(700001);a.s(["Card",0,()=>b.x])},500002,500003,c=>{c.v(1)}]})()];"#;
            let (a, g) = (split(alone).unwrap(), split(grouped).unwrap());
            assert_eq!(g.modules.len(), 3, "{g:?}");
            // A module is the same alone and in a group: where the directive stands is not
            // content.
            assert_eq!(a.modules, g.modules);
            // Module ids inside the group are marked.
            assert_eq!(super::super::integers(&g.modules[0].text), ["700001"]);
            // The client form, with the script element first.
            let client = r#"(globalThis.TURBOPACK||(globalThis.TURBOPACK=[])).push(["object"==typeof document?document.currentScript:void 0,(()=>{"use strict";return[500001,a=>{var b=a.i(700001);a.s(["Card",0,()=>b.x])},500002,500003,c=>{c.v(1)}]})()]);"#;
            assert_eq!(split(client).unwrap().modules, g.modules);
            // Another target and another string are changes.
            let target = split(&grouped.replace("700001", "700777")).unwrap();
            assert_ne!(plain(&g), plain(&target));
            let string = split(&grouped.replace(r#""Card""#, r#""Cart""#)).unwrap();
            assert_ne!(g.modules[0], string.modules[0]);
            assert_eq!(g.modules[1..], string.modules[1..]);
        }

        #[test]
        fn declarations_of_a_group_are_a_text_of_their_own() {
            let g =
                split(r#"module.exports=[(()=>{"use strict";let k=1;return[7,a=>{a.v(k)}]})()];"#)
                    .unwrap();
            assert_eq!(g.modules.len(), 1, "{g:?}");
            assert!(g.other.iter().any(|t| t.contains("=1")), "{g:?}");
            let other =
                split(r#"module.exports=[(()=>{"use strict";let k=2;return[7,a=>{a.v(k)}]})()];"#)
                    .unwrap();
            assert_eq!(g.modules, other.modules);
            assert_ne!(g.other, other.other);
        }

        #[test]
        fn reexport_pairs_are_sorted_and_their_modules_marked() {
            // `S` lists what a module re-exports from other modules as `name,local` pairs,
            // sorted by name; the name of a server action is its id.
            let a = r#"module.exports=[7,a=>{"use strict";a.S([810002,"<ACTION lib/a.ts#save>,x,<ACTION lib/a.ts#load>,y",0,810003,"<ACTION lib/b.ts#run>,z"])}];"#;
            let b = a.replace(
                "<ACTION lib/a.ts#save>,x,<ACTION lib/a.ts#load>,y",
                "<ACTION lib/a.ts#load>,y,<ACTION lib/a.ts#save>,x",
            );
            assert_eq!(split(a), split(&b));
            assert_eq!(
                super::super::integers(&split(a).unwrap().modules[0].text),
                ["810002", "810003"]
            );
            // Another local for a name and another source module are changes.
            assert_ne!(split(a), split(&a.replace("save>,x", "save>,w")));
            assert_ne!(
                plain(&split(a).unwrap()),
                plain(&split(&a.replace("810002", "810009")).unwrap())
            );
        }

        #[test]
        fn a_list_of_chunk_names_is_grouping() {
            // The loader of a lazy import: the chunks to load, then the module to run.
            let a = r#"module.exports=[7,a=>{a.v(b=>Promise.all(["static/chunks/<F>.js","static/chunks/<F>.css"].map(c=>b.l(c))).then(()=>b(900001)))}];"#;
            let b = a.replace(r#","static/chunks/<F>.css""#, "");
            assert_eq!(split(a), split(&b));
            // What runs after the chunks are loaded is content.
            assert_ne!(split(a), split(&a.replace("900001", "900002")));
            // A list of other strings is content.
            let c = r#"module.exports=[7,a=>{a.v(["x","y"])}];"#;
            assert_ne!(split(c), split(&c.replace(r#","y""#, "")));
        }
    }
}
