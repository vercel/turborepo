#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, setup, turbo_output_filters};

#[test]
fn test_with_deps_run_and_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    // First run: cache miss for both build and test
    let output1 = run_turbo(tempdir.path(), &["run", "test"]);
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(stdout1.contains("2 successful, 2 total"));
    assert!(stdout1.contains("0 cached, 2 total"));

    // Second run: cache hit for both
    let output2 = run_turbo(tempdir.path(), &["run", "test"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected full turbo, got: {stdout2}"
    );
    assert!(stdout2.contains("2 cached, 2 total"));
}

#[test]
fn test_with_deps_dry_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "test", "--dry"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("single_package_with_deps_dry_run", stdout.to_string());
    });
}
