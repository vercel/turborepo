//! Two revisions of a JS/TS config module compared outside some of its property
//! values.
//!
//! A config module is code: whether an edit matters depends on where it is.
//! [`equal_outside`] answers one question exactly: do the base and head
//! revisions have the same runtime JavaScript once the values of the object
//! properties with given names are blanked? If so, the edit is confined to
//! those values (and to types, comments and formatting), and the caller can
//! reason about the values alone.
//!
//! 1. Both revisions must parse without syntax or early errors, the blanked
//!    parts included ([`crate::erase::canonical`]): a syntax error anywhere
//!    breaks the load.
//! 2. A *site* is the value of a property in an object literal whose key is one
//!    of the names (an identifier or a string, not computed, not a getter or
//!    setter, not a shorthand). With [`What::Values`] the whole value is
//!    blanked (a method has no value expression and is not a site). With
//!    [`What::FunctionBodies`] only a function value counts (a method, a
//!    function expression, an arrow function), and only its body is blanked:
//!    the property, its parameters and `async` stay part of the comparison.
//! 3. Nothing else in the module may reach a site: outside the sites there must
//!    be no member access `.name`, no string `'name'` (a computed access) and
//!    no destructuring `{ name } = ...`. A variable or an import that happens
//!    to have the name is not the property.
//! 4. The blanked revisions are compared by [`crate::erase::erased_equal`].
//!
//! Two more things let a revision that *introduces* a site compare equal to one
//! without it. A [`Sites::wrapper`] property whose object literal holds nothing
//! but one site (`resolve: { alias: {...} }`) is removed whole, for a caller
//! that knows the empty object means what its absence means. And
//! [`without_imports`] drops import declarations the caller names (a Node
//! built-in that only the new site uses).

use std::collections::BTreeSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrowFunctionBody, BindingProperty, Expression, ObjectProperty, ObjectPropertyKind,
    PropertyKey, PropertyKind, Statement, StaticMemberExpression, StringLiteral,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// What is blanked of a named property.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum What {
    /// The body of a function value (other values are not sites).
    FunctionBodies,
    /// The whole value.
    Values,
}

/// Which parts of a module are sites.
#[derive(Copy, Clone, Debug)]
pub struct Sites {
    /// The property names.
    pub props: &'static [&'static str],
    /// What is blanked of such a property.
    pub what: What,
    /// A property of this name whose value is an object literal with one
    /// property, a site, is removed whole (with [`What::Values`]).
    pub wrapper: Option<&'static str>,
}

/// One revision with its sites blanked.
#[derive(Debug, Default)]
pub struct Blanked {
    /// The source with every site replaced by a placeholder.
    pub code: String,
    /// The names that have a site.
    pub names: BTreeSet<&'static str>,
    /// The text of the sites, in source order.
    pub sites: Vec<String>,
    /// The byte ranges of the source that were replaced, in source order.
    pub spans: Vec<(u32, u32)>,
}

/// How a span is replaced.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Fill {
    /// An expression: `0`.
    Expression,
    /// A function body: `{}`.
    Block,
    /// A property: nothing, and one comma next to it goes too.
    Property,
}

struct Finder {
    sites: Sites,
    /// Byte range to replace, how, and the byte range of the site's text.
    spans: Vec<(u32, u32, Fill, (u32, u32))>,
    names: BTreeSet<&'static str>,
    /// A name used outside a site.
    used: Option<&'static str>,
}

