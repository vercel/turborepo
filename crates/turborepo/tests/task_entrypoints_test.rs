#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::HashSet, fs, path::Path};

use common::{git, run_turbo, setup};

fn setup_fixture(dir: &Path, strict_entrypoints: bool, filter_using_tasks: bool) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();

    let mut app: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("apps/my-app/package.json")).unwrap())
            .unwrap();
    app["scripts"]["test"] = "echo testing".into();
    app["scripts"]["test-transit"] = "echo testing transit".into();
    fs::write(
        dir.join("apps/my-app/package.json"),
        serde_json::to_string_pretty(&app).unwrap(),
    )
    .unwrap();

    let mut another: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join("packages/another/package.json")).unwrap(),
    )
    .unwrap();
    another["scripts"]["build"] = "echo building".into();
    another["scripts"]["package:types"] = "echo checking package".into();
    fs::write(
        dir.join("packages/another/package.json"),
        serde_json::to_string_pretty(&another).unwrap(),
    )
    .unwrap();

    let future_flags = serde_json::json!({
        "strictTaskEntrypointSelection": strict_entrypoints,
        "filterUsingTasks": filter_using_tasks
    });
    let turbo_json = serde_json::json!({
        "futureFlags": future_flags,
        "tasks": {
            "build": { "dependsOn": ["^build"] },
            "test": { "dependsOn": ["build"] },
            "checks": { "dependsOn": ["build"] },
            "package-checks": { "dependsOn": ["package:types"] },
            "package:types": {},
            "transit": { "dependsOn": ["^transit"] },
            "topo": { "dependsOn": ["^topo"] },
            "test-transit": { "dependsOn": ["transit"] }
        }
    });
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "entrypoint fixture", "--quiet"]);
}

fn dry_tasks(dir: &Path, args: &[&str]) -> HashSet<String> {
    let mut command = vec!["run"];
    command.extend_from_slice(args);
    command.push("--dry=json");
    let output = run_turbo(dir, &command);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    let summary: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn requested_task_starts_only_where_it_has_a_command() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["test"]);

    assert_eq!(
        tasks,
        HashSet::from([
            "my-app#test".to_string(),
            "my-app#build".to_string(),
            "util#build".to_string(),
        ])
    );
}

#[test]
fn task_without_commands_remains_an_orchestration_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["checks"]);

    assert!(tasks.contains("my-app#checks"));
    assert!(tasks.contains("util#checks"));
    assert!(tasks.contains("another#checks"));
    assert!(tasks.contains("my-app#build"));
    assert!(tasks.contains("util#build"));
    assert!(tasks.contains("another#build"));
}

#[test]
fn orchestration_task_prunes_branches_without_runnable_work() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["package-checks"]);

    assert_eq!(
        tasks,
        HashSet::from([
            "another#package-checks".to_string(),
            "another#package:types".to_string(),
        ])
    );
}

#[test]
fn task_filter_selects_only_orchestration_branches_with_runnable_work() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, true);

    let tasks = dry_tasks(tempdir.path(), &["package-checks", "--filter=another"]);
    assert_eq!(
        tasks,
        HashSet::from([
            "another#package-checks".to_string(),
            "another#package:types".to_string(),
        ])
    );

    let tasks = dry_tasks(tempdir.path(), &["package-checks", "--filter=my-app"]);
    assert!(tasks.is_empty());
}

#[test]
fn fully_scriptless_orchestration_graph_is_preserved() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["topo"]);

    assert!(tasks.contains("my-app#topo"));
    assert!(tasks.contains("util#topo"));
    assert!(tasks.contains("another#topo"));
}

#[test]
fn explicitly_requested_dependency_task_still_runs_everywhere() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["build", "test"]);

    assert!(tasks.contains("another#build"));
    assert!(!tasks.contains("another#test"));
    assert!(!tasks.contains("util#test"));
}

#[test]
fn missing_transit_tasks_remain_inside_a_runnable_task_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["test-transit"]);

    assert!(tasks.contains("my-app#transit"));
    assert!(tasks.contains("util#transit"));
}

#[test]
fn task_filter_skips_a_missing_command_without_dependency_expansion() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, true);

    let tasks = dry_tasks(tempdir.path(), &["test", "--filter=another"]);

    assert!(tasks.is_empty());
}

#[test]
fn package_filter_skips_a_missing_command_and_its_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let tasks = dry_tasks(tempdir.path(), &["test", "--filter=another"]);

    assert!(tasks.is_empty());
}

#[test]
fn task_filter_explicitly_selects_dependencies_through_a_missing_command() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, true);

    let tasks = dry_tasks(tempdir.path(), &["test", "--filter=another..."]);

    assert_eq!(tasks, HashSet::from(["another#build".to_string()]));
}

#[test]
fn missing_task_branches_remain_without_the_future_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), false, false);

    let tasks = dry_tasks(tempdir.path(), &["test"]);

    assert!(tasks.contains("another#test"));
    assert!(tasks.contains("another#build"));
    assert!(tasks.contains("util#test"));
}

#[test]
fn filter_using_tasks_does_not_enable_strict_entrypoint_selection() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), false, true);

    let tasks = dry_tasks(tempdir.path(), &["test", "--filter=another"]);

    assert!(tasks.contains("another#test"));
    assert!(tasks.contains("another#build"));
}
