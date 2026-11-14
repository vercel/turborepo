//! Fixture snapshot tests for `fixtures/js/basic`.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_core::Tracer;
use turborepo_trace_lang_js::JsLanguage;
use turborepo_trace_testkit::fixture_snapshot;

const BASIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/js/basic"
);

#[test]
#[ignore = "TODO: flesh out fixtures/js/basic, record the snapshot with `INSTA_UPDATE=always cargo \
            test -p turborepo-trace-lang-js --test fixtures -- --include-ignored`, review it, then \
            remove this attribute"]
fn basic_main() {
    fixture_snapshot!(
        "basic_main",
        Tracer::builder().language(JsLanguage),
        BASIC,
        &["main.js"],
    );
}
