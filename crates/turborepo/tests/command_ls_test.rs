#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_ls_all_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("3 packages"));
    assert!(stdout.contains("another"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
}

#[test]
fn test_ls_json_preserves_package_paths_and_order() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--output", "json"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["packages"],
        serde_json::json!({
            "count": 3,
            "items": [
                { "name": "another", "path": "packages/another" },
                { "name": "my-app", "path": "apps/my-app" },
                { "name": "util", "path": "packages/util" }
            ]
        })
    );
}

#[test]
fn test_ls_with_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "-F", "my-app..."]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 packages"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
    assert!(!stdout.contains("another"));
}

#[test]
fn test_ls_package_detail() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "my-app"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app depends on: util"));
    assert!(stdout.contains("build: echo building"));
}

#[test]
fn test_ls_package_no_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "another"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("another depends on: <no packages>"));
}

#[test]
fn test_ls_does_not_read_lockfile() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    fs::write(tempdir.path().join("package-lock.json"), "not valid json").unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "my-app"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app depends on: util"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("attempting to parse"));
}

#[test]
fn test_filtered_ls_still_reads_lockfile() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    fs::write(tempdir.path().join("package-lock.json"), "not valid json").unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--filter", "my-app"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("attempting to parse"));
}