impl Finder {
    fn prop(&self, name: &str) -> Option<&'static str> {
        self.sites.props.iter().copied().find(|n| *n == name)
    }

    /// The name of a plain property (not computed) if it is one of `names`.
    fn key<'n>(it: &ObjectProperty<'_>, names: &[&'n str]) -> Option<&'n str> {
        let key = match &it.key {
            PropertyKey::StaticIdentifier(id) if !it.computed => id.name.as_str(),
            PropertyKey::StringLiteral(s) if !it.computed => s.value.as_str()?,
            _ => return None,
        };
        names.iter().copied().find(|n| *n == key)
    }

    /// The span to blank of the property `it`, if it is a site.
    fn site(&self, it: &ObjectProperty<'_>) -> Option<(&'static str, (u32, u32), Fill)> {
        let name = Self::key(it, self.sites.props)?;
        // A method has no value expression to replace: only its body can be blanked.
        let plain = it.kind == PropertyKind::Init
            && !it.shorthand
            && (!it.method || self.sites.what == What::FunctionBodies);
        if !plain {
            return None;
        }
        let (span, fill) = match (&it.value, self.sites.what) {
            (Expression::FunctionExpression(f), What::FunctionBodies) => {
                let b = f.body.as_ref()?;
                ((b.span.start, b.span.end), Fill::Block)
            }
            (Expression::ArrowFunctionExpression(f), What::FunctionBodies) => match &f.body {
                ArrowFunctionBody::FunctionBody(b) => ((b.span.start, b.span.end), Fill::Block),
                body => ((body.span().start, body.span().end), Fill::Expression),
            },
            (_, What::FunctionBodies) => return None,
            (value, What::Values) => ((value.span().start, value.span().end), Fill::Expression),
        };
        Some((self.prop(name)?, span, fill))
    }
}

impl<'a> Visit<'a> for Finder {
    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        // Not walked below a site: what a site names is its own business.
        if let Some((name, span, fill)) = self.site(it) {
            self.spans.push((span.0, span.1, fill, span));
            self.names.insert(name);
            return;
        }
        // `wrapper: { site }`: the whole property goes.
        if let Some(w) = self.sites.wrapper
            && self.sites.what == What::Values
            && Self::key(it, &[w]).is_some()
            && it.kind == PropertyKind::Init
            && !it.shorthand
            && let Expression::ObjectExpression(o) = &it.value
            && let [ObjectPropertyKind::ObjectProperty(inner)] = o.properties.as_slice()
            && let Some((name, span, _)) = self.site(inner)
        {
            self.spans
                .push((it.span.start, it.span.end, Fill::Property, span));
            self.names.insert(name);
            return;
        }
        walk::walk_object_property(self, it);
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        if let Some(n) = self.prop(it.property.name.as_str()) {
            self.used.get_or_insert(n);
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        if let Some(n) = it.value.as_str().and_then(|s| self.prop(s)) {
            self.used.get_or_insert(n);
        }
    }

    fn visit_binding_property(&mut self, it: &BindingProperty<'a>) {
        if let PropertyKey::StaticIdentifier(id) = &it.key
            && let Some(n) = self.prop(id.name.as_str())
        {
            self.used.get_or_insert(n);
        }
        walk::walk_binding_property(self, it);
    }
}

/// `src` with its sites blanked; the error says why the revision has no such
/// form.
pub fn blanked(path: &str, src: &str, sites: Sites) -> Result<Blanked, String> {
    let st = SourceType::from_path(path).map_err(|_| "not a JS or TS file".to_owned())?;
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return Err("parse errors".to_owned());
    }
    let mut found = Finder {
        sites,
        spans: Vec::new(),
        names: BTreeSet::new(),
        used: None,
    };
    found.visit_program(&ret.program);
    if let Some(name) = found.used {
        return Err(format!(
            "`{name}` is used outside its definition: other code of the module may reach it"
        ));
    }
    found.spans.sort_unstable();
    let mut out = Blanked {
        code: String::with_capacity(src.len()),
        names: found.names,
        sites: Vec::with_capacity(found.spans.len()),
        spans: Vec::with_capacity(found.spans.len()),
    };
    let bytes = src.as_bytes();
    let mut at = 0usize;
    for (start, end, fill, (s0, s1)) in found.spans {
        let (start, mut end) = (start as usize, end as usize);
        if start < at || end > src.len() || start > end {
            return Err("overlapping sites".to_owned());
        }
        out.code.push_str(&src[at..start]);
        match fill {
            Fill::Expression => out.code.push('0'),
            Fill::Block => out.code.push_str("{}"),
            // The comma after the property, else the one before it.
            Fill::Property => {
                let next = bytes[end..]
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .map(|i| end + i);
                if let Some(i) = next.filter(|&i| bytes[i] == b',') {
                    end = i + 1;
                } else {
                    let kept = out.code.trim_end().len();
                    if out.code[..kept].ends_with(',') {
                        out.code.truncate(kept - 1);
                    }
                }
            }
        }
        out.sites.push(src[s0 as usize..s1 as usize].to_owned());
        out.spans.push((start as u32, end as u32));
        at = end;
    }
    out.code.push_str(&src[at..]);
    Ok(out)
}

