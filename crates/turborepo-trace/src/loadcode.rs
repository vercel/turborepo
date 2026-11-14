//! What loading a module runs: its code outside function bodies.
//!
//! When a module is loaded, its top level runs. The bodies of the functions it
//! defines run only if something calls them. [`scan`] separates the two for one
//! module, and says whether the top level is *inert*: it calls nothing of the
//! repository, so no function body of the module runs while it loads, and it
//! does nothing but define its bindings and load the modules it names.
//!
//! This is what `crate::nextconfigdiff` needs for a module that a Next.js
//! config imports and only its route functions use: such a module is loaded
//! when the config loads, and if every module loaded that way is inert, a
//! change inside a function body (or to a file that only a function body reads)
//! cannot change what the load does.
//!
//! # Inert
//!
//! Outside function bodies the module may only have:
//!
//! - `import` and `export` declarations (the loads are reported in
//!   [`LoadCode::loads`]) and `require('<literal>')`;
//! - declarations of functions, classes (no static blocks, static fields with a
//!   value or decorators) and variables;
//! - values: literals, templates, arrays, objects (no accessors: reading a
//!   property of a value the module defines then runs nothing), function and
//!   class expressions, references, member reads, operators, conditionals;
//! - calls of a few built-ins that call nothing back, with plain arguments:
//!   `new URL(...)`, `fileURLToPath(...)`, `createRequire(...)`, the functions
//!   of `node:path`, `String(...)`, `Number(...)`, `Boolean(...)`. An argument
//!   is plain when it is a literal, a template or a `+` of plain values,
//!   `import.meta.url` / `dirname` / `filename`, `__dirname` / `__filename`, or
//!   a variable (an object with its own `toString` is not ruled out, see
//!   below);
//! - `module.exports = ...` and `exports.name = ...`.
//!
//! Not allowed, each of which makes the module not inert: any other call or
//! `new`, a tagged template, an assignment to anything else, `++` / `--`,
//! `delete`, `await`, `yield`, `import()`, a decorator, and a reference to a
//! binding imported from a package (a value of a package can have accessors,
//! and a package's exports are not in the repository).
//!
//! The check is deliberately incomplete on the side of "not inert". Two things
//! it does not rule out, both of which would need the module to be written
//! against the check: a built-in call that converts an object with a
//! user-defined `toString` or `valueOf`, and an implicit conversion in an
//! operator or a template of such an object.
//!
//! # Comparing two revisions
//!
//! [`same_load`] says whether two revisions of an inert module run the same
//! code when they load: with every function body blanked they have the same
//! runtime JavaScript ([`crate::erase::erased_equal`]: types, comments and
//! formatting don't count).

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrowFunctionBody, AssignmentExpression, AssignmentTarget, AwaitExpression,
    CallExpression, Decorator, Expression, FunctionBody, IdentifierReference,
    ImportDeclarationSpecifier, ImportExpression, NewExpression, ObjectProperty,
    PropertyDefinition, PropertyKind, Statement, StaticBlock, TaggedTemplateExpression,
    UnaryExpression, UnaryOperator, UpdateExpression, YieldExpression,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

/// The load-time code of one module.
#[derive(Clone, Debug)]
pub struct LoadCode {
    /// Why the top level is not inert, if it is not.
    pub not_inert: Option<String>,
    /// The specifiers of the modules the top level loads: `import`, `export ...
    /// from` and `require('<literal>')` outside function bodies, in source
    /// order.
    pub loads: Vec<String>,
    /// The source with every function body replaced by `{}`.
    pub blanked: String,
}

struct Scan<'s> {
    src: &'s str,
    bodies: Vec<(u32, u32)>,
    loads: Vec<String>,
    /// Local names bound by imports of packages.
    package_locals: Vec<String>,
    /// Local names bound to the namespace or default export of `node:path`.
    path_locals: Vec<String>,
    /// Local names of built-in functions that call nothing back (imported).
    safe_fns: Vec<String>,
    not_inert: Option<String>,
}

