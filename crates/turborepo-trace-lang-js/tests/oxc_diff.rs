//! Differential test: our extractor vs `oxc_parser`'s AST.
//!
//! For every `.js/.mjs/.cjs/.jsx/.ts/.mts/.cts/.tsx` file under the fixture
//! tree (and, if set, under `TURBO_TRACE_DEBUG_DIFF_DIR`, for example a
//! `node_modules` directory), compares the multiset of literal specifiers per
//! category:
//!
//! | category | ours | oxc |
//! |---|---|---|
//! | `static` | `Import` (except `/// <reference>`), `ReExport` | `ImportDeclaration`, `ExportNamedDeclaration` with source, `ExportAllDeclaration` |
//! | `require` | `Require` | `require('x')`, `require.resolve('x')`, `module.require('x')` calls, TS `import x = require('x')` |
//! | `dynamic` | `DynamicImport` | `import('x')`, TS `import('x')` types |
//!
//! Files oxc cannot parse without errors (tried as module, then as script with
//! top-level `return` allowed) or that oxc detects as Flow are ignored.
//!
//! The `require` category is not compared for a file in which a `require`
//! function may search from another place ([`resolves_elsewhere`]:
//! `createRequire(x)` with `x` not literally the file, a `resolve` call with
//! options): our reference then names what the call resolves from there
//! (`./sub/lib.node` for `require('./lib.node')` under `createRequire(new
//! URL('./sub/', import.meta.url))`), not the literal oxc sees.
//!
//! ```sh
//! cargo test -p turborepo-trace-lang-js --test oxc_diff                       # fixtures only
//! TURBO_TRACE_DEBUG_DIFF_DIR=/tmp/x/node_modules \
//!   cargo test --release -p turborepo-trace-lang-js --test oxc_diff -- --nocapture
//! ```
//!
//! It also counts lexer recoveries: files that oxc parses without errors and on
//! which our lexer reports `parse_incomplete` (`...; lexer recovered`). Each
//! one is a lexer bug, and keeps every test that reaches the file selected.
//!
//! `TURBO_TRACE_DEBUG_DIFF_ALLOW=N` tolerates up to `N` mismatching files
//! (default 0), `TURBO_TRACE_DEBUG_DIFF_ALLOW_RECOVERIES=N` up to `N` files
//! with a lexer recovery (default 0); `TURBO_TRACE_DEBUG_DIFF_REPORT=path`
//! writes every mismatch and recovery to a file.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression,
    ImportDeclaration, ImportExpression, TSImportEqualsDeclaration, TSImportType,
    TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use turborepo_trace_core::{RefKind, Specifier, UncertaintyKind};
use turborepo_trace_lang_js::{Dialect, ScanOptions, scan};

const EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"];

