//! Extraction snapshots for `fixtures/js/extract/*`: every
//! reference with its position, kind, specifier, base (when not the file's
//! directory), flags and uncertainty, plus file diagnostics.
//!
//! Accept changes with `INSTA_UPDATE=always cargo test -p
//! turborepo-trace-lang-js --test extract_snapshots` (or `cargo insta review`).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use turborepo_trace_core::{
    ExtractCx, Language, PathInterner, RefSink, TraceOptions, lexkit::LineIndex,
};
use turborepo_trace_lang_js::JsLanguage;

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/js/extract"
);

/// Extracts a fixture through the `Language` trait (so the dialect comes from
/// the file name) and renders the result.
fn render(name: &str) -> String {
    let path = PathBuf::from(DIR).join(name);
    let src = std::fs::read(&path).unwrap();
    let file = PathInterner::new().intern(&path);
    let options = TraceOptions::default();
    let cx = ExtractCx {
        file,
        path: &path,
        options: &options,
    };
    let mut sink = RefSink::new();
    JsLanguage.extract(&src, &cx, &mut sink);
    let lines = LineIndex::new(&src);
    let snippet = |span: turborepo_trace_core::Span| -> String {
        let s = String::from_utf8_lossy(span.slice(&src).unwrap_or_default()).into_owned();
        let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if one_line.chars().count() > 70 {
            format!("{}…", one_line.chars().take(70).collect::<String>())
        } else {
            one_line
        }
    };
    let mut out = String::new();
    for r in sink.refs() {
        let lc = lines.line_col(r.span.start);
        let flags: Vec<_> = r.flags.names().collect();
        write!(
            out,
            "{}:{} {} {:?}",
            lc.line,
            lc.col,
            r.kind,
            r.spec.to_string()
        )
        .unwrap();
        match &r.base {
            turborepo_trace_core::Base::FileDir => {}
            turborepo_trace_core::Base::Cwd => out.push_str(" @cwd"),
            b => write!(out, " @{b:?}").unwrap(),
        }
        if !flags.is_empty() {
            write!(out, " [{}]", flags.join(", ")).unwrap();
        }
        writeln!(out, "\n    span: {}", snippet(r.span)).unwrap();
        if let Some(u) = &r.uncertainty {
            writeln!(out, "    uncertainty: {} {:?}", u.kind, u.detail).unwrap();
        }
    }
    for d in sink.diagnostics() {
        let lc = lines.line_col(d.span.start);
        writeln!(
            out,
            "diagnostic {}:{} {}: {}",
            lc.line, lc.col, d.kind, d.detail
        )
        .unwrap();
    }
    out
}

macro_rules! fixture {
    ($test:ident, $file:literal) => {
        #[test]
        fn $test() {
            insta::assert_snapshot!(stringify!($test), render($file));
        }
    };
}

fixture!(esm, "esm.mjs");
fixture!(cjs, "cjs.cjs");
fixture!(typescript, "typescript.ts");
fixture!(declarations, "declarations.d.ts");
fixture!(component_tsx, "component.tsx");
fixture!(generics_tsx, "generics.tsx");
fixture!(component_jsx, "component.jsx");
fixture!(jsx_in_js, "jsx_in_js.js");
fixture!(flow, "flow.js");
fixture!(tricky_lexing, "tricky_lexing.js");
fixture!(minified, "minified.js");
fixture!(malformed, "malformed.js");
fixture!(dynamic, "dynamic.js");
fixture!(fs_apis, "fs_apis.mjs");
fixture!(fs_apis_cjs, "fs_apis_cjs.cjs");
fixture!(workers, "workers.mjs");
fixture!(loaders, "loaders.js");
fixture!(ts_type_positions, "ts_type_positions.ts");
fixture!(real_world, "real_world.js");

/// Every fixture has a snapshot test above.
#[test]
fn all_fixtures_covered() {
    let src = include_str!("extract_snapshots.rs");
    for entry in std::fs::read_dir(Path::new(DIR)).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        assert!(
            src.contains(&format!("\"{name}\"")),
            "fixtures/js/extract/{name} has no snapshot test"
        );
    }
}
