mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_basic(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();
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
fn test_global_env_var_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    // Prime the cache
    run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
    );

    // Run with SOME_ENV_VAR set - should be a cache miss
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
        &[("SOME_ENV_VAR", "hi")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss with global env var, got: {stdout}"
    );
}

#[test]
fn test_thash_env_var_does_not_affect_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    // Prime the cache
    run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
    );

    // THASH-prefixed vars should not bust the cache
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
        &[("SOMETHING_THASH_YES", "hi")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "THASH env var should not bust cache, got: {stdout}"
    );
}

#[test]
fn test_vercel_analytics_env_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    // Prime the cache
    run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=util", "--output-logs=hash-only"],
        &[("VERCEL_ANALYTICS_ID", "hi")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "VERCEL_ANALYTICS_ID should bust cache, got: {stdout}"
    );
}

#[test]
fn test_thash_env_var_not_in_dry_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_basic(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=util", "--dry=json"],
        &[("SOMETHING_THASH_YES", "hi")],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let global_env = &json["tasks"][0]["environmentVariables"]["global"][0];
    assert!(
        global_env.is_null(),
        "THASH var should not appear in dry-run global env, got: {global_env}"
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
