//! Fixture snapshot tests for `fixtures/go`. Every trace pins the
//! target (`linux/amd64` unless stated), cgo, the module cache (a fake one
//! inside the fixture) and disables `GOROOT` lookup, so results do not depend
//! on the machine.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_core::{Externals, Tracer, TracerBuilder};
use turborepo_trace_lang_go::{GoLanguage, GoOptions, GoRoot};
use turborepo_trace_testkit::fixture_snapshot;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../turborepo-trace-testkit/fixtures/go"
);

fn dir(name: &str) -> String {
    format!("{FIXTURES}/{name}")
}

fn go(name: &str, opts: GoOptions) -> GoLanguage {
    GoLanguage::new(GoOptions {
        modcache: Some(format!("{}/modcache", dir(name)).into()),
        goroot: GoRoot::None,
        cgo: Some(opts.cgo.unwrap_or(true)),
        workspace: Some(true),
        ..opts
    })
}

fn builder(name: &str, target: &str, opts: GoOptions) -> TracerBuilder {
    Tracer::builder()
        .language(go(name, opts))
        .target(target)
        .threads(2)
}

#[test]
fn basic_main() {
    fixture_snapshot!(
        "basic_main",
        builder("basic", "linux/amd64", GoOptions::default()),
        dir("basic"),
        &["main.go"],
    );
}

#[test]
fn basic_main_darwin_arm64_nocgo() {
    fixture_snapshot!(
        "basic_main_darwin_arm64_nocgo",
        builder(
            "basic",
            "darwin/arm64",
            GoOptions {
                cgo: Some(false),
                ..GoOptions::default()
            }
        ),
        dir("basic"),
        &["main.go"],
    );
}

#[test]
fn basic_all_targets() {
    fixture_snapshot!(
        "basic_all_targets",
        builder(
            "basic",
            "linux/amd64",
            GoOptions {
                tags: vec!["integration".into()],
                ..GoOptions::default()
            }
        )
        .all_targets(true),
        dir("basic"),
        &["internal/util/util.go"],
    );
}

#[test]
fn basic_test_file() {
    fixture_snapshot!(
        "basic_test_file",
        builder("basic", "linux/amd64", GoOptions::default()),
        dir("basic"),
        &["internal/util/export_test.go"],
    );
}

#[test]
fn basic_externals_stop() {
    fixture_snapshot!(
        "basic_externals_stop",
        builder("basic", "linux/amd64", GoOptions::default()).externals(Externals::Stop),
        dir("basic"),
        &["main.go"],
    );
}

#[test]
fn vendored_main() {
    fixture_snapshot!(
        "vendored_main",
        builder("vendored", "linux/amd64", GoOptions::default()),
        dir("vendored"),
        &["main.go"],
    );
}

#[test]
fn workspace_main() {
    fixture_snapshot!(
        "workspace_main",
        builder("work", "linux/amd64", GoOptions::default()),
        dir("work"),
        &["app/main.go"],
    );
}

/// The same trace on 1 and 8 threads is byte-identical.
#[test]
fn deterministic_across_threads() {
    let one = turborepo_trace_testkit::fixture_json(
        builder("basic", "linux/amd64", GoOptions::default()).threads(1),
        dir("basic"),
        &["main.go", "internal/util/export_test.go"],
    );
    let many = turborepo_trace_testkit::fixture_json(
        builder("basic", "linux/amd64", GoOptions::default()).threads(8),
        dir("basic"),
        &["main.go", "internal/util/export_test.go"],
    );
    assert_eq!(one, many);
}

// Cross-language parity scenarios (`fixtures/go/xlang/*`), also run by
// the trace's runtime oracle.

fn xlang(cat: &str) {
    let name = format!("xlang/{cat}");
    let snap = format!("xlang_{cat}");
    fixture_snapshot!(
        snap.as_str(),
        builder(&name, "linux/amd64", GoOptions::default()),
        dir(&name),
        &["main.go"],
    );
}

/// `runtime.Caller(0)` + `filepath.Dir` + `filepath.Join` in locals (assets
/// with an `env_dependent` record).
#[test]
fn xlang_assets() {
    xlang("assets");
}

/// `fmt.Sprintf` with literal arguments, constants and locals.
#[test]
fn xlang_evaluator() {
    xlang("evaluator");
}

/// `plugin.Open`.
#[test]
fn xlang_loaders() {
    xlang("loaders");
}

/// `os.ReadDir` + `filepath.Join(dir, e.Name())`, and an embedded directory.
#[test]
fn xlang_discovery() {
    xlang("discovery");
}

/// Build tags select the configuration file.
#[test]
fn xlang_conditional() {
    xlang("conditional");
}

/// `time.LoadLocation` without `time/tzdata`.
#[test]
fn xlang_special() {
    xlang("special");
}