/// `src` without the import declarations for which `drop(module, declaration
/// text)` holds.
pub fn without_imports(
    path: &str,
    src: &str,
    drop: &dyn Fn(&str, &str) -> bool,
) -> Result<String, String> {
    let st = SourceType::from_path(path).map_err(|_| "not a JS or TS file".to_owned())?;
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return Err("parse errors".to_owned());
    }
    let mut out = String::with_capacity(src.len());
    let mut at = 0usize;
    for s in &ret.program.body {
        let Statement::ImportDeclaration(d) = s else {
            continue;
        };
        let (start, end) = (d.span.start as usize, d.span.end as usize);
        if start < at || end > src.len() {
            continue;
        }
        if d.source
            .value
            .as_str()
            .is_some_and(|m| drop(m, &src[start..end]))
        {
            out.push_str(&src[at..start]);
            at = end;
        }
    }
    out.push_str(&src[at..]);
    Ok(out)
}

/// Whether the base and head revisions of the module at `path` have the same
/// runtime JavaScript outside their sites (see the module docs): both revisions
/// blanked, base first. The error says why not, or why that cannot be shown.
pub fn equal_outside(
    path: &str,
    base: &str,
    head: &str,
    sites: Sites,
) -> Result<(Blanked, Blanked), String> {
    crate::erase::canonical(path, head).map_err(|e| format!("head: {e}"))?;
    crate::erase::canonical(path, base).map_err(|e| format!("base: {e}"))?;
    let h = blanked(path, head, sites).map_err(|e| format!("head: {e}"))?;
    let b = blanked(path, base, sites).map_err(|e| format!("base: {e}"))?;
    crate::erase::erased_equal(path, &b.code, &h.code)?;
    Ok((b, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODIES: Sites = Sites {
        props: &["rules", "routes"],
        what: What::FunctionBodies,
        wrapper: None,
    };
    const VALUES: Sites = Sites {
        props: &["routes"],
        what: What::Values,
        wrapper: Some("resolve"),
    };

    #[test]
    fn function_bodies() {
        let src = "export default { a: 1, routes: { x: './x' }, rules() { return [1]; }, 'rules': \
                   () => [2] };\n";
        let f = blanked("c.mjs", src, BODIES).unwrap();
        assert_eq!(
            f.code,
            "export default { a: 1, routes: { x: './x' }, rules() {}, 'rules': () => 0 };\n"
        );
        assert_eq!(f.sites, ["{ return [1]; }", "[2]"]);
        assert_eq!(f.names.into_iter().collect::<Vec<_>>(), ["rules"]);
    }

    #[test]
    fn what_is_not_a_site() {
        // Getters, shorthands and computed keys.
        for src in [
            "const rules = () => []; export default { rules };\n",
            "export default { get rules() { return []; } };\n",
            "const k = 'x'; export default { [k]: () => [] };\n",
        ] {
            let b = blanked("c.mjs", src, BODIES).unwrap();
            assert_eq!(b.code, src);
            assert!(b.names.is_empty());
        }
        // Another reader of the property.
        for src in [
            "const c = { rules() {} }; use(c.rules); export default c;\n",
            "const c = { rules() {} }; use(c['rules']); export default c;\n",
            "const c = { rules() {} }; const { rules } = c; export default c;\n",
        ] {
            let e = blanked("c.mjs", src, BODIES).unwrap_err();
            assert!(e.contains("`rules` is used outside its definition"), "{e}");
        }
        assert!(blanked("c.json", "{}", BODIES).is_err());
        assert!(blanked("c.mjs", "export default {", BODIES).is_err());
    }

    #[test]
    fn equal_outside_the_sites() {
        let a = "export default { a: 1, rules: () => [{ x: './x' }] };\n";
        let b =
            "export default {\n  a: 1, // one\n  rules: () => [{ x: './x' }, { y: './y' }],\n};\n";
        let (old, new) = equal_outside("c.mjs", a, b, BODIES).unwrap();
        assert_eq!(old.code, "export default { a: 1, rules: () => 0 };\n");
        assert_eq!(new.code.matches("rules: () => 0").count(), 1);
        let c = "export default { a: 2, rules: () => [{ x: './x' }] };\n";
        assert!(equal_outside("c.mjs", a, c, BODIES).is_err());
        // A syntax error inside a site.
        let d = "export default { a: 1, rules: () => [{ x: './x' y: 1 }] };\n";
        assert!(equal_outside("c.mjs", a, d, BODIES).is_err());
    }

    #[test]
    fn values() {
        let src = "export default { a: 1, routes: { x: './x' }, rules() { return [1]; }, \
                   'routes': () => [2] };\n";
        let v = blanked("c.mjs", src, VALUES).unwrap();
        assert_eq!(
            v.code,
            "export default { a: 1, routes: 0, rules() { return [1]; }, 'routes': 0 };\n"
        );
        assert_eq!(v.sites, ["{ x: './x' }", "() => [2]"]);
        // A shorthand names a variable, a method has no value expression.
        for src in [
            "const routes = {}; export default { routes };\n",
            "export default { routes() { return {}; } };\n",
        ] {
            let v = blanked("c.mjs", src, VALUES).unwrap();
            assert_eq!(v.code, src);
            assert!(v.sites.is_empty());
        }
        // One more entry in a value.
        let a = "export default { a: 1, routes: { x: './x' } };\n";
        let b = "export default {\n  a: 1,\n  routes: { x: './x', y: './y' },\n};\n";
        let (old, new) = equal_outside("c.mjs", a, b, VALUES).unwrap();
        assert_eq!(old.sites, ["{ x: './x' }"]);
        assert_eq!(new.sites, ["{ x: './x', y: './y' }"]);
        let d = "export default { a: 1, routes: { x: './x' y: 1 } };\n";
        assert!(equal_outside("c.mjs", a, d, VALUES).is_err());
    }

    #[test]
    fn a_wrapper_with_nothing_but_a_site_goes_whole() {
        let base = "export default define({\n  test: { globals: true },\n});\n";
        // First, last and only property.
        for head in [
            "export default define({\n  resolve: {\n    routes: { x: './x' },\n  },\n  test: { \
             globals: true },\n});\n",
            "export default define({\n  test: { globals: true },\n  resolve: { routes: { x: './x' \
             } },\n});\n",
            "export default define({\n  test: { globals: true },\n  resolve: { routes: { x: './x' \
             } }\n});\n",
        ] {
            let (old, new) = equal_outside("c.mjs", base, head, VALUES).unwrap();
            assert!(old.sites.is_empty());
            assert_eq!(new.sites, ["{ x: './x' }"], "{head}");
        }
        let v = blanked(
            "c.mjs",
            "export default { resolve: { routes: 1 } };\n",
            VALUES,
        )
        .unwrap();
        assert_eq!(v.code, "export default {  };\n");
        // A wrapper with something else keeps it.
        let v = blanked(
            "c.mjs",
            "export default { resolve: { routes: 1, conditions: [] } };\n",
            VALUES,
        )
        .unwrap();
        assert_eq!(
            v.code,
            "export default { resolve: { routes: 0, conditions: [] } };\n"
        );
    }

    #[test]
    fn imports_dropped_by_the_caller() {
        let src = "import { a } from 'node:url';\nimport b from './b';\nexport default { a, b };\n";
        let out = without_imports("c.mjs", src, &|module, text| {
            module == "node:url" && text.contains("{ a }")
        })
        .unwrap();
        assert_eq!(out, "\nimport b from './b';\nexport default { a, b };\n");
        assert_eq!(without_imports("c.mjs", src, &|_, _| false).unwrap(), src);
    }
}