impl Scan<'_> {
    fn no(&mut self, why: &str, at: u32) {
        if self.not_inert.is_none() {
            let line = self.src[..(at as usize).min(self.src.len())]
                .bytes()
                .filter(|b| *b == b'\n')
                .count()
                + 1;
            self.not_inert = Some(format!("{why} (line {line})"));
        }
    }

    /// Whether `e` is a plain argument (module docs).
    fn plain(&self, e: &Expression<'_>) -> bool {
        match e {
            Expression::StringLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_) => true,
            Expression::TemplateLiteral(t) => t.expressions.iter().all(|x| self.plain(x)),
            Expression::BinaryExpression(b) => self.plain(&b.left) && self.plain(&b.right),
            Expression::ParenthesizedExpression(p) => self.plain(&p.expression),
            Expression::Identifier(i) => !self.package_locals.iter().any(|n| n == i.name.as_str()),
            Expression::StaticMemberExpression(m) => {
                matches!(&m.object, Expression::ImportMeta(_))
                    && matches!(m.property.name.as_str(), "url" | "dirname" | "filename")
            }
            // A nested call of the same kind (`path.dirname(fileURLToPath(import.meta.url))`).
            Expression::CallExpression(c) => self.safe_call(&c.callee, &c.arguments),
            Expression::NewExpression(n) => self.safe_new(&n.callee, &n.arguments),
            _ => false,
        }
    }

    fn plain_args(&self, args: &[Argument<'_>]) -> bool {
        args.iter()
            .all(|a| a.as_expression().is_some_and(|e| self.plain(e)))
    }

    fn safe_call(&self, callee: &Expression<'_>, args: &[Argument<'_>]) -> bool {
        let known = match callee {
            Expression::Identifier(i) => {
                matches!(i.name.as_str(), "String" | "Number" | "Boolean")
                    || self.safe_fns.iter().any(|n| n == i.name.as_str())
            }
            Expression::StaticMemberExpression(m) => {
                matches!(&m.object, Expression::Identifier(o)
                    if self.path_locals.iter().any(|n| n == o.name.as_str()))
            }
            _ => false,
        };
        known && self.plain_args(args)
    }

    fn safe_new(&self, callee: &Expression<'_>, args: &[Argument<'_>]) -> bool {
        matches!(callee, Expression::Identifier(i) if i.name == "URL") && self.plain_args(args)
    }
}

impl<'a> Visit<'a> for Scan<'_> {
    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        // Not walked: it runs only when the function is called.
        self.bodies.push((it.span.start, it.span.end));
    }

    fn visit_arrow_function_body(&mut self, it: &ArrowFunctionBody<'a>) {
        let span = it.span();
        self.bodies.push((span.start, span.end));
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        // `require('<literal>')` loads a module.
        if let Expression::Identifier(i) = &it.callee
            && i.name == "require"
            && let [Argument::StringLiteral(s)] = it.arguments.as_slice()
        {
            if let Some(v) = s.value.as_str() {
                self.loads.push(v.to_owned());
            }
            return;
        }
        if !self.safe_call(&it.callee, &it.arguments) {
            self.no("a call at the top level", it.span.start);
        }
        // Not walked further: the callee and the arguments were checked as a
        // whole.
    }

    fn visit_new_expression(&mut self, it: &NewExpression<'a>) {
        if !self.safe_new(&it.callee, &it.arguments) {
            self.no("`new` at the top level", it.span.start);
        }
    }

    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        self.no("a tagged template at the top level", it.span.start);
    }

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        let exports = match &it.left {
            AssignmentTarget::StaticMemberExpression(m) => match &m.object {
                Expression::Identifier(o) => {
                    (o.name == "module" && m.property.name == "exports") || o.name == "exports"
                }
                Expression::StaticMemberExpression(mm) => {
                    matches!(&mm.object, Expression::Identifier(o) if o.name == "module")
                        && mm.property.name == "exports"
                }
                _ => false,
            },
            _ => false,
        };
        if !exports {
            self.no("an assignment at the top level", it.span.start);
        }
        self.visit_expression(&it.right);
    }

    fn visit_update_expression(&mut self, it: &UpdateExpression<'a>) {
        self.no("an update at the top level", it.span.start);
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        if it.operator == UnaryOperator::Delete {
            self.no("`delete` at the top level", it.span.start);
        }
        walk::walk_unary_expression(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        self.no("`await` at the top level", it.span.start);
    }

    fn visit_yield_expression(&mut self, it: &YieldExpression<'a>) {
        self.no("`yield` at the top level", it.span.start);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.no("`import()` at the top level", it.span.start);
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.no("a static block", it.span.start);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.r#static && it.value.is_some() {
            self.no("a static field with a value", it.span.start);
        }
        // An instance field's value runs when the class is constructed, which
        // nothing allowed here does.
    }

    fn visit_decorator(&mut self, it: &Decorator<'a>) {
        self.no("a decorator", it.span.start);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if it.kind != PropertyKind::Init {
            self.no("an accessor in an object", it.span.start);
        }
        walk::walk_object_property(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        if self.package_locals.iter().any(|n| n == it.name.as_str()) {
            self.no(
                "a value of a package is used at the top level",
                it.span.start,
            );
        }
    }
}

