#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_basic(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();
}

#[test]
fn test_baseline_cache_hit() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    // First run: cache miss
    let output1 = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
    );
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(stdout1.contains("cache miss"));

    // Second run: cache hit
    let output2 = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected cache hit on second run, got: {stdout2}"
    );
}

#[test]
fn test_thash_env_var_does_not_break_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=util", "--graph"],
        &[("SOMETHING_THASH_YES", "hi")],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("digraph {"),
        "expected DOT graph output, got: {stdout}"
    );
}
