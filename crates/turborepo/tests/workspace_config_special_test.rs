#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{run_turbo, setup};

// persistent tests

#[test]
fn test_persistent_and_cache_workspace_config() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", false)
        .unwrap();

    // persistent-task-1 is persistent:true in root, not overridden in workspace
    let output = run_turbo(
        tempdir.path(),
        &["run", "persistent-task-1-parent", "--filter=persistent"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is a persistent task"),
        "expected persistent dependency error: {stderr}"
    );

    // persistent-task-2 is overridden to persistent:false in workspace
    let output = run_turbo(
        tempdir.path(),
        &["run", "persistent-task-2-parent", "--filter=persistent"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));

    // persistent-task-3 is persistent:true in root, workspace defines task but
    // doesn't touch persistent
    let output = run_turbo(
        tempdir.path(),
        &["run", "persistent-task-3-parent", "--filter=persistent"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is a persistent task"),
        "inherited persistent should block parent: {stderr}"
    );

    // persistent-task-4 has no persistent in root, workspace adds persistent:true
    let output = run_turbo(
        tempdir.path(),
        &["run", "persistent-task-4-parent", "--filter=persistent"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is a persistent task"),
        "workspace-added persistent should block parent: {stderr}"
    );

    let output = run_turbo(tempdir.path(), &["run", "cached-task-1", "--filter=cached"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // cache:true override means it should cache (cache miss, not bypass)
    assert!(
        stdout.contains("cache miss"),
        "cache:true override should cache: {stdout}"
    );

    let output = run_turbo(tempdir.path(), &["run", "cached-task-2", "--filter=cached"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache bypass"),
        "cache:false override should bypass: {stdout}"
    );

    let output = run_turbo(tempdir.path(), &["run", "cached-task-3", "--filter=cached"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache bypass"),
        "cache:false in workspace should bypass: {stdout}"
    );
}

// config-change tests

#[test]
fn test_config_change_causes_hash_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", false)
        .unwrap();

    // Get initial hash
    let output1 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "config-change-task",
            "--filter=config-change",
            "--dry=json",
        ],
    );
    let json1: serde_json::Value = serde_json::from_slice(&output1.stdout).unwrap();
    let hash1 = json1["tasks"][0]["hash"].as_str().unwrap().to_string();

    // Same hash on second run
    let output2 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "config-change-task",
            "--filter=config-change",
            "--dry=json",
        ],
    );
    let json2: serde_json::Value = serde_json::from_slice(&output2.stdout).unwrap();
    let hash2 = json2["tasks"][0]["hash"].as_str().unwrap().to_string();
    assert_eq!(hash1, hash2, "hash should be stable");

    // Change workspace turbo.json
    fs::copy(
        tempdir.path().join("apps/config-change/turbo-changed.json"),
        tempdir.path().join("apps/config-change/turbo.json"),
    )
    .unwrap();

    let output3 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "config-change-task",
            "--filter=config-change",
            "--dry=json",
        ],
    );
    let json3: serde_json::Value = serde_json::from_slice(&output3.stdout).unwrap();
    let hash3 = json3["tasks"][0]["hash"].as_str().unwrap().to_string();
    assert_ne!(hash1, hash3, "hash should change when config changes");
}

// task-extends tests

#[test]
fn test_task_extends_inheritance_and_exclusion() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "task_extends", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=task-extends-exclude"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 successful, 1 total"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "test", "--filter=task-extends-exclude"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 successful, 1 total"));

    // lint has extends: false, so it should be excluded
    let output = run_turbo(
        tempdir.path(),
        &["run", "lint", "--filter=task-extends-exclude"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "excluded task should not run: {stdout}"
    );
}

// invalid-config tests

#[test]
fn test_invalid_config_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", false)
        .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=invalid-config"]);
    assert!(!output.status.success());

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("nvalid turbo.json"),
        "expected invalid config error: {combined}"
    );
    assert!(
        combined.contains("No \"extends\" key found"),
        "expected extends key error: {combined}"
    );

    // Even running a valid task in the package should error
    let output = run_turbo(
        tempdir.path(),
        &["run", "valid-task", "--filter=invalid-config"],
    );
    assert!(!output.status.success());

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("No \"extends\" key found"),
        "expected extends error even for valid task: {combined}"
    );

    // Write malformed JSON
    fs::write(
        tempdir.path().join("apps/bad-json/turbo.json"),
        r#"{"tasks": {"trailing-comma": {},}}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "trailing-comma", "--filter=bad-json"],
    );
    assert!(!output.status.success());
}
