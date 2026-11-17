//! Who consumes a package that a tool's configuration names.
//!
//! A `package.json` dependency edit selects the consumers of the dependency
//! ([`crate::configdiff`]). Most are import edges. A config file can also name
//! a package in a string, and what that string does depends on the field that
//! holds it: Next.js *transpiles* the packages of `transpilePackages` when a
//! module imports them, and jest *loads* the package of `testEnvironment` for
//! every test. This module reads where a config names a package and says which
//! of three answers applies.
//!
//! # The three answers
//!
//! | Answer | Meaning | What is selected |
//! |---|---|---|
//! | [`Named::Importers`] | the field tells the tool how to treat the package's files *when a module loads them* | nothing beyond the nodes that load the package (its importers; the tool's build task reaches them like every other task does) |
//! | [`Named::Readers`] | the tool loads the package itself, or the role of the string is not one of the fields below | everything an edit of the config file would select: the nodes with an edge to it (a runner's tests through their `config` edge, tasks, importers) and the nodes that have it as an input (the jest tests of a `next/jest` project) |
//! | `None` | the file is not the config of a tool whose readers the graph knows (Babel, SWC, PostCSS, ESLint, ...) | the caller keeps "consumers are not known": the manifest stays a whole-file change |
//!
//! [`Named::Readers`] is always sound for a config whose readers the graph
//! knows: a string in a config file can only act through a tool that reads that
//! file. It is the fallback for every position this module does not recognize,
//! a config that does not parse included. [`Named::Importers`] needs an
//! argument per field.
//!
//! # The fields that are decided, and why
//!
//! A *field* is a path of plain property keys from the outermost object literal
//! that holds the string (`experimental.optimizePackageImports[]`), read where
//! it is written. The exported value is not evaluated: a real config hands its
//! object to wrappers (`withPlugin(nextConfig)`), and the meaning of a key of
//! the tool's own schema does not depend on them. A list may be put together
//! from array literals: `[...].concat(more)`, `[...].filter(keep)`, `cond ?
//! [...] : [...]`, `a || [...]` and `[...other, ...[...]]` hold the strings of
//! those literals as elements, or leave them out. One binding is followed: a
//! list that initializes a `const` (also joined: `const names =
//! [...].join('|')`) is in the fields in which the variable is used, as the
//! value, spread, or inside a template that is an element of the field (``
//! transformIgnorePatterns: [`/node_modules/(?!(${names})/)`] ``); every use of
//! the variable must be such a use, and the variable must not be exported. A
//! string in any other position (a call argument, a variable used elsewhere, a
//! function body) is not a field.
//!
//! **`next.config.*`** (Next.js; `next/jest` reads the same file):
//!
//! | Field | What Next.js does with it | Answer |
//! |---|---|---|
//! | `transpilePackages[]` | the bundler compiles the files of these packages instead of taking them as they are; `next/jest` leaves them out of `transformIgnorePatterns`, so the SWC transform runs on their files when a test loads them ([docs](https://nextjs.org/docs/app/api-reference/config/next-config-js/transpilePackages), `next/src/build/jest/jest.ts`) | importers |
//! | `serverExternalPackages[]`, `experimental.serverComponentsExternalPackages[]` | the server bundle leaves `require` of these packages to Node ([docs](https://nextjs.org/docs/app/api-reference/config/next-config-js/serverExternalPackages)); `next/jest` does not read it | importers |
//! | `experimental.optimizePackageImports[]` | imports of these packages are rewritten to the modules they use ([docs](https://nextjs.org/docs/app/api-reference/config/next-config-js/optimizePackageImports)); `next/jest` does not read it | importers |
//! | `modularizeImports.<package>` (the key) and its `transform` | import statements of that package are rewritten in the importing file; `next/jest` hands the table to the transform, which applies it to files that import the package | importers |
//! | `experimental.swcPlugins[][]` and every other position | a plugin runs on every file the compiler handles, and `next/jest` hands the list to the transform of every test file; elsewhere the role is not known | readers |
//!
//! **`jest.config.*`** ([configuration](https://jestjs.io/docs/configuration)):
//!
//! | Field | What jest does with it | Answer |
//! |---|---|---|
//! | `transformIgnorePatterns[]` (also under `projects[]`) | a file whose path matches is not transformed; the pattern is consulted for a file when a test loads it | importers |
//! | everything else (`transform`, `preset`, `testEnvironment`, `setupFiles`, `resolver`, `runner`, `snapshotSerializers`, `moduleNameMapper`, ...) | loaded for every test of the project. `moduleNameMapper` is here too: jest applies it to every module it loads, the files of dependencies included, whose requests the graph does not record | readers |
//!
//! **`vitest.config.*`, `vite.config.*`, `vitest.workspace.*`**
//! ([vitest](https://vitest.dev/config/), [vite](https://vite.dev/config/)):
//!
//! | Field | What the tool does with it | Answer |
//! |---|---|---|
//! | `test.deps.inline[]`, `test.server.deps.inline[]`, `test.server.deps.external[]`, `test.deps.optimizer.{web,ssr}.{include,exclude}[]`, `optimizeDeps.{include,exclude}[]`, `ssr.noExternal[]`, `ssr.external[]` (also under `test.projects[]` / `projects[]`) | whether a dependency's files go through Vite's pipeline or are loaded natively, decided for a file when something loads it | importers |
//! | everything else (`test.environment`, `test.setupFiles`, `resolve.alias`, `resolve.dedupe`, plugins and their options, ...) | loaded or applied for every test. `resolve.dedupe` is here: it redirects the imports of every package of the test's module graph to the project's copy, also from files of other packages | readers |
//!
//! Why "importers" is enough for those fields: a test can observe the version
//! of a package only by loading one of its files. A file of the repository that
//! loads it has an edge to it and is seeded by the manifest diff. A third-party
//! package that loads it does so through its own dependency, which the lockfile
//! diff reports for the importer. The field changes how the tool treats those
//! files once they are loaded, not who loads them.
//!
//! An import of the package in the config (`import { withPlugin } from
//! 'plugin'`, `require('plugin')`) is an edge of the graph: the config is one
//! of the package's importers and is seeded as such by the caller, like any
//! other file, and it counts as changed for the nodes that read its values
//! without loading it ([`crate::configdiff::config_imports`]).
//!
//! These rules are argued from each tool's documentation and source, for jest,
//! `next/jest` and vitest: when a package is bumped to a version that behaves
//! differently, every test whose result changes must be selected.
//! `crates/turborepo-trace/tests/affected_depcfg.rs` pins every field.
//! `TURBO_TRACE_DEBUG_CONFIG_NAMES=0` turns them off: a package named by one of
//! these configs then has unknown consumers again, except for a runner config,
//! which is seeded whole.

