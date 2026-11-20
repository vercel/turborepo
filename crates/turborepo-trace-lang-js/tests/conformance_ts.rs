//! The plugin-testkit conformance suite for TypeScript (`.ts`: no JSX, TS-only
//! forms).

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use turborepo_trace_lang_js::JsLanguage;
use turborepo_trace_testkit::{Config, conformance};

const SAMPLES: &[&str] = &[
    "import x from './a'",
    "import type T from './a'",
    "import {type A, b} from './a'",
    "export type {A} from './a'",
    "import fs = require('./a')",
    "import type T = require('./a')",
    "const a = require('./a')!",
    "const b = <any>require('./a')",
    "import('./a')",
    "type T = typeof import('./a')",
];

const CONFIG: Config = Config {
    samples: SAMPLES,
    comments: common::COMMENTS,
    strings: common::STRINGS,
    corpus: common::CORPUS,
    tokens: common::TOKENS,
    ..Config::new("/testkit/sample.ts")
};

conformance!(JsLanguage, CONFIG);