/// Whether `spec` names a Node built-in module.
pub fn is_builtin(spec: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "assert",
        "buffer",
        "crypto",
        "events",
        "fs",
        "module",
        "os",
        "path",
        "querystring",
        "stream",
        "url",
        "util",
        "zlib",
    ];
    spec.starts_with("node:") || BUILTINS.contains(&spec.split('/').next().unwrap_or(spec))
}

/// Scans the module `src` at `path` (the extension says how to parse it).
/// `None` when it does not parse.
pub fn scan(path: &str, src: &str) -> Option<LoadCode> {
    let st = SourceType::from_path(path).ok()?;
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return None;
    }
    let mut scan = Scan {
        src,
        bodies: Vec::new(),
        loads: Vec::new(),
        package_locals: Vec::new(),
        path_locals: Vec::new(),
        safe_fns: Vec::new(),
        not_inert: None,
    };
    // What the imports bind.
    for s in &ret.program.body {
        let Statement::ImportDeclaration(d) = s else {
            continue;
        };
        if d.import_kind.is_type() {
            continue;
        }
        let spec = d.source.value.as_str()?;
        let builtin = is_builtin(spec);
        let package = !builtin && !spec.starts_with(['.', '/']);
        let module = spec.strip_prefix("node:").unwrap_or(spec);
        for sp in d.specifiers.iter().flatten() {
            let (local, imported) = match sp {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    (s.local.name.as_str(), Some(s.imported.name()))
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                    (s.local.name.as_str(), None)
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                    (s.local.name.as_str(), None)
                }
            };
            if package {
                scan.package_locals.push(local.to_owned());
            } else if builtin {
                match (module, imported.as_ref().map(|i| i.as_str())) {
                    ("path", None) => scan.path_locals.push(local.to_owned()),
                    ("path", Some(_))
                    | ("url", Some("fileURLToPath" | "pathToFileURL"))
                    | ("module", Some("createRequire")) => scan.safe_fns.push(local.to_owned()),
                    _ => {}
                }
            }
        }
    }
    for s in &ret.program.body {
        match s {
            Statement::ImportDeclaration(d) if !d.import_kind.is_type() => {
                scan.loads.push(d.source.value.as_str()?.to_owned());
            }
            Statement::ExportAllDeclaration(d) if !d.export_kind.is_type() => {
                scan.loads.push(d.source.value.as_str()?.to_owned());
            }
            Statement::ExportFromDeclaration(d) if !d.export_kind.is_type() => {
                scan.loads.push(d.source.value.as_str()?.to_owned());
            }
            _ => {}
        }
    }
    scan.visit_program(&ret.program);
    scan.bodies.sort_unstable();
    let mut blanked = String::with_capacity(src.len());
    let mut at = 0usize;
    for (a, b) in &scan.bodies {
        let (a, b) = (*a as usize, *b as usize);
        if a < at || b > src.len() {
            continue;
        }
        blanked.push_str(&src[at..a]);
        blanked.push_str("{}");
        at = b;
    }
    blanked.push_str(&src[at..]);
    Some(LoadCode {
        not_inert: scan.not_inert,
        loads: scan.loads,
        blanked,
    })
}