use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{
        Expression, IdentifierReference, PropertyKey, PropertyKind, StringLiteral,
        VariableDeclarator,
    },
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

use crate::configdiff::{Named, mention_at};

/// Whether the per-field rules are on (`TURBO_TRACE_DEBUG_CONFIG_NAMES=0` turns
/// them off).
pub fn enabled() -> bool {
    turborepo_trace_core::debug::var_os("CONFIG_NAMES").is_none_or(|v| v != "0")
}

/// The tools whose config readers the graph knows.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tool {
    /// `next.config.*`: a node that the app's build tasks read, and a content
    /// input of the jest tests of a `next/jest` project.
    Next,
    /// `jest.config.*`: every test of the project has a `jest:config` edge to
    /// it.
    Jest,
    /// `vitest.config.*`, `vite.config.*`, `vitest.workspace.*`: every test of
    /// the project has a `vitest:config` edge to it.
    Vite,
}

impl Tool {
    /// The tool that reads the config file named `file` (a base name), if it is
    /// one whose readers the graph knows.
    pub fn of(file: &str) -> Option<Tool> {
        let code = |f: &str, stem: &str| {
            f.strip_prefix(stem).is_some_and(|ext| {
                matches!(
                    ext,
                    "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "json" | "jsx" | "tsx"
                )
            })
        };
        if code(file, "next.config.") {
            Some(Tool::Next)
        } else if code(file, "jest.config.") {
            Some(Tool::Jest)
        } else if code(file, "vitest.config.")
            || code(file, "vite.config.")
            || code(file, "vitest.workspace.")
        {
            Some(Tool::Vite)
        } else {
            None
        }
    }

