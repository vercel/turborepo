//! JS / TS / JSX / TSX language plugin of the tracer.
//!
//! Extraction: a hand-written byte-oriented lexer ([`lexer`]) feeds a
//! recognizer ([`extract`]) that finds static and CommonJS module references
//! (`import`, `export ... from`, `import()`, `require()`, `require.resolve()`,
//! `module.require()`, TS `import x = require()`, triple-slash `///
//! <reference>`), evaluates dynamic arguments (concatenation, templates,
//! ternaries, constants, `__dirname`, `path.*`, `new URL(x, import.meta.url)`)
//! into literals, patterns and alternatives, and tracks runtime file APIs (`fs`
//! reads, `new Worker`, `child_process.fork`) through import bindings. Files
//! without trigger words are skipped by a keyword prefilter. See the module
//! docs for the exact forms and flags.
//!
//! Resolution: [`resolve::JsResolver`], `oxc_resolver` on the tracer's shared
//! fs cache, with Node/TypeScript semantics, builtins, manifests and externals.
//! See the [`resolve`] docs.
//!
//! [`wgsl::WgslLanguage`] is a second, small language of this crate: shader
//! modules (`.wgsl`) that JS/TS modules import and that import each other with
//! ES import lines, resolved by the same resolver.

// Every public item is documented (the workspace lint, kept per crate here).
#![warn(missing_docs)]

pub mod extract;
pub mod flow;
pub mod lexer;
pub mod mocks;
pub mod names;
pub mod resolve;
pub mod special;
pub mod symbols;
pub mod wgsl;
pub mod xfile;

use std::path::Path;

pub use extract::{Binding, BindingSource, Extraction, Imported, ScanOptions, scan, scan_named};
pub use lexer::{Dialect, Jsx};
use turborepo_trace_core::{ExtractCx, LangId, Language, ProjectCx, RefSink, Resolver};

/// `TURBO_TRACE_DEBUG_ANCHORS=0` turns following anchors off (for A/B
/// measurements; graphs built either way are not interchangeable): a file that
/// uses its location or the working directory then names everything below it.
fn anchors_enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        turborepo_trace_core::debug::var_os("ANCHORS").is_none_or(|v| v != "0")
    });
    *ENABLED
}

/// An escaping path with an environment tail (`flow::anchors::EnvEscape`), as
/// the environment registry sees a read: a use with the value as a template,
/// and a reference with the record the registry resolves, relabels or leaves.
fn escape_use(src: &[u8], e: &flow::anchors::EnvEscape, out: &mut RefSink<'_>) {
    use turborepo_trace_core::{Origin, RawRef, RefKind, Specifier, Uncertainty, UncertaintyKind};
    let kind = RefKind::FsRead;
    let line = 1 + memchr::memchr_iter(b'\n', &src[..(e.span.start as usize).min(src.len())])
        .count() as u32;
    out.fact(
        turborepo_trace_core::env::EnvFact::Use {
            span: [e.span.start, e.span.end],
            line,
            kind: RefKind::ALL.iter().position(|k| *k == kind).unwrap_or(0) as u8,
            origin: Origin::ALL.iter().position(|o| *o == e.origin).unwrap_or(0) as u8,
            value: e.value.clone(),
            from: turborepo_trace_core::env::EnvFrom::File,
            shared: false,
        }
        .encode(),
    );
    let record = if e.origin == Origin::Env {
        UncertaintyKind::EnvDependent
    } else {
        UncertaintyKind::ComputedSpecifier
    };
    let mut r = RawRef::new(kind, Specifier::Unknown, e.span);
    r.uncertainty = Some(Uncertainty::new(record, e.span, e.detail.as_str()).with_origin(e.origin));
    out.push(r);
}

