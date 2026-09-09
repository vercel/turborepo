#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::HashSet, fs, path::Path};

use common::{git, run_turbo, setup};

/// Writes the shared fixture: `basic_monorepo` plus the scripts and task
/// definitions used by these tests.
///
/// `filter_using_tasks` toggles the future flag. `declare_with` adds a
/// `with` sibling to a task that no run in this file requests, which forces
/// the repository-wide (general) filter path without changing any run's
/// task set.
fn setup_fixture(dir: &Path, filter_using_tasks: bool, declare_with: bool) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();

    let mut app: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("apps/my-app/package.json")).unwrap())
            .unwrap();
    app["scripts"]["test"] = "echo testing".into();
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

    let mut turbo_json = serde_json::json!({
        "futureFlags": {
            "filterUsingTasks": filter_using_tasks
        },
        "tasks": {
            "build": { "dependsOn": ["^build"] },
            "test": { "dependsOn": ["build"] },
            "package:types": {},
            "package-checks": { "dependsOn": ["package:types"] }
        }
    });
    if declare_with {
        // The task is never requested and no task depends on it, so the
        // sibling is never scheduled. Its mere presence opts this repository
        // out of the package-scoped filter path.
        turbo_json["tasks"]["zzz-never-requested"] = serde_json::json!({ "with": ["another#dev"] });
    }
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "package scope fixture", "--quiet"]);
}

/// Adds the inert `with` declaration to an existing fixture, switching
/// subsequent runs to the general filter path.
fn force_general_path(dir: &Path) {
    let mut turbo_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("turbo.json")).unwrap()).unwrap();
    turbo_json["tasks"]["zzz-never-requested"] = serde_json::json!({ "with": ["another#dev"] });
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();
}

/// Runs `turbo run <args> --dry=json` and returns `(taskId, hash)` pairs.
fn dry_task_records(dir: &Path, args: &[&str]) -> Vec<(String, String)> {
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
    let mut records: Vec<(String, String)> = summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            (
                task["taskId"].as_str().unwrap().to_string(),
                task["hash"]
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            )
        })
        .collect();
    records.sort();
    records
}

fn dry_task_ids(dir: &Path, args: &[&str]) -> HashSet<String> {
    dry_task_records(dir, args)
        .into_iter()
        .map(|(task_id, _)| task_id)
        .collect()
}

/// Runs the same command on the package-scoped path and on the general path
/// (by adding the inert `with` declaration) and asserts both paths produce
/// the same tasks and hashes.
fn assert_fast_path_matches_general_path(dir: &Path, args: &[&str]) -> Vec<(String, String)> {
    let fast = dry_task_records(dir, args);
    force_general_path(dir);
    let general = dry_task_records(dir, args);
    assert_eq!(
        fast, general,
        "package-scoped and general filter paths disagree for {args:?}"
    );
    fast
}

#[test]
fn only_filter_matches_general_path_for_a_single_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let records = assert_fast_path_matches_general_path(
        tempdir.path(),
        &["build", "--filter=my-app", "--only"],
    );
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    assert_eq!(
        task_ids,
        HashSet::from(["my-app#build".to_string(), "util#build".to_string()])
    );
}

#[test]
fn only_filter_matches_general_path_for_exclude_selectors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let records = assert_fast_path_matches_general_path(
        tempdir.path(),
        &["build", "--filter=my-app", "--filter=!util", "--only"],
    );
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    // Excluding `util` only removes it from the entrypoint set; its `build`
    // task remains reachable as a `^build` dependency of `my-app#build`.
    assert_eq!(
        task_ids,
        HashSet::from(["my-app#build".to_string(), "util#build".to_string()])
    );
}

#[test]
fn only_filter_matches_general_path_for_glob_selectors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let records = assert_fast_path_matches_general_path(
        tempdir.path(),
        &["build", "--filter=my-*", "--only"],
    );
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    assert_eq!(
        task_ids,
        HashSet::from(["my-app#build".to_string(), "util#build".to_string()])
    );
}

#[test]
fn only_filter_matches_general_path_for_a_task_without_a_command() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    // `another` has no `test` script, but `test` is defined in the root
    // turbo.json, so the task exists as an orchestration-only node.
    let records = assert_fast_path_matches_general_path(
        tempdir.path(),
        &["test", "--filter=another", "--only"],
    );
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    assert_eq!(task_ids, HashSet::from(["another#test".to_string()]));
}

#[test]
fn only_filter_matches_general_path_for_qualified_task_arguments() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    let records = assert_fast_path_matches_general_path(
        tempdir.path(),
        &["my-app#test", "--filter=util", "--only"],
    );
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    assert_eq!(task_ids, HashSet::from(["my-app#test".to_string()]));
}

#[test]
fn without_only_both_paths_stay_on_the_general_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    // Without `--only` the run must keep using the task-level filter, and
    // toggling the inert `with` declaration must not change the result.
    let records =
        assert_fast_path_matches_general_path(tempdir.path(), &["build", "--filter=my-app"]);
    let task_ids: HashSet<_> = records.into_iter().map(|(task_id, _)| task_id).collect();
    assert_eq!(
        task_ids,
        HashSet::from(["my-app#build".to_string(), "util#build".to_string()])
    );
}

#[test]
fn only_filter_matches_package_level_default_path() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);
    // `test` has no `^task` dependencies, so the task-level and package-level
    // filter semantics coincide for it.
    let scoped = dry_task_ids(tempdir.path(), &["test", "--filter=my-app", "--only"]);

    // The package-scoped path produces the same tasks as the package-level
    // filter used without the future flag.
    let default_tempdir = tempfile::tempdir().unwrap();
    setup_fixture(default_tempdir.path(), false, false);
    let default = dry_task_ids(
        default_tempdir.path(),
        &["test", "--filter=my-app", "--only"],
    );

    assert_eq!(scoped, default);
}

#[test]
fn only_filter_surfaces_missing_tasks_identically() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), true, false);

    // No package defines `does-not-exist`, so both paths must report the
    // same missing-task error.
    let fast = run_turbo(
        tempdir.path(),
        &[
            "run",
            "does-not-exist",
            "--filter=my-app",
            "--only",
            "--dry=json",
        ],
    );
    force_general_path(tempdir.path());
    let general = run_turbo(
        tempdir.path(),
        &[
            "run",
            "does-not-exist",
            "--filter=my-app",
            "--only",
            "--dry=json",
        ],
    );

    assert_eq!(fast.status.success(), general.status.success());
    assert!(!fast.status.success());
    assert_eq!(
        String::from_utf8_lossy(&fast.stderr),
        String::from_utf8_lossy(&general.stderr)
    );
}