    /// The fields of this tool's config whose packages only their importers
    /// consume (see the module docs). `[]` is an array element, `{}` a
    /// property key.
    fn importer_fields(self) -> &'static [&'static str] {
        match self {
            Tool::Next => &[
                "transpilePackages.[]",
                "serverExternalPackages.[]",
                "experimental.serverComponentsExternalPackages.[]",
                "experimental.optimizePackageImports.[]",
                "modularizeImports.{}",
                "modularizeImports.*.transform",
            ],
            Tool::Jest => &["transformIgnorePatterns.[]"],
            Tool::Vite => &[
                "test.deps.inline.[]",
                "test.server.deps.inline.[]",
                "test.server.deps.external.[]",
                "test.deps.optimizer.web.include.[]",
                "test.deps.optimizer.web.exclude.[]",
                "test.deps.optimizer.ssr.include.[]",
                "test.deps.optimizer.ssr.exclude.[]",
                "optimizeDeps.include.[]",
                "optimizeDeps.exclude.[]",
                "ssr.noExternal.[]",
                "ssr.external.[]",
            ],
        }
    }

    /// Prefixes under which the same fields are written per project.
    fn project_prefixes(self) -> &'static [&'static str] {
        match self {
            Tool::Next => &[],
            Tool::Jest => &["projects.[]."],
            Tool::Vite => &["test.projects.[].", "projects.[]."],
        }
    }

    /// Whether `field` is one of [`Tool::importer_fields`], directly or per
    /// project. A `*` segment of a field stands for any one key.
    fn importers(self, field: &str) -> bool {
        let is = |field: &str| {
            self.importer_fields().iter().any(|f| {
                let (mut a, mut b) = (f.split('.'), field.split('.'));
                loop {
                    match (a.next(), b.next()) {
                        (None, None) => return true,
                        (Some(x), Some(y)) if x == y || (x == "*" && !matches!(y, "[]" | "{}")) => {
                        }
                        _ => return false,
                    }
                }
            })
        };
        is(field)
            || self
                .project_prefixes()
                .iter()
                .any(|p| field.strip_prefix(p).is_some_and(is))
    }
}

/// Where one string of a config module is written.
enum Place {
    /// The specifier of an import, a re-export, `require(...)` or
    /// `import(...)`.
    Import,
    /// A field: the property path from the outermost object or array literal.
    Field(String),
    /// Anything else, with the innermost property name for display, if any.
    Other(String),
    /// An element of the list that initializes the `const` of this name (also
    /// through `.join(...)`, `.concat(...)`, `.filter(...)`): where the
    /// variable is used decides.
    Binding(String),
}

