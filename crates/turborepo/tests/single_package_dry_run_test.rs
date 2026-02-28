mod common;

use std::fs;

use common::{git, run_turbo, setup, turbo_output_filters};

#[test]
fn test_single_package_dry_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("single_package_dry_run", stdout.to_string());
    });
}

#[test]
fn test_single_package_dry_run_pnpm() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "pnpm@8.0.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(
        output.status.success(),
        "dry-run with pnpm should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_single_package_no_config_dry_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    fs::remove_file(tempdir.path().join("turbo.json")).unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "Delete turbo config", "--quiet"],
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("single_package_no_config_dry_run", stdout.to_string());
    });
}

#[test]
fn test_single_package_no_config_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    fs::remove_file(tempdir.path().join("turbo.json")).unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "Delete turbo config", "--quiet"],
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--graph"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph {"));
    assert!(stdout.contains(r#""[root] build" -> "[root] ___ROOT___""#));
}

#[test]
fn test_single_package_no_config_run_bypasses_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    fs::remove_file(tempdir.path().join("turbo.json")).unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "Delete turbo config", "--quiet"],
    );

    let output1 = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(
        stdout1.contains("cache bypass"),
        "expected cache bypass without config, got: {stdout1}"
    );

    let output2 = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("cache bypass"),
        "expected cache bypass on second run too, got: {stdout2}"
    );
}