/// Whether the revisions `base` and `head` of the module at `path` run the same
/// code when they load (module docs). `false` when it cannot be shown.
pub fn same_load(path: &str, base: &str, head: &str) -> bool {
    let (Some(b), Some(h)) = (scan(path, base), scan(path, head)) else {
        return false;
    };
    crate::erase::erased_equal(path, &b.blanked, &h.blanked).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inert(src: &str) -> Result<Vec<String>, String> {
        let c = scan("m.mjs", src).expect("parses");
        match c.not_inert {
            None => Ok(c.loads),
            Some(why) => Err(why),
        }
    }

    /// The shape of a real helper: a file-relative location, a function that
    /// reads files and lists directories, a package whose values only the
    /// function uses.
    const HELPER: &str = r#"import fs from 'node:fs';
import { Config } from '@acme/zones/config';

const appRoot = new URL('./', import.meta.url);

/** The paths other zones serve. */
export function childPaths() {
  const config = Config.fromFile(fs.readFileSync(new URL('zones.jsonc', appRoot), 'utf8'));
  return segments(new URL('./app', appRoot)).filter((s) => config.has(s));
}

function segments(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    out.push(entry.name);
  }
  return out;
}
"#;

    #[test]
    fn a_module_that_only_defines_things_is_inert() {
        assert_eq!(
            inert(HELPER),
            Ok(vec!["node:fs".to_owned(), "@acme/zones/config".to_owned()])
        );
        for src in [
            "export const a = 1;\nexport const b = [a, 'x', `y${a}`];\nexport default { a, b, f() \
             { return a(); } };",
            "import path from 'node:path';\nimport { fileURLToPath } from 'node:url';\nconst here \
             = path.dirname(fileURLToPath(import.meta.url));\nexport const file = path.join(here, \
             'x.json');",
            "import { createRequire } from 'node:module';\nconst require = \
             createRequire(import.meta.url);\nconst { list } = require('./list.cjs');\nexport { \
             list };",
            "const { a } = require('./a');\nmodule.exports = { a, b: () => a() };\nexports.c = \
             1;\nmodule.exports.d = 2;",
            "export class A { x = make(); static y; m() { return go(); } }\nexport const f = \
             async () => { await go(); };",
            "import { helper } from './helper.mjs';\nexport const g = () => helper();\nexport \
             const table = { helper };",
            "export const flag = process.env.FLAG === '1' ? 'a' : 'b';",
            "export * from './a.mjs';\nexport { b } from './b.mjs';",
        ] {
            assert!(inert(src).is_ok(), "{src}: {:?}", inert(src));
        }
        assert_eq!(
            inert("const { a } = require('./a');\nexport * from './b.mjs';").unwrap(),
            ["./b.mjs", "./a"]
        );
    }

    #[test]
    fn a_module_whose_load_does_something_is_not() {
        for (src, why) in [
            ("process.env.X = '1';\nexport const a = 1;", "an assignment"),
            (
                "export const a = compute();\nfunction compute() { return 1; }",
                "a call",
            ),
            ("import { setup } from './setup.mjs';\nsetup();", "a call"),
            ("export const list = [1, 2].map((x) => x);", "a call"),
            ("export const m = new Map();", "`new`"),
            (
                "export const u = new URL(target(), import.meta.url);",
                "`new`",
            ),
            (
                "import { pkg } from 'some-package';\nexport const v = pkg.value;",
                "a value of a package",
            ),
            (
                "import pkg from 'some-package';\nexport const u = new URL(pkg, import.meta.url);",
                "`new`",
            ),
            (
                "export const o = { get x() { return go(); } };",
                "an accessor",
            ),
            ("export class A { static x = make(); }", "a static field"),
            ("export class A { static { go(); } }", "a static block"),
            ("const data = await load();", "`await`"),
            ("const m = await import('./m.mjs');", "`await`"),
            ("let n = 0;\nn++;", "an update"),
            ("delete globalThis.x;", "`delete`"),
            (
                "export const s = String({ toString() { return go(); } });",
                "a call",
            ),
            ("export const q = sql`select 1`;", "a tagged template"),
            ("globalThis.flag = true;", "an assignment"),
        ] {
            let got = inert(src);
            assert!(
                got.as_ref().is_err_and(|e| e.starts_with(why)),
                "{src}: {got:?}"
            );
        }
        assert!(scan("m.mjs", "export const = ;").is_none());
    }

    #[test]
    fn two_revisions_that_run_the_same_code_when_they_load() {
        // One more line in a function, a changed call, a comment.
        let head = HELPER
            .replace(
                "out.push(entry.name);",
                "if (entry.isDirectory()) out.push(entry.name);",
            )
            .replace("/** The paths other zones serve. */", "// paths");
        assert!(same_load("m.mjs", HELPER, &head));
        // The top level changes: another location, another import, a new statement.
        for head in [
            HELPER.replace(
                "new URL('./', import.meta.url)",
                "new URL('../', import.meta.url)",
            ),
            HELPER.replace("'@acme/zones/config'", "'@acme/zones/other'"),
            format!("{HELPER}\nprocess.env.ZONES = '1';\n"),
            HELPER.replace(
                "function segments(dir)",
                "function segments(dir, deep = flag())",
            ),
        ] {
            assert!(!same_load("m.mjs", HELPER, &head), "{head}");
        }
        assert!(!same_load("m.mjs", HELPER, "export const = ;"));
    }
}
