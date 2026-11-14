//! The plugin-testkit conformance suite for plain JS (`.js`, JSX heuristic on).
//! `conformance_ts.rs` and `conformance_tsx.rs` run it for the other
//! dialects.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use turborepo_trace_lang_js::JsLanguage;
use turborepo_trace_testkit::{Config, conformance};

const CONFIG: Config = Config {
    samples: SAMPLES,
    comments: COMMENTS,
    strings: &ALL_STRINGS,
    corpus: CORPUS,
    tokens: TOKENS,
    ..Config::new("/testkit/sample.js")
};

conformance!(JsLanguage, CONFIG);
