#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{git, run_turbo, setup};

#[test]
fn test_query_from_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("query.gql"),
        "query { packages { items { name } } }",
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["query", "query.gql"]);
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let names: Vec<&str> = json["data"]["packages"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"//"));
    assert!(names.contains(&"my-app"));
    assert!(names.contains(&"util"));
    assert!(names.contains(&"another"));
}

#[test]
fn test_query_inline() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["query", "query { version }"]);
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let version = json["data"]["version"].as_str().unwrap();
    assert!(!version.is_empty(), "version should not be empty");
}

fn affected_task_full_names(output: &std::process::Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut names: Vec<String> = json["data"]["affectedTasks"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["fullName"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

// Regression test for https://github.com/vercel/turborepo/issues/12947
//
// A root task declared as `//#task` in the root turbo.json was never included
// in the engine that `turbo query affected` builds, so changing a file the task
// declares as an input reported zero affected tasks. The basic_monorepo fixture
// declares `//#something`, which uses default inputs (every root file), so a
// change to a root file should mark it affected.
#[test]
fn test_query_affected_detects_root_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // bar.txt lives at the repo root and is not a declared globalDependency,
    // so changing it must not mark every task affected. Only the root task
    // (default inputs) and nothing else should be flagged.
    fs::write(tempdir.path().join("bar.txt"), "changed\n").unwrap();
    git(tempdir.path(), &["commit", "-am", "change bar", "--quiet"]);

    let output = run_turbo(
        tempdir.path(),
        &["query", "affected", "--base", "HEAD~1", "--head", "HEAD"],
    );
    let names = affected_task_full_names(&output);
    assert!(
        names.contains(&"//#something".to_string()),
        "root task //#something should be affected by a root file change, got {names:?}"
    );
}

// Companion to test_query_affected_detects_root_task: the `--tasks` filter must
// accept the canonical `//#task` name as well as the bare task name, otherwise
// the documented `turbo query affected --tasks //#task --exit-code` pattern can
// never select a root task.
#[test]
fn test_query_affected_tasks_filter_matches_root_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("bar.txt"), "changed\n").unwrap();
    git(tempdir.path(), &["commit", "-am", "change bar", "--quiet"]);

    // Fully-qualified root task name.
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "affected",
            "--tasks",
            "//#something",
            "--base",
            "HEAD~1",
            "--head",
            "HEAD",
        ],
    );
    assert_eq!(
        affected_task_full_names(&output),
        vec!["//#something".to_string()],
        "--tasks //#something should select the root task"
    );

    // Bare task name resolves to the same root task.
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "affected",
            "--tasks",
            "something",
            "--base",
            "HEAD~1",
            "--head",
            "HEAD",
        ],
    );
    assert_eq!(
        affected_task_full_names(&output),
        vec!["//#something".to_string()],
        "--tasks something (bare name) should select the root task"
    );
}
