//! Wrappers, tokens and corpus shared by the conformance suites of every
//! dialect.
#![allow(dead_code)]

use turborepo_trace_testkit::{Escape, Wrapper};

/// One statement per reference form.
pub const SAMPLES: &[&str] = &[
    "import x from './a'",
    "import {a as b, c} from \"./a\"",
    "import * as ns from './a'",
    "import './a'",
    "import data from './a.json' with { type: 'json' }",
    "export * from './a'",
    "export * as ns from './a'",
    "export {a, b as c} from './a'",
    "const a = require('./a')",
    "require.resolve('./a')",
    "module.require('./a')",
    "import('./a').then(m => m)",
    "const b = require(name)",
    "try { require('./a') } catch (e) {}",
];

/// JSX text cannot contain `{`, `}`, `<`, `>` unescaped.
fn jsx_text(s: &str) -> String {
    s.replace('{', "&#123;")
        .replace('}', "&#125;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A JSX attribute string cannot contain its quote (no escapes in JSX strings).
fn jsx_attr(s: &str) -> String {
    s.replace('"', "&quot;")
}

/// Comment forms.
pub const COMMENTS: &[Wrapper] = &[
    Wrapper::new("// ", "\n"),
    Wrapper::new("/* ", " */"),
    Wrapper::new("/**\n * ", "\n */\n"),
    Wrapper::new("x = 1 // ", "\r\ny = 2\n"),
];

/// String, template and regex forms shared by every dialect.
pub const STRINGS: &[Wrapper] = &[
    Wrapper::new("x = \"", "\"\n").escape(Escape::Backslash(b"\"")),
    Wrapper::new("x = '", "'\n").escape(Escape::Backslash(b"'")),
    Wrapper::new("x = `", "`\n").escape(Escape::Backslash(b"`$")),
    Wrapper::new("x = `${y}", "${z}`\n").escape(Escape::Backslash(b"`$")),
    Wrapper::new("x = `${`", "`}`\n").escape(Escape::Backslash(b"`$")),
    Wrapper::new("x = tag`", "`\n").escape(Escape::Backslash(b"`$")),
    Wrapper::new("x = /", "/g\n").escape(Escape::Backslash(b"/")),
    Wrapper::new("x = /[", "]/\n").escape(Escape::Backslash(b"]")),
    Wrapper::new("if (a) /", "/.test(b)\n").escape(Escape::Backslash(b"/")),
    Wrapper::new("function f() { return /", "/ }\n").escape(Escape::Backslash(b"/")),
    Wrapper::new("x = a ? /", "/ : b\n").escape(Escape::Backslash(b"/")),
    Wrapper::new("{}\n/", "/.test(b)\n").escape(Escape::Backslash(b"/")),
];

/// JSX forms (`.js` heuristic, `.jsx`, `.tsx`).
pub const JSX_STRINGS: &[Wrapper] = &[
    Wrapper::new("x = <div>", "</div>\n").escape(Escape::Custom(jsx_text)),
    Wrapper::new("x = <>", "</>\n").escape(Escape::Custom(jsx_text)),
    Wrapper::new("x = <a b=\"", "\" />\n").escape(Escape::Custom(jsx_attr)),
    Wrapper::new("x = <a><b>{y}", "</b></a>\n").escape(Escape::Custom(jsx_text)),
    Wrapper::new("x = <div>{/* ", " */}</div>\n"),
    Wrapper::new("x = <div>{'", "'}</div>\n").escape(Escape::Backslash(b"'")),
];

/// Mutation tokens.
pub const TOKENS: &[&[u8]] = &[
    b"import", b"export", b"require", b"module.", b"from", b"type", b"\"", b"'", b"`", b"${", b"}",
    b"//", b"/*", b"*/", b"/", b"<div>", b"</div>", b"<", b"(", b")", b"\\", b"try {", b"\n",
];

/// Corpus seeds.
pub const CORPUS: &[&[u8]] = &[
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/basic/main.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/basic/lib.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/esm.mjs"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/cjs.cjs"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/typescript.ts"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/component.tsx"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/generics.tsx"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/component.jsx"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/jsx_in_js.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/flow.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/tricky_lexing.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/minified.js"),
    include_bytes!("../../../turborepo-trace-testkit/fixtures/js/extract/malformed.js"),
];

pub const fn strings_with_jsx() -> [Wrapper; STRINGS.len() + JSX_STRINGS.len()] {
    let mut out = [STRINGS[0]; STRINGS.len() + JSX_STRINGS.len()];
    let mut i = 0;
    while i < STRINGS.len() {
        out[i] = STRINGS[i];
        i += 1;
    }
    while i < out.len() {
        out[i] = JSX_STRINGS[i - STRINGS.len()];
        i += 1;
    }
    out
}

/// Strings plus JSX forms.
pub const ALL_STRINGS: [Wrapper; STRINGS.len() + JSX_STRINGS.len()] = strings_with_jsx();
