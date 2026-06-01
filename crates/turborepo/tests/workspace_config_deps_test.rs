#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, setup};

// Tests that dependsOn (regular + topological) from root config is retained
// when workspace has no turbo.json.

#[test]
fn test_workspace_config_dependency_inheritance_and_overrides() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", false)
        .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task-with-deps",
            "--filter=missing-workspace-config",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Regular dependency
    assert!(
        stdout.contains("missing-workspace-config-underlying-task"),
        "regular dep should run: {stdout}"
    );
    // Topological dependency
    assert!(
        stdout.contains("blank-pkg:missing-workspace-config-underlying-topo-task"),
        "topo dep should run: {stdout}"
    );
    assert!(stdout.contains("3 successful, 3 total"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task-with-deps", "--filter=omit-keys"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("omit-keys:omit-keys-underlying-task"),
        "regular dep should run: {stdout}"
    );
    assert!(
        stdout.contains("blank-pkg:omit-keys-underlying-topo-task"),
        "topo dep should run: {stdout}"
    );
    assert!(stdout.contains("3 successful, 3 total"));

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "override-values-task-with-deps",
            "--filter=override-values",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Only the top-level task should run — no dependencies
    assert!(
        stdout.contains("1 successful, 1 total"),
        "only the task itself should run: {stdout}"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "override-values-task-with-deps",
            "--filter=override-values",
            "--dry=json",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["taskId"]
                .as_str()
                .unwrap()
                .contains("override-values-task-with-deps")
        })
        .unwrap();
    let resolved = &task["resolvedTaskDefinition"];
    assert_eq!(
        resolved["dependsOn"],
        serde_json::json!([]),
        "dependsOn should be overridden to empty"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "override-values-task-with-deps-2",
            "--filter=override-values",
            "--dry=json",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["taskId"]
                .as_str()
                .unwrap()
                .contains("override-values-task-with-deps-2")
        })
        .unwrap();
    let resolved = &task["resolvedTaskDefinition"];
    assert_eq!(
        resolved["dependsOn"],
        serde_json::json!([]),
        "topo-only dependsOn should be overridden to empty"
    );
    let output = run_turbo(
        tempdir.path(),
        &["run", "cross-workspace-task", "--filter=cross-workspace"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("blank-pkg:cross-workspace-underlying-task"),
        "cross-workspace dep should run: {stdout}"
    );
    assert!(stdout.contains("2 successful, 2 total"));

    // Prime cache
    run_turbo(
        tempdir.path(),
        &["run", "cross-workspace-task", "--filter=cross-workspace"],
    );

    // Run with package#task syntax — should hit cache
    let output = run_turbo(
        tempdir.path(),
        &["run", "cross-workspace#cross-workspace-task"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected cache hit with task-id syntax: {stdout}"
    );
}
