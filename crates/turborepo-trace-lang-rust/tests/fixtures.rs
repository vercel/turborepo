//! Fixture snapshot tests for `fixtures/rust`. Traces run for
//! `x86_64-unknown-linux-gnu` so snapshots do not depend on the host.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_core::Tracer;
use turborepo_trace_lang_rust::RustLanguage;
use turborepo_trace_testkit::fixture_snapshot;

const BASIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/rust/basic"
);
const REGISTRY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/rust/registry"
);

fn linux() -> turborepo_trace_core::TracerBuilder {
    Tracer::builder().target("x86_64-unknown-linux-gnu")
}

/// Workspace member binary: module rules (`cli.rs` → `cli/args.rs`, inline
/// modules, `#[path]` files owning their directory), cfg, embeds, the build
/// script and its `rerun-if-changed` inputs, workspace path dependencies and a
/// vendored registry crate with feature-gated modules.
#[test]
fn basic_main() {
    fixture_snapshot!(
        "basic_main",
        linux().language(RustLanguage::new()),
        BASIC,
        &["app/src/main.rs"],
    );
}

/// A library with a `cfg(test)` module using insta.
#[test]
fn basic_util() {
    fixture_snapshot!(
        "basic_util",
        linux().language(RustLanguage::new()),
        BASIC,
        &["util/src/lib.rs"],
    );
}

/// An integration test: its own module tree, the package library, build script
/// and dev-dependencies.
#[test]
fn basic_integration_test() {
    fixture_snapshot!(
        "basic_integration_test",
        linux().language(RustLanguage::new()),
        BASIC,
        &["app/tests/integration.rs"],
    );
}

/// Registry crates below `$CARGO_HOME/registry/src`, a renamed dependency, a
/// git checkout inside a git workspace, and a Windows-only dependency that is
/// skipped for Linux.
#[test]
fn registry_bin() {
    let home = std::path::Path::new(REGISTRY).join("cargo-home");
    fixture_snapshot!(
        "registry_bin",
        linux().language(RustLanguage::with_cargo_home(home)),
        REGISTRY,
        &["src/cli.rs"],
    );
}

// Cross-language parity scenarios (`fixtures/rust/xlang/*`), also run by
// turbotrace's runtime oracle.

fn xlang(cat: &str, entry: &str) {
    let dir = format!(
        "{}/../turborepo-trace-testkit/fixtures/rust/xlang/{cat}",
        env!("CARGO_MANIFEST_DIR")
    );
    let snap = format!("xlang_{cat}");
    fixture_snapshot!(
        snap.as_str(),
        linux().language(RustLanguage::new()),
        dir,
        &[entry],
    );
}

/// `concat!(env!("CARGO_MANIFEST_DIR"), ...)` in a `const` and
/// `Path::new(env!(...)).join(...)` in a `let`, read through a helper: assets
/// (a missing one is reported).
#[test]
fn xlang_assets() {
    xlang("assets", "src/main.rs");
}

/// `format!` with literal and bound arguments, `PathBuf::from(...).join(...)`.
#[test]
fn xlang_evaluator() {
    xlang("evaluator", "src/main.rs");
}

/// `fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/plugins"))`.
#[test]
fn xlang_discovery() {
    xlang("discovery", "src/main.rs");
}

/// `#[cfg]` alternatives of a constant.
#[test]
fn xlang_conditional() {
    xlang("conditional", "src/main.rs");
}

/// `libloading::Library::new` with a relative path and a bare library name.
#[test]
fn xlang_loaders() {
    xlang("loaders", "src/main.rs");
}

/// `cargo:rustc-link-search` / `cargo:rustc-link-lib` in a build script.
#[test]
fn xlang_platform() {
    xlang("platform", "build.rs");
}