/// The place of a string literal with the span `lit` below `ancestors`
/// (outermost first; see the module docs). With `list`, `lit` is the span of an
/// expression that stands for a list of strings (a reference to a variable that
/// holds one).
fn place(ancestors: &[AstKind<'_>], lit: Span, list: bool) -> Place {
    let mut path: Vec<&str> = if list { vec!["[]"] } else { Vec::new() };
    let mut child = lit;
    let mut broken = false;
    // The array literal below was spread into the next one: its elements are that
    // one's.
    let mut spread = false;
    for (i, kind) in ancestors.iter().rev().enumerate() {
        // Some versions of the visitor enter the literal itself.
        if matches!(
            kind,
            AstKind::StringLiteral(_) | AstKind::IdentifierReference(_)
        ) && kind.span() == lit
        {
            continue;
        }
        // Whether the list built so far is still a list of the same elements one level
        // up.
        let list = path.last() == Some(&"[]");
        let _ = i;
        match kind {
            AstKind::ParenthesizedExpression(_)
            | AstKind::TSAsExpression(_)
            | AstKind::TSSatisfiesExpression(_)
            | AstKind::TSNonNullExpression(_)
            | AstKind::TSTypeAssertion(_)
            | AstKind::ObjectExpression(_) => {}
            AstKind::ArrayExpression(_) if spread => spread = false,
            AstKind::ArrayExpression(_) => path.push("[]"),
            // `[...list]`: the elements of a list, in the enclosing list. A template that
            // holds the list (`(?!${names.join('|')})`) is one string of the enclosing list.
            AstKind::SpreadElement(_) if list && !spread => spread = true,
            AstKind::TemplateLiteral(_) if list => spread = true,
            // `const names = [...]`: the uses of the variable decide.
            AstKind::VariableDeclarator(d)
                if list
                    && path.len() == 1
                    && d.init.as_ref().is_some_and(|x| x.span() == child)
                    && ancestors.iter().rev().nth(i + 1).is_some_and(
                        |k| matches!(k, AstKind::VariableDeclaration(v) if v.kind.is_const()),
                    ) =>
            {
                // Not exported (`export const`): another module could use it. A name in an
                // export list is a use like any other.
                let exported = ancestors
                    .iter()
                    .rev()
                    .skip(i + 1)
                    .any(|k| matches!(k, AstKind::ExportDeclaration(_)));
                return match d.id.get_identifier_name() {
                    Some(name) if !exported => Place::Binding(name.to_string()),
                    _ => Place::Other("[]".to_owned()),
                };
            }
            // `cond ? list : other`, `list || other`: the list or nothing of it.
            AstKind::ConditionalExpression(c) if list && c.test.span() != child => {}
            AstKind::LogicalExpression(_) if list => {}
            // `list.concat(more)` and `list.filter(keep)` hold elements of `list`, and
            // `other.concat(list)` holds every element of `list`.
            AstKind::StaticMemberExpression(m)
                if list
                    && m.object.span() == child
                    && matches!(m.property.name.as_str(), "concat" | "filter" | "join") => {}
            AstKind::CallExpression(c)
                if list
                    && matches!(&c.callee, Expression::StaticMemberExpression(m)
                        if (c.callee.span() == child
                            && matches!(m.property.name.as_str(), "concat" | "filter" | "join"))
                            || (m.property.name == "concat"
                                && c.arguments.iter().any(|a| a.span() == child))) =>
            {
                // A joined list is one string: an element of a list, or part of a template.
                if matches!(&c.callee, Expression::StaticMemberExpression(m)
                    if c.callee.span() == child && m.property.name == "join")
                {
                    spread = true;
                }
            }
            AstKind::ObjectProperty(pr) => {
                let key = pr.key.span();
                let is_key = key.start <= child.start && child.end <= key.end;
                if pr.computed || pr.kind != PropertyKind::Init || pr.method {
                    broken = true;
                    break;
                }
                if is_key {
                    path.push("{}");
                } else {
                    match &pr.key {
                        PropertyKey::StaticIdentifier(k) => path.push(k.name.as_str()),
                        PropertyKey::StringLiteral(k) => match k.value.as_str() {
                            Some(k) => path.push(k),
                            None => {
                                broken = true;
                                break;
                            }
                        },
                        _ => {
                            broken = true;
                            break;
                        }
                    }
                }
            }
            AstKind::ImportDeclaration(_)
            | AstKind::ExportAllDeclaration(_)
            | AstKind::ExportFromDeclaration(_)
            | AstKind::ImportExpression(_)
                if path.is_empty() =>
            {
                return Place::Import;
            }
            AstKind::CallExpression(c) if path.is_empty() => {
                let require = matches!(&c.callee, Expression::Identifier(i) if i.name == "require");
                let first = c.arguments.first().is_some_and(|a| a.span() == child);
                if require && first && c.arguments.len() == 1 {
                    return Place::Import;
                }
                break;
            }
            _ => break,
        }
        child = kind.span();
    }
    path.reverse();
    let named = path.iter().any(|s| !matches!(*s, "[]" | "{}"));
    if broken || spread || !named {
        return Place::Other(path.join("."));
    }
    Place::Field(path.join("."))
}

/// Finds the place of every string literal that holds one of the offsets.
struct Finder<'a, 'o> {
    stack: Vec<AstKind<'a>>,
    /// The byte offsets looked for, sorted.
    offsets: &'o [usize],
    /// Per offset found inside a string literal: its place.
    found: Vec<(usize, Place)>,
    /// Every reference to a variable, with its place as a list of strings.
    uses: Vec<(String, Place)>,
    /// The names variable declarators bind.
    declared: Vec<String>,
    /// The module names `eval`.
    eval: bool,
}

impl<'a> Visit<'a> for Finder<'a, '_> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        self.stack.push(kind);
    }

    fn leave_node(&mut self, _: AstKind<'a>) {
        self.stack.pop();
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        let (a, b) = (it.span.start as usize, it.span.end as usize);
        let from = self.offsets.partition_point(|&o| o < a);
        for &o in self.offsets[from..].iter().take_while(|&&o| o < b) {
            self.found.push((o, place(&self.stack, it.span, false)));
        }
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        // A direct `eval` can use a variable without naming it.
        self.eval |= it.name == "eval";
        self.uses
            .push((it.name.to_string(), place(&self.stack, it.span, true)));
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        if let Some(name) = it.id.get_identifier_name() {
            self.declared.push(name.to_string());
        }
        walk::walk_variable_declarator(self, it);
    }
}