type Refs = Vec<(&'static str, String)>;

/// Our references, and the lexer's `parse_incomplete` diagnostic if it
/// recovered.
fn ours(path: &Path, src: &[u8]) -> (Refs, Option<String>) {
    let ex = scan(src, Dialect::from_path(path), ScanOptions::default());
    let recovered = ex
        .diagnostics
        .iter()
        .find(|d| d.kind == UncertaintyKind::ParseIncomplete)
        .map(|d| d.detail.to_string());
    let mut out: Refs = ex
        .refs
        .iter()
        .filter_map(|r| {
            let Specifier::Literal(s) = &r.spec else {
                return None;
            };
            let cat = match r.kind {
                RefKind::Import => {
                    // `/// <reference>` spans start at the quoted value.
                    if !r.span.slice(src)?.starts_with(b"import") {
                        return None;
                    }
                    "static"
                }
                RefKind::ReExport => "static",
                // oxc only sees direct literal arguments of the real loaders; evaluated
                // specifiers (`require('u' + 'rl')`) and aliases (`r('x')`) are M2 extras.
                RefKind::Require if direct_literal_call(r.span.slice(src)?) => "require",
                RefKind::DynamicImport if direct_literal_call(r.span.slice(src)?) => "dynamic",
                _ => return None,
            };
            Some((cat, s.to_string()))
        })
        .collect();
    out.sort();
    (out, recovered)
}

/// Whether a call span is `require|require.resolve|module.require|import (
/// <literal> ...` (optionally parenthesized), the shape oxc's collector counts.
fn direct_literal_call(text: &[u8]) -> bool {
    let Some(lp) = text.iter().position(|&b| b == b'(') else {
        return false;
    };
    let callee: Vec<u8> = text[..lp]
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    // TS `import x = require('y')` spans start at `import`.
    let import_equals = callee.starts_with(b"import") && callee.ends_with(b"=require");
    if !import_equals
        && !matches!(
            callee.as_slice(),
            b"require" | b"require.resolve" | b"module.require" | b"import"
        )
    {
        return false;
    }
    let mut i = lp + 1;
    loop {
        match (text.get(i), text.get(i + 1)) {
            (Some(b), _) if b.is_ascii_whitespace() || *b == b'(' => i += 1,
            (Some(b'/'), Some(b'/')) => {
                i += text[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .unwrap_or(text.len() - i);
            }
            (Some(b'/'), Some(b'*')) => {
                let Some(end) = text[i + 2..].windows(2).position(|w| w == b"*/") else {
                    return false;
                };
                i += 2 + end + 2;
            }
            _ => break,
        }
    }
    let Some(&q @ (b'"' | b'\'' | b'`')) = text.get(i) else {
        return false;
    };
    i += 1;
    loop {
        match text.get(i) {
            None => return false,
            Some(b'\\') => i += 2,
            Some(b'$') if q == b'`' && text.get(i + 1) == Some(&b'{') => return false,
            Some(&c) if c == q => break,
            Some(_) => i += 1,
        }
    }
    i += 1;
    while text
        .get(i)
        .is_some_and(|b| b.is_ascii_whitespace() || *b == b')')
    {
        i += 1;
    }
    matches!(text.get(i), None | Some(b','))
}

/// An oxc `JSStr` (lone surrogates displayed as U+FFFD) as a `String`.
macro_rules! js_str {
    ($e:expr) => {
        $e.display().to_string()
    };
}

#[derive(Default)]
struct Collector {
    refs: Refs,
}

fn literal_arg(arg: Option<&Argument<'_>>) -> Option<String> {
    match arg? {
        Argument::StringLiteral(s) => Some(js_str!(s.value)),
        Argument::TemplateLiteral(t) if t.is_no_substitution_template() => {
            t.single_quasi().map(|q| js_str!(q))
        }
        _ => None,
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.refs.push(("static", js_str!(it.source.value)));
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        self.refs.push(("static", js_str!(it.source.value)));
        walk::walk_export_from_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        self.refs.push(("static", js_str!(it.source.value)));
        walk::walk_export_all_declaration(self, it);
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(m) = &it.module_reference {
            self.refs.push(("require", js_str!(m.expression.value)));
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        let lit = match &it.source {
            Expression::StringLiteral(s) => Some(js_str!(s.value)),
            Expression::TemplateLiteral(t) if t.is_no_substitution_template() => {
                t.single_quasi().map(|q| js_str!(q))
            }
            _ => None,
        };
        if let Some(s) = lit {
            self.refs.push(("dynamic", s));
        }
        walk::walk_import_expression(self, it);
    }

    fn visit_ts_import_type(&mut self, it: &TSImportType<'a>) {
        self.refs.push(("dynamic", js_str!(it.source.value)));
        walk::walk_ts_import_type(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let is_require = match &it.callee {
            Expression::Identifier(id) => id.name == "require",
            Expression::StaticMemberExpression(m) => {
                matches!(
                    (&m.object, m.property.name.as_str()),
                    (Expression::Identifier(o), "resolve") if o.name == "require"
                ) || matches!(
                    (&m.object, m.property.name.as_str()),
                    (Expression::Identifier(o), "require") if o.name == "module"
                )
            }
            _ => false,
        };
        if is_require
            && !it.optional
            && let Some(s) = literal_arg(it.arguments.first())
        {
            self.refs.push(("require", s));
        }
        walk::walk_call_expression(self, it);
    }
}

/// `None` if oxc cannot parse the file cleanly.
fn oxc(path: &Path, src: &str) -> Option<Refs> {
    let base = SourceType::from_path(path).ok()?;
    let mut attempts = vec![base];
    if base.is_javascript() {
        attempts.push(base.with_script(true));
    }
    for st in attempts {
        let alloc = Allocator::default();
        let ret = Parser::new(&alloc, src, st)
            .with_options(ParseOptions {
                allow_return_outside_function: true,
                preserve_parens: false,
                ..ParseOptions::default()
            })
            .parse();
        if ret.is_flow_language {
            return None;
        }
        if ret.fatal_error || !ret.diagnostics.is_empty() {
            continue;
        }
        let mut c = Collector::default();
        c.visit_program(&ret.program);
        c.refs.sort();
        return Some(c.refs);
    }
    None
}

/// Whether `src` has a call whose callee decides where a request is resolved
/// from: `createRequire(x)` with `x` other than the file itself, or
/// `….resolve(request, options)`. A text scan (comments and strings count: it
/// only makes the comparison skip more).
fn resolves_elsewhere(src: &str) -> bool {
    // The text of the call arguments starting right after a `(`, and whether a `,`
    // separates two of them.
    fn args(rest: &str) -> (&str, bool) {
        let (mut depth, mut comma) = (0usize, false);
        for (i, c) in rest.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' if depth == 0 => return (&rest[..i], comma),
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => comma = true,
                _ => {}
            }
        }
        (rest, comma)
    }
    let own = ["import.meta.url", "import.meta.filename", "__filename"];
    src.match_indices("createRequire(")
        .any(|(i, m)| !own.contains(&args(&src[i + m.len()..]).0.trim()))
        || src
            .match_indices(".resolve(")
            .any(|(i, m)| args(&src[i + m.len()..]).1)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_files(&path, out);
        } else if ft.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTENSIONS.contains(&e))
        {
            out.push(path);
        }
    }
}

