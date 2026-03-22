mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

#[test]
fn test_fails_without_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Rename turbo.json so it can't be found
    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turborepo.json"),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find turbo.json or turbo.jsonc"),
        "expected missing config error, got: {stderr}"
    );
}

#[test]
fn test_root_turbo_json_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turborepo.json"),
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "build",
            "--filter=my-app",
            "--root-turbo-json=turborepo.json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss on first run, got: {stdout}"
    );
}

#[test]
fn test_root_turbo_json_env_var_with_cache_hit() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turborepo.json"),
    )
    .unwrap();

    // First run via flag to prime the cache
    run_turbo(
        tempdir.path(),
        &[
            "build",
            "--filter=my-app",
            "--root-turbo-json=turborepo.json",
        ],
    );

    // Second run via env var should be a cache hit
    let output = run_turbo_with_env(
        tempdir.path(),
        &["build", "--filter=my-app"],
        &[("TURBO_ROOT_TURBO_JSON", "turborepo.json")],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected FULL TURBO cache hit, got: {stdout}"
    );
}
