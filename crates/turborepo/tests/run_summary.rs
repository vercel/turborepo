#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{run_turbo, setup};

fn read_run_summaries(dir: &Path) -> Vec<serde_json::Value> {
    let runs_dir = dir.join(".turbo/runs");
    if !runs_dir.exists() {
        return vec![];
    }
    let mut files: Vec<_> = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
        .iter()
        .map(|p| serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap())
        .collect()
}

fn get_task(summary: &serde_json::Value, task_id: &str) -> serde_json::Value {
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"].as_str() == Some(task_id))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[test]
fn test_run_summary_discovery() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--filter=my-app"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Summary:"));
    assert!(stdout.contains(".turbo"));
}

#[test]
fn test_run_summary_monorepo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    // First run (cache miss)
    run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--", "someargs"],
    );

    // Second run (cache hit)
    run_turbo(
        tempdir.path(),
        &["run", "build", "--summarize", "--", "someargs"],
    );

    let mut summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 2);

    // Sort by cached count so first=miss (0 cached), second=hit (2 cached).
    // This avoids relying on ksuid timestamp ordering which needs a 1s sleep.
    summaries.sort_by_key(|s| s["execution"]["cached"].as_u64().unwrap_or(0));

    let first = &summaries[0];
    let second = &summaries[1];

    // Top-level keys
    let mut keys: Vec<String> = first.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert!(keys.contains(&"execution".to_string()));
    assert!(keys.contains(&"tasks".to_string()));

    assert_eq!(first["scm"]["type"], "git");
    assert_eq!(first["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(first["version"], "1");
    assert_eq!(first["execution"]["exitCode"], 0);
    assert_eq!(first["execution"]["attempted"], 2);
    assert_eq!(first["execution"]["cached"], 0);
    assert_eq!(first["execution"]["success"], 2);

    // Task summaries
    let first_app = get_task(first, "my-app#build");
    let second_app = get_task(second, "my-app#build");

    assert_eq!(first_app["execution"]["exitCode"], 0);
    assert_eq!(first_app["cliArguments"], serde_json::json!(["someargs"]));
    insta::assert_snapshot!(
        "external_deps_hash",
        first_app["hashOfExternalDependencies"].as_str().unwrap()
    );

    // First run: MISS, second run: HIT
    assert_eq!(first_app["cache"]["status"], "MISS");
    assert_eq!(second_app["cache"]["status"], "HIT");
    assert_eq!(second_app["cache"]["local"], true);

    // util#build present
    let first_util = get_task(first, "util#build");
    assert_eq!(first_util["execution"]["exitCode"], 0);

    // another#build not present (no build script)
    let another = get_task(first, "another#build");
    assert!(another.is_null());
}

#[test]
fn test_run_summary_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));

    // Run failing task with summarize
    run_turbo(
        tempdir.path(),
        &["run", "maybefails", "--filter=my-app", "--summarize"],
    );

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 1);

    let summary = &summaries[0];
    assert_eq!(summary["execution"]["failed"], serde_json::json!(1));
    assert!(
        [1, 4].contains(&summary["execution"]["exitCode"].as_i64().unwrap()),
        "exitCode should be 1 or 4"
    );
    assert_eq!(summary["execution"]["attempted"], serde_json::json!(1));

    // Task summary for failed task
    let task = get_task(summary, "my-app#maybefails");
    assert!(!task.is_null());
    assert_eq!(task["hash"], "9f05a7188fdf4e93");
    assert_eq!(task["cache"]["status"], "MISS");
    assert!([1, 4].contains(&task["execution"]["exitCode"].as_i64().unwrap()));
    let error = task["execution"]["error"].as_str().unwrap();
    assert!(
        error.contains("maybefails exited"),
        "expected error message, got: {error}"
    );

    // With --continue --force
    let _ = fs::remove_dir_all(tempdir.path().join(".turbo/runs"));
    run_turbo(
        tempdir.path(),
        &["run", "maybefails", "--summarize", "--force", "--continue"],
    );

    let summaries = read_run_summaries(tempdir.path());
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary["execution"]["success"], serde_json::json!(1));
    assert_eq!(summary["execution"]["failed"], serde_json::json!(1));
    assert_eq!(summary["execution"]["attempted"], serde_json::json!(2));
    assert_eq!(summary["tasks"].as_array().unwrap().len(), 2);

    let failed_task = get_task(summary, "my-app#maybefails");
    assert!([1, 4].contains(&failed_task["execution"]["exitCode"].as_i64().unwrap()));
}