#[derive(Default)]
struct Stats {
    files: usize,
    compared: usize,
    oxc_failed: usize,
    refs: usize,
    mismatches: BTreeMap<PathBuf, (Refs, Refs)>,
    /// Files oxc parses cleanly on which our lexer recovered, with the
    /// diagnostic.
    recoveries: BTreeMap<PathBuf, String>,
}

/// Multiset difference `a - b`.
fn minus(a: &Refs, b: &Refs) -> Refs {
    let mut b = b.clone();
    let mut out = Vec::new();
    for x in a {
        if let Some(i) = b.iter().position(|y| y == x) {
            b.swap_remove(i);
        } else {
            out.push(x.clone());
        }
    }
    out
}

fn run(dirs: &[PathBuf]) -> Stats {
    let mut files = Vec::new();
    for d in dirs {
        collect_files(d, &mut files);
    }
    files.sort();
    let stats = Mutex::new(Stats {
        files: files.len(),
        ..Stats::default()
    });
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = files.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in files.chunks(chunk) {
            let stats = &stats;
            s.spawn(move || {
                for path in part {
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    let Ok(text) = std::str::from_utf8(&bytes) else {
                        stats.lock().unwrap().oxc_failed += 1;
                        continue;
                    };
                    let mut theirs = oxc(path, text);
                    let (mut mine, recovered) = ours(path, &bytes);
                    if resolves_elsewhere(text) {
                        mine.retain(|r| r.0 != "require");
                        if let Some(t) = &mut theirs {
                            t.retain(|r| r.0 != "require");
                        }
                    }
                    let mut st = stats.lock().unwrap();
                    match theirs {
                        None => st.oxc_failed += 1,
                        Some(theirs) => {
                            st.compared += 1;
                            st.refs += theirs.len();
                            if mine != theirs {
                                st.mismatches.insert(path.clone(), (mine, theirs));
                            }
                            if let Some(detail) = recovered {
                                st.recoveries.insert(path.clone(), detail);
                            }
                        }
                    }
                }
            });
        }
    });
    stats.into_inner().unwrap()
}

#[test]
fn matches_oxc() {
    let mut dirs = vec![PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../turborepo-trace-testkit/fixtures/js"
    ))];
    if let Some(d) = std::env::var_os("TURBO_TRACE_DEBUG_DIFF_DIR") {
        dirs.push(PathBuf::from(d));
    }
    let stats = run(&dirs);
    let mut report = String::new();
    for (path, (mine, theirs)) in &stats.mismatches {
        writeln!(report, "{}", path.display()).unwrap();
        for (c, s) in minus(mine, theirs) {
            writeln!(report, "  only ours: {c} {s:?}").unwrap();
        }
        for (c, s) in minus(theirs, mine) {
            writeln!(report, "  only oxc:  {c} {s:?}").unwrap();
        }
    }
    for (path, detail) in &stats.recoveries {
        writeln!(report, "{}\n  lexer recovery: {detail}", path.display()).unwrap();
    }
    if let Some(p) = std::env::var_os("TURBO_TRACE_DEBUG_DIFF_REPORT") {
        std::fs::write(p, &report).unwrap();
    }
    println!(
        "oxc diff: {} files, {} compared ({} oxc parse failures/flow skipped), {} oxc refs, {} \
         mismatching files, {} files with a lexer recovery",
        stats.files,
        stats.compared,
        stats.oxc_failed,
        stats.refs,
        stats.mismatches.len(),
        stats.recoveries.len()
    );
    let shown: String = report.lines().take(200).collect::<Vec<_>>().join("\n");
    println!("{shown}");
    let limit = |var: &str| -> usize {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let allow = limit("TURBO_TRACE_DEBUG_DIFF_ALLOW");
    let allow_recoveries = limit("TURBO_TRACE_DEBUG_DIFF_ALLOW_RECOVERIES");
    assert!(stats.compared > 0, "no files compared");
    assert!(
        stats.mismatches.len() <= allow,
        "{} files differ from oxc (allowed {allow}):\n{shown}",
        stats.mismatches.len()
    );
    assert!(
        stats.recoveries.len() <= allow_recoveries,
        "the lexer recovered on {} files that oxc parses (allowed {allow_recoveries}):\n{shown}",
        stats.recoveries.len()
    );
}
