mod common;

use common::{run_turbo, run_turbo_with_env, setup};

#[test]
fn test_fails_without_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "monorepo_no_turbo_json", "npm@10.5.0", true)
        .unwrap();

    let output = run_turbo(tempdir.path(), &["test"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find turbo.json or turbo.jsonc"),
        "expected missing config error, got: {stderr}"
    );
}

#[test]
fn test_allow_no_turbo_json_flag_runs_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "monorepo_no_turbo_json", "npm@10.5.0", true)
        .unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["test", "--experimental-allow-no-turbo-json"],
        &[("MY_VAR", "foo")],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app:test"), "expected test task to run");
    assert!(
        stdout.contains("cache bypass"),
        "expected cache bypass in no-turbo-json mode"
    );
}

#[test]
fn test_allow_no_turbo_json_caching_disabled() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "monorepo_no_turbo_json", "npm@10.5.0", true)
        .unwrap();

    // First run
    run_turbo_with_env(
        tempdir.path(),
        &["test", "--experimental-allow-no-turbo-json"],
        &[("MY_VAR", "foo")],
    );

    // Second run should still bypass cache
    let output = run_turbo_with_env(
        tempdir.path(),
        &["test", "--experimental-allow-no-turbo-json"],
        &[("MY_VAR", "foo")],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache bypass"),
        "expected cache bypass on second run too, got: {stdout}"
    );
}

#[test]
fn test_allow_no_turbo_json_env_var_discovers_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "monorepo_no_turbo_json", "npm@10.5.0", true)
        .unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["build", "test", "--dry=json"],
        &[("TURBO_ALLOW_NO_TURBO_JSON", "true")],
    );
    assert!(output.status.success());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("expected valid JSON from --dry=json");

    let mut task_ids: Vec<String> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["taskId"].as_str().unwrap().to_string())
        .collect();
    task_ids.sort();

    assert_eq!(task_ids, vec!["my-app#build", "my-app#test", "util#build"],);
}
