//! The plugin-testkit conformance suite for TSX (JSX on, TS generics vs JSX).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use turborepo_trace_lang_js::JsLanguage;
use turborepo_trace_testkit::{Config, conformance};

const SAMPLES: &[&str] = &[
    "import x from './a'",
    "import type T from './a'",
    "const f = <T,>(x: T) => require('./a')",
    "const g = <T extends object>(x: T) => import('./a')",
    "const el = <div>{require('./a')}</div>",
    "const el = <a b={import('./a')} />",
    // Type arguments on an element, a type parameter with a default, a generic function type.
    "const el = <Sel<Opt, true> v={require('./a')} />",
    "const el = <Fld<() => void>>{import('./a')}</Fld>",
    "const h = <A = any,>(x: A) => require('./a')",
    "const v: <T>(x: Box<T>) => T = require('./a')",
];

const CONFIG: Config = Config {
    samples: SAMPLES,
    comments: common::COMMENTS,
    strings: &common::ALL_STRINGS,
    corpus: common::CORPUS,
    tokens: common::TOKENS,
    ..Config::new("/testkit/sample.tsx")
};

conformance!(JsLanguage, CONFIG);
