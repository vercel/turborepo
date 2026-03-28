mod common;

use common::{run_turbo, setup};

#[test]
fn test_single_package_build_npm() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    // First run: cache miss
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"), "expected cache miss");

    // No .turbo/runs/ directory should exist
    assert!(
        !tempdir.path().join(".turbo/runs").exists(),
        ".turbo/runs/ should not exist"
    );

    // Second run: cache hit
    let output2 = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected full turbo cache hit, got: {stdout2}"
    );
}

#[test]
fn test_single_package_build_yarn() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "yarn@1.22.17", true).unwrap();

    // First run: cache miss
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"), "expected cache miss");

    // Second run: cache hit
    let output2 = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected full turbo cache hit, got: {stdout2}"
    );
}