/// Who consumes the package that the config file `file` (a base name of a
/// [`Tool`]'s config) with the text `src` names by one of `needles` (the
/// package name, and the short names the tool accepts for it). `links`: the
/// config has an edge to the package, so its import specifiers are accounted
/// for by the caller.
pub fn named(tool: Tool, file: &str, src: &[u8], needles: &[&str], links: bool) -> Named {
    let whole = |why: &str| Named::Readers(vec![why.to_owned()]);
    let Ok(text) = std::str::from_utf8(src) else {
        return whole("not UTF-8");
    };
    let mut offsets: Vec<usize> = Vec::new();
    for n in needles {
        let mut from = 0;
        while let Some(at) = mention_at(src, n, from) {
            offsets.push(at);
            from = at + 1;
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    // JSON is parsed as a parenthesized expression.
    let json = file.ends_with(".json");
    let (code, shift) = if json {
        (std::borrow::Cow::Owned(format!("({text}\n)")), 1usize)
    } else {
        (std::borrow::Cow::Borrowed(text), 0)
    };
    let st = if json {
        SourceType::mjs()
    } else {
        match SourceType::from_path(file) {
            Ok(st) => st,
            Err(_) => return whole("not a JS or TS file"),
        }
    };
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, &code, st).parse();
    if ret.fatal_error || !ret.diagnostics.is_empty() || ret.is_flow_language {
        return whole("the file does not parse");
    }
    let comments: Vec<(usize, usize)> = ret
        .program
        .comments
        .iter()
        .map(|c| (c.span.start as usize, c.span.end as usize))
        .collect();
    let shifted: Vec<usize> = offsets.iter().map(|o| o + shift).collect();
    let mut finder = Finder {
        stack: Vec::new(),
        offsets: &shifted,
        found: Vec::new(),
        uses: Vec::new(),
        declared: Vec::new(),
        eval: false,
    };
    finder.visit_program(&ret.program);
    let mut importers: Vec<String> = Vec::new();
    let mut readers: Vec<String> = Vec::new();
    for &at in &shifted {
        if comments.iter().any(|&(a, b)| a <= at && at < b) {
            continue;
        }
        let Some(i) = finder.found.iter().position(|(o, _)| *o == at) else {
            // A template, a regular expression, an identifier: not a field.
            if !readers.iter().any(|r| r == "outside a string") {
                readers.push("outside a string".to_owned());
            }
            continue;
        };
        let found = match finder.found.swap_remove(i).1 {
            // A list in a variable: one binding is followed. Every use of the variable
            // must put the list into a field only importers consume.
            Place::Binding(name) => {
                let uses: Vec<&Place> = finder
                    .uses
                    .iter()
                    .filter(|(n, _)| *n == name)
                    .map(|(_, p)| p)
                    .collect();
                let once =
                    finder.declared.iter().filter(|n| **n == name).count() == 1 && !finder.eval;
                let fields: Vec<&String> = uses
                    .iter()
                    .filter_map(|p| match p {
                        Place::Field(f) if tool.importers(f) => Some(f),
                        _ => None,
                    })
                    .collect();
                if once && fields.len() == uses.len() {
                    for f in fields {
                        let what = f.replace(".[]", "[]").replace(".{}", ".<package>");
                        let what = format!("{what} (through the variable {name})");
                        if !importers.contains(&what) {
                            importers.push(what);
                        }
                    }
                    continue;
                }
                Place::Other(format!("the variable {name}"))
            }
            p => p,
        };
        let (list, what) = match found {
            Place::Binding(name) => (&mut readers, format!("the variable {name}")),
            Place::Import if links => continue,
            Place::Import => (
                &mut readers,
                "an import the graph has no edge for".to_owned(),
            ),
            Place::Field(f) if tool.importers(&f) => (&mut importers, f),
            Place::Field(f) => (&mut readers, f),
            Place::Other(f) if f.is_empty() => (&mut readers, "outside a field".to_owned()),
            Place::Other(f) => (&mut readers, f),
        };
        let what = what.replace(".[]", "[]").replace(".{}", ".<package>");
        if !list.contains(&what) {
            list.push(what);
        }
    }
    if readers.is_empty() {
        Named::Importers(importers)
    } else {
        Named::Readers(readers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next(src: &str, name: &str) -> Named {
        named(
            Tool::Next,
            "next.config.mjs",
            src.as_bytes(),
            &[name],
            false,
        )
    }

    fn importers(fields: &[&str]) -> Named {
        Named::Importers(fields.iter().map(|s| (*s).to_owned()).collect())
    }

    fn readers(fields: &[&str]) -> Named {
        Named::Readers(fields.iter().map(|s| (*s).to_owned()).collect())
    }

    /// The shape of a real app config: an object literal in a variable, handed
    /// to wrappers of several packages before it is exported.
    const WRAPPED: &str = r#"
import path from 'node:path';
import { withPlugin } from 'plugin/next';
import { withOther } from '@acme/other';

const isDev = process.env.NODE_ENV === 'development';

const nextConfig = {
  reactStrictMode: true,
  transpilePackages:
    // packages with untranspiled code
    [
      '@acme/ui',
      '@flow/web-shared',
      'react-tweet',
    ]
      .concat(
        isDev
          ? []
          : // bundled for production
            [
              'prod-only',
              ...(process.env.EXTRA ? ['extra-lib'] : []),
            ],
      )
      .filter(Boolean),
  serverExternalPackages: ['@acme/native'],
  experimental: {
    optimizePackageImports: ['@acme/icons', 'date-lib'],
    serverComponentsExternalPackages: ['legacy-native'],
    swcPlugins: [['swc-plugin-x', { mode: 'a' }]],
  },
  modularizeImports: {
    'lodash-lib': { transform: 'lodash-lib/{{member}}' },
  },
  images: { loaderFile: 'loader-pkg/next' },
  async rewrites() {
    return [{ source: '/a', destination: 'https://routed-pkg.example/a' }];
  },
};
Object.assign(nextConfig.experimental, { optimizePackageImports: ['late-lib'] });

let config = withOther(withPlugin(nextConfig));
if (process.env.TOOLBAR !== 'off') {
  config = require('toolbar-pkg/next')()(config);
}
export default config;
"#;

    #[test]
    fn next_fields_that_only_importers_consume() {
        assert_eq!(
            next(WRAPPED, "@flow/web-shared"),
            importers(&["transpilePackages[]"])
        );
        assert_eq!(
            next(WRAPPED, "react-tweet"),
            importers(&["transpilePackages[]"])
        );
        // The lists the value is put together from: a `concat` argument under a
        // condition, a spread list inside it.
        assert_eq!(
            next(WRAPPED, "prod-only"),
            importers(&["transpilePackages[]"])
        );
        assert_eq!(
            next(WRAPPED, "extra-lib"),
            importers(&["transpilePackages[]"])
        );
        assert_eq!(
            next(WRAPPED, "@acme/native"),
            importers(&["serverExternalPackages[]"])
        );
        assert_eq!(
            next(WRAPPED, "date-lib"),
            importers(&["experimental.optimizePackageImports[]"])
        );
        assert_eq!(
            next(WRAPPED, "legacy-native"),
            importers(&["experimental.serverComponentsExternalPackages[]"])
        );
        // The key of `modularizeImports`, and the rewritten specifier that names the
        // same package.
        assert_eq!(
            next(WRAPPED, "lodash-lib"),
            importers(&[
                "modularizeImports.<package>",
                "modularizeImports.lodash-lib.transform"
            ])
        );
    }

    #[test]
    fn every_other_position_is_read_by_the_tool() {
        // A compiler plugin runs on every file.
        assert_eq!(
            next(WRAPPED, "swc-plugin-x"),
            readers(&["experimental.swcPlugins[][]"])
        );
        // A field this module does not decide.
        assert_eq!(next(WRAPPED, "loader-pkg"), readers(&["images.loaderFile"]));
        // The same key outside the config's own nesting (an argument of a call).
        assert_eq!(
            next(WRAPPED, "late-lib"),
            readers(&["optimizePackageImports[]"])
        );
        // Not a field at all.
        for src in [
            // A variable that something else uses too, that is exported, or that is
            // declared twice.
            "const list = ['pkg'];\nconsole.log(list);\nexport default { transpilePackages: list \
             };",
            "const list = ['pkg'];\nexport default { transpilePackages: list, env: { L: \
             list.join() } };",
            "export const list = ['pkg'];\nexport default { transpilePackages: list };",
            "const list = ['pkg'];\n{ const list = []; }\nexport default { transpilePackages: \
             list };",
            "let list = ['pkg'];\nexport default { transpilePackages: list };",
            "const list = ['pkg'];\nconst other = list;\nexport default { transpilePackages: \
             other };",
            "const list = ['pkg'];\nexport default { images: { domains: list } };",
            "export default { transpilePackages: [...base, `pkg`] };",
            "export default { transpilePackages: [flag ? 'pkg' : 'other'] };",
            "export default { transpilePackages: ['a'].map((x) => x + 'pkg') };",
            "export default { transpilePackages: ['pkg'].map(fix) };",
            "export default { transpilePackages: ['pkg'].join(',') };",
            "export default { transpilePackages: other.concat('pkg') };",
            "export default { transpilePackages: ['pkg' === name ? 'a' : 'b'] };",
            "export default { transpilePackages: [['pkg'] ? 'a' : 'b'] };",
            "export default { transpilePackages: load(['pkg']) };",
            "export default { transpilePackages: [...load(['pkg'])] };",
            "export default { transpilePackages: (['pkg'].length ? [] : []) };",
            "export default { x: [...['pkg']].concat };",
            "export default { transpilePackages: [name('pkg')] };",
            "export default { ['transpilePackages']: ['pkg'] };",
            "export default { get transpilePackages() { return ['pkg']; } };",
            "export default { webpack(config) { config.externals.push('pkg'); return config; } };",
            "export default withTM(['pkg'])({});",
            "export default { transpilePackages: ['pkg' };",
        ] {
            assert!(
                matches!(next(src, "pkg"), Named::Readers(_)),
                "{src}: {:?}",
                next(src, "pkg")
            );
        }
        // Lists put together in ways that keep the elements.
        for src in [
            "export default { transpilePackages: ['pkg'].concat(more) };",
            "export default { transpilePackages: base.concat(['pkg']) };",
            "export default { transpilePackages: [...base, ...['pkg']] };",
            "export default { transpilePackages: flag ? ['pkg'] : [] };",
            "export default { transpilePackages: (flag && ['pkg']) || [] };",
            "export default { transpilePackages: ['pkg', flag && 'x'].filter(Boolean) };",
            "export default { transpilePackages: [...(flag ? ['pkg'] : [])] };",
        ] {
            assert_eq!(
                next(src, "pkg"),
                importers(&["transpilePackages[]"]),
                "{src}"
            );
        }
        // One binding is followed.
        for src in [
            "const list = ['pkg'];\nexport default { transpilePackages: list };",
            "const list = ['pkg', 'other'];\nexport default withX({ transpilePackages: [...list, \
             'more'] });",
            "const list = ['pkg'].concat(extra);\nconst nextConfig = { transpilePackages: flag ? \
             list : [] };\nexport default nextConfig;",
        ] {
            assert_eq!(
                next(src, "pkg"),
                importers(&["transpilePackages[] (through the variable list)"]),
                "{src}"
            );
        }
        // A string that says `eval` is not a call of it; a call is.
        assert_eq!(
            next(
                "const csp = \"script-src 'unsafe-eval'\";\nconst list = ['pkg'];\nexport default \
                 { transpilePackages: list };",
                "pkg"
            ),
            importers(&["transpilePackages[] (through the variable list)"])
        );
        assert!(matches!(
            next(
                "const list = ['pkg'];\neval('list.push(1)');\nexport default { \
                 transpilePackages: list };",
                "pkg"
            ),
            Named::Readers(_)
        ));
        // Named by a variable nothing uses: no field holds it.
        assert_eq!(
            next("const list = ['pkg'];\nexport default {};", "pkg"),
            importers(&[])
        );
        // TypeScript wrappers are transparent.
        let ts = "export default { transpilePackages: (['pkg'] as string[]).concat(more) } \
                  satisfies Config;";
        assert_eq!(
            named(Tool::Next, "next.config.ts", ts.as_bytes(), &["pkg"], false),
            importers(&["transpilePackages[]"])
        );
        // A string of one decided field and one other use: the other use decides.
        let both = "export default { transpilePackages: ['pkg'], env: { X: 'pkg' } };";
        assert_eq!(next(both, "pkg"), readers(&["env.X"]));
    }

    #[test]
    fn imports_and_comments() {
        // An import is an edge: with the edge in the graph it is the caller's business.
        let linked = |name| {
            named(
                Tool::Next,
                "next.config.mjs",
                WRAPPED.as_bytes(),
                &[name],
                true,
            )
        };
        assert_eq!(linked("plugin"), importers(&[]));
        assert_eq!(linked("@acme/other"), importers(&[]));
        assert_eq!(linked("toolbar-pkg"), importers(&[]));
        assert!(matches!(next(WRAPPED, "plugin"), Named::Readers(_)));
        // A re-export is an edge too.
        let src = "export { helper } from 'pkg/helper';\nexport default {};";
        assert_eq!(
            named(
                Tool::Next,
                "next.config.mjs",
                src.as_bytes(),
                &["pkg"],
                true
            ),
            importers(&[])
        );
        // `require.resolve` and other calls are not imports.
        let src = "module.exports = { x: require.resolve('pkg') };";
        assert!(matches!(
            named(Tool::Next, "next.config.js", src.as_bytes(), &["pkg"], true),
            Named::Readers(_)
        ));
        // Only a comment names it.
        let src = "// pkg used to be transpiled\nexport default { transpilePackages: ['other'] };";
        assert_eq!(next(src, "pkg"), importers(&[]));
        // Part of a longer name is no mention.
        assert_eq!(next(WRAPPED, "tweet"), importers(&[]));
    }

    #[test]
    fn jest_and_vite_fields() {
        let jest = |src: &str, file: &str| named(Tool::Jest, file, src.as_bytes(), &["pkg"], false);
        let src =
            "module.exports = { transformIgnorePatterns: ['/node_modules/(?!(pkg|other)/)'] };";
        assert_eq!(
            jest(src, "jest.config.js"),
            importers(&["transformIgnorePatterns[]"])
        );
        let src = "export default { projects: [{ transformIgnorePatterns: \
                   ['node_modules/(?!pkg/)'] }] };";
        assert_eq!(
            jest(src, "jest.config.ts"),
            importers(&["projects[].transformIgnorePatterns[]"])
        );
        let src = r#"{ "transformIgnorePatterns": ["/node_modules/(?!pkg)"], "testEnvironment": "node" }"#;
        assert_eq!(
            jest(src, "jest.config.json"),
            importers(&["transformIgnorePatterns[]"])
        );
        // The usual way to write the list of packages that are let through the
        // transform.
        for src in [
            "const esModules = ['pkg', 'other'].join('|');\nmodule.exports = { \
             transformIgnorePatterns: [`/node_modules/(?!${esModules})`] };",
            "const esModules = ['pkg', 'other'];\nmodule.exports = { transformIgnorePatterns: \
             [`/node_modules/(?!(${esModules.join('|')})/)`, '\\.css$'] };",
            "const esModules = ['pkg'];\nconst config = { transformIgnorePatterns: \
             ['/node_modules/(?!(' + 'x' + ')/)', `node_modules/(?!${esModules.join('|')})`] \
             };\nmodule.exports = config;",
        ] {
            assert_eq!(
                jest(src, "jest.config.js"),
                importers(&["transformIgnorePatterns[] (through the variable esModules)"]),
                "{src}"
            );
        }
        for src in [
            "const esModules = ['pkg'].join('|');\nmodule.exports = { transformIgnorePatterns: \
             [`/node_modules/(?!${esModules})`], testEnvironment: esModules };",
            "const esModules = ['pkg'].join('|');\nmodule.exports = { testPathIgnorePatterns: \
             [`/node_modules/(?!${esModules})`] };",
        ] {
            assert!(
                matches!(jest(src, "jest.config.js"), Named::Readers(_)),
                "{src}"
            );
        }
        for src in [
            "module.exports = { testEnvironment: 'pkg' };",
            "module.exports = { transform: { '\\\\.tsx?$': ['pkg', {}] } };",
            "module.exports = { moduleNameMapper: { '^react$': 'pkg/compat' } };",
            "module.exports = { setupFiles: ['pkg/register'] };",
            "module.exports = { preset: 'pkg' };",
        ] {
            assert!(
                matches!(jest(src, "jest.config.js"), Named::Readers(_)),
                "{src}"
            );
        }
        let vite = |src: &str| {
            named(
                Tool::Vite,
                "vitest.config.ts",
                src.as_bytes(),
                &["pkg"],
                false,
            )
        };
        for (src, field) in [
            (
                "export default defineConfig({ test: { server: { deps: { inline: ['pkg'] } } } });",
                "test.server.deps.inline[]",
            ),
            (
                "export default defineConfig({ test: { deps: { inline: ['pkg', /x/] } } });",
                "test.deps.inline[]",
            ),
            (
                "export default { optimizeDeps: { exclude: ['pkg'] } };",
                "optimizeDeps.exclude[]",
            ),
            (
                "export default mergeConfig(base, { ssr: { noExternal: ['pkg'] } });",
                "ssr.noExternal[]",
            ),
            (
                "export default defineConfig({ test: { projects: [{ test: { deps: { inline: \
                 ['pkg'] } } }] } });",
                "test.projects[].test.deps.inline[]",
            ),
        ] {
            assert_eq!(vite(src), importers(&[field]), "{src}");
        }
        for src in [
            "export default { test: { environment: 'pkg' } };",
            "export default { resolve: { dedupe: ['pkg'] } };",
            "export default { resolve: { alias: { x: 'pkg' } } };",
            "export default { plugins: [react({ jsxImportSource: 'pkg' })] };",
            "export default { test: { include: ['pkg'] } };",
        ] {
            assert!(matches!(vite(src), Named::Readers(_)), "{src}");
        }
    }

    #[test]
    fn tools_by_file_name() {
        assert_eq!(Tool::of("next.config.mjs"), Some(Tool::Next));
        assert_eq!(Tool::of("next.config.ts"), Some(Tool::Next));
        assert_eq!(Tool::of("jest.config.cjs"), Some(Tool::Jest));
        assert_eq!(Tool::of("jest.config.json"), Some(Tool::Jest));
        assert_eq!(Tool::of("vitest.config.mts"), Some(Tool::Vite));
        assert_eq!(Tool::of("vite.config.js"), Some(Tool::Vite));
        assert_eq!(Tool::of("vitest.workspace.ts"), Some(Tool::Vite));
        for f in [
            "babel.config.js",
            ".babelrc",
            ".swcrc",
            "postcss.config.mjs",
            "eslint.config.js",
            "tsconfig.json",
            "next.config.md",
            "jest.config.base.js",
        ] {
            assert_eq!(Tool::of(f), None, "{f}");
        }
    }
}