/// The import that a `@jsxImportSource <module>` pragma stands for: with the
/// automatic JSX runtime the transpiler loads the factory from
/// `<module>/jsx-runtime` (the development transform from
/// `<module>/jsx-dev-runtime`, which re-exports or mirrors it). The span is the
/// module's name in the comment. Only the first pragma counts, as for a
/// transpiler.
fn jsx_import_source(src: &[u8]) -> Option<turborepo_trace_core::RawRef<'static>> {
    use turborepo_trace_core::{RawRef, RefKind, Span, Specifier};
    const PRAGMA: &[u8] = b"@jsxImportSource";
    let at = memchr::memmem::find(src, PRAGMA)? + PRAGMA.len();
    let rest = src.get(at..)?;
    // The pragma and its value are separated by blanks on one line.
    let skip = rest
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    if skip == 0 {
        return None;
    }
    let value = &rest[skip..];
    let len = value
        .iter()
        .take_while(|b| !b.is_ascii_whitespace() && **b != b'*')
        .count();
    let module = std::str::from_utf8(&value[..len]).ok()?;
    if module.is_empty() {
        return None;
    }
    let start = (at + skip) as u32;
    Some(RawRef::new(
        RefKind::Import,
        Specifier::Literal(format!("{module}/jsx-runtime").into()),
        Span::new(start, start + len as u32),
    ))
}

/// The Js language.
#[derive(Copy, Clone, Debug, Default)]
pub struct JsLanguage;

impl Language for JsLanguage {
    fn id(&self) -> LangId {
        LangId("js")
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"]
    }

    /// Extensionless scripts with a `node`, `bun`, `deno`, `tsx` or `ts-node`
    /// shebang.
    fn sniff(&self, _path: &Path, head: &[u8]) -> bool {
        let Some(rest) = head.strip_prefix(b"#!") else {
            return false;
        };
        let line = &rest[..memchr::memchr(b'\n', rest).unwrap_or(rest.len())];
        [&b"node"[..], b"bun", b"deno", b"tsx", b"ts-node"]
            .iter()
            .any(|w| memchr::memmem::find(line, w).is_some())
    }

    fn extract<'s>(&self, src: &'s [u8], cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        let file_name = cx.path.file_name().and_then(|n| n.to_str());
        // Environment assignments and helpers are first-party facts: installed packages
        // are not scanned for them.
        let first_party = !cx
            .options
            .local(cx.path)
            .components()
            .any(|c| c.as_os_str() == "node_modules");
        let ex = extract::scan_named(
            src,
            Dialect::from_path(cx.path),
            ScanOptions {
                env_facts: first_party,
                ..ScanOptions::default()
            },
            file_name,
        );
        for f in &ex.facts {
            out.fact(f.encode());
        }
        // The file's path names, for reads of other files whose path cannot be
        // evaluated (`turborepo_trace_core::names`), and where its own location
        // and the working directory go (`flow::anchors`).
        let dialect = Dialect::from_path(cx.path);
        let parts = names::scan_parts(src, dialect);
        let anchors = if first_party && anchors_enabled() {
            let templates: Vec<u32> = parts.templates.iter().map(|t| t.at).collect();
            flow::follow_anchors(
                src,
                dialect,
                file_name,
                &parts.uses,
                &ex.site_args,
                &templates,
            )
        } else {
            None
        };
        // What the file does with the mock APIs of the test runners (first-party files
        // only: an installed package does not mock the repository's modules).
        if first_party && mocks::may_mock(src) {
            let m = mocks::scan(src, Dialect::from_path(cx.path));
            if !m.is_empty() {
                out.fact(m.encode());
            }
        }
        // A `@jsxImportSource <module>` pragma makes the transpiler import the JSX
        // factory from `<module>/jsx-runtime`: an import no statement of the
        // file writes.
        if let Some(r) = jsx_import_source(src) {
            out.push(r);
        }
        for r in ex.refs {
            out.push(r);
        }
        for d in ex.diagnostics {
            out.diagnostic(d);
        }
        if let Some(a) = &anchors {
            for e in &a.escapes {
                escape_use(src, e, out);
            }
        }
        let names = parts.finish(anchors.as_ref().map(|a| &a.followed));
        if !names.is_empty() {
            out.fact(names.encode());
        }
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(resolve::JsResolver::new(
            project,
            &resolve::JsResolveOptions::default(),
        ))
    }
}
