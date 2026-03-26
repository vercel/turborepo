mod common;

use common::{git, run_turbo, run_turbo_with_env, setup};

#[test]
fn test_dry_run_packages_in_scope() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Packages in Scope"));
    assert!(stdout.contains("another"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
}

#[test]
fn test_dry_run_global_hash_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Global Hash Inputs"));
    assert!(stdout.contains("Global Env Vars"));
    assert!(stdout.contains("SOME_ENV_VAR"));
}

#[test]
fn test_dry_run_task_details() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check my-app task details
    assert!(stdout.contains("my-app#build"));
    assert!(stdout.contains("echo building"));

    // Check util task details
    assert!(stdout.contains("util#build"));
    assert!(stdout.contains("Env Vars"));
    assert!(stdout.contains("NODE_ENV"));
}

#[test]
fn test_dry_run_env_var_not_in_output() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Set NODE_ENV and verify it doesn't leak into the output as "Environment
    // Variables"
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry", "--filter=util"],
        &[("NODE_ENV", "banana")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "Environment Variables" header should NOT appear in dry-run output for
    // output
    assert!(
        !stdout.contains("Environment Variables"),
        "should not contain 'Environment Variables' header: {stdout}"
    );
}

#[test]
fn test_dry_run_cache_hit_after_real_run() {
    // Regression test for https://github.com/vercel/turborepo/issues/9044
    // After a real run populates the cache, --dry=json should report HIT.
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // First: real run to populate cache
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success(), "real run should succeed");

    // Second: dry run should see cache hits
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(output.status.success(), "dry run should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    for task in tasks {
        let task_id = task["taskId"].as_str().unwrap_or("<unknown>");
        let command = task["command"].as_str().unwrap_or("");
        // Skip tasks that don't have a real command (transit nodes, etc.)
        if command == "<NONEXISTENT>" {
            continue;
        }
        let status = task["cache"]["status"].as_str().unwrap_or("UNKNOWN");
        assert_eq!(
            status, "HIT",
            "task {task_id} should be a cache HIT in dry run after a real run"
        );
    }
}

#[test]
fn test_dry_run_respects_cache_false() {
    // When a task has cache:false, --dry=json should report MISS,
    // matching what a normal run would do (cache bypass).
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Replace turbo.json with a config that disables caching for build
    std::fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "cache": false
    }
  }
}"#,
    )
    .unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "disable cache", "--quiet"],
    );

    // Real run (cache won't be written since cache:false)
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success());

    // Dry run should also report MISS since caching is disabled
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run JSON: {e}\nstdout: {stdout}"));

    let tasks = json["tasks"].as_array().expect("tasks should be an array");
    for task in tasks {
        let command = task["command"].as_str().unwrap_or("");
        if command == "<NONEXISTENT>" {
            continue;
        }
        let task_id = task["taskId"].as_str().unwrap_or("<unknown>");
        let status = task["cache"]["status"].as_str().unwrap_or("UNKNOWN");
        assert_eq!(
            status, "MISS",
            "task {task_id} with cache:false should be MISS in dry run"
        );
    }
}
