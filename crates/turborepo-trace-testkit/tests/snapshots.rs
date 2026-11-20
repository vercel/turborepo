//! Fixture snapshot tests for `fixtures/stub/basic`.
//!
//! Update snapshots with `INSTA_UPDATE=always cargo test -p
//! turborepo-trace-testkit --test snapshots` (or `cargo insta review`), then
//! review the diff.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_core::Tracer;
use turborepo_trace_testkit::{fixture_snapshot, stub::StubLanguage};

const BASIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/stub/basic"
);

#[test]
fn basic_main() {
    fixture_snapshot!(
        "basic_main",
        Tracer::builder().language(StubLanguage),
        BASIC,
        &["main.stub"],
    );
}

#[test]
fn basic_main_and_second() {
    fixture_snapshot!(
        "basic_main_and_second",
        Tracer::builder().language(StubLanguage).threads(2),
        BASIC,
        &["main.stub", "second.stub"],
    );
}
