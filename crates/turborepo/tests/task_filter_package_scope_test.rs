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

fn setup_tag_fixture(dir: &Path, filter_using_tasks: bool) {
    setup_fixture(dir, filter_using_tasks, false);
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("turbo.json")).unwrap()).unwrap();
    json["tasks"]["another#build"] = serde_json::json!({"tags": ["ci"]});
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("apps/my-app/turbo.json"),
        r#"{
        "extends": ["//"], "tags": ["ci"]
    }"#,
    )
    .unwrap();
}

#[test]
fn tag_filter_matches_composed_task_tags_without_matching_the_extends_marker() {
    for future_flag in [false, true] {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path();
        setup::setup_integration_test(dir, "task_tags_extends", "npm@10.5.0", false).unwrap();
        let config_path = dir.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["futureFlags"] = serde_json::json!({"filterUsingTasks": future_flag});
        fs::write(config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        for (task, label, packages) in [
            ("build", "root", vec!["app", "plain", "shared"]),
            ("build", "shared", vec!["app", "shared"]),
            ("build", "local", vec!["app"]),
            ("build", "repeat", vec!["app", "plain", "shared"]),
            ("test", "check", vec!["plain", "shared"]),
            ("untagged", "orphan", vec!["app"]),
            ("fresh", "fresh", vec!["app"]),
            ("fresh", "inherited-fresh", vec!["plain", "shared"]),
        ] {
            let expected = packages
                .into_iter()
                .map(|package| format!("{package}#{task}"))
                .collect();
            assert_eq!(
                dry_task_ids(dir, &[task, &format!("--filter=tag:{label}")]),
                expected,
                "{task}, {label}, flag={future_flag}"
            );
        }
        assert!(
            dry_task_ids(
                dir,
                &[
                    "build",
                    "test",
                    "untagged",
                    "fresh",
                    "markerOnly",
                    "--filter=tag:$TURBO_EXTENDS$"
                ]
            )
            .is_empty()
        );
    }
}

#[test]
fn tag_filter_matches_package_and_individual_task_tags_without_future_flag() {
    for future_flag in [false, true] {
        let tempdir = tempfile::tempdir().unwrap();
        setup_tag_fixture(tempdir.path(), future_flag);
        let ids = dry_task_ids(tempdir.path(), &["build", "test", "--filter=tag:ci"]);
        assert_eq!(
            ids,
            HashSet::from([
                "my-app#build".to_string(),
                "my-app#test".to_string(),
                "another#build".to_string(),
                "util#build".to_string(),
            ])
        );
        assert_eq!(
            dry_task_ids(tempdir.path(), &["build", "test", "--filter=...tag:ci"]),
            HashSet::from([
                "my-app#build".to_string(),
                "my-app#test".to_string(),
                "another#build".to_string(),
                "another#test".to_string(),
                "util#build".to_string()
            ])
        );
        assert_eq!(
            ids,
            dry_task_ids(
                tempdir.path(),
                &["build", "test", "--filter=tag:ci", "--only"]
            )
        );
        assert!(dry_task_ids(tempdir.path(), &["build", "--filter=tag:missing"]).is_empty());
        assert!(
            dry_task_ids(
                tempdir.path(),
                &["build", "--filter=tag:ci", "--filter=!tag:ci"]
            )
            .is_empty()
        );
        assert_eq!(
            dry_task_ids(tempdir.path(), &["build", "--filter=tag:ci^..."]),
            HashSet::from(["util#build".to_string()])
        );
    }
}

#[test]
fn tag_filter_preserves_with_siblings_of_dependencies_in_both_modes() {
    for future_flag in [false, true] {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path();
        setup_fixture(dir, future_flag, false);
        let app_path = dir.join("apps/my-app/package.json");
        let mut app: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&app_path).unwrap()).unwrap();
        for task in ["sidecar", "prepare", "watcher"] {
            app["scripts"][task] = format!("echo {task}").into();
        }
        fs::write(&app_path, serde_json::to_string_pretty(&app).unwrap()).unwrap();
        let config_path = dir.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["tasks"]["my-app#test"] =
            serde_json::json!({"tags": ["ci"], "dependsOn": ["build"]});
        config["tasks"]["my-app#build"] = serde_json::json!({"with": ["sidecar"]});
        config["tasks"]["sidecar"] =
            serde_json::json!({"persistent": true, "cache": false, "dependsOn": ["prepare"]});
        config["tasks"]["prepare"] = serde_json::json!({"with": ["watcher"]});
        config["tasks"]["watcher"] = serde_json::json!({"persistent": true, "cache": false});
        let expected = HashSet::from([
            "my-app#test".to_string(),
            "my-app#build".to_string(),
            "my-app#sidecar".to_string(),
            "my-app#prepare".to_string(),
            "my-app#watcher".to_string(),
        ]);
        for strict in [false, true] {
            config["futureFlags"]["strictTaskEntrypointSelection"] = strict.into();
            fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
            assert_eq!(
                dry_task_ids(dir, &["test", "--filter=tag:ci"]),
                expected,
                "filterUsingTasks={future_flag}, strictTaskEntrypointSelection={strict}"
            );
        }
    }
}

#[test]
fn quoted_tag_filters_match_arbitrary_package_and_task_labels() {
    let labels = [
        "",
        "ci.",
        "ci...",
        "ci[main]{dir}",
        "...^![]{}",
        "quote\"slash\\",
        "line\n\t\0",
        "é🚀",
    ];
    for future_flag in [false, true] {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path();
        setup_fixture(dir, future_flag, false);
        let config_path = dir.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["tasks"]["another#build"] = serde_json::json!({"tags": labels});
        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let app_config = serde_json::json!({"extends": ["//"], "tags": labels});
        fs::write(
            dir.join("apps/my-app/turbo.json"),
            serde_json::to_string_pretty(&app_config).unwrap(),
        )
        .unwrap();
        let expected = HashSet::from([
            "my-app#build".to_string(),
            "util#build".to_string(),
            "another#build".to_string(),
        ]);
        for label in labels {
            let selector = format!("tag:{}", serde_json::to_string(label).unwrap());
            let filter = format!("--filter={selector}");
            assert_eq!(dry_task_ids(dir, &["build", &filter]), expected, "{filter}");
            let exclude = format!("--filter=!{selector}");
            assert!(
                dry_task_ids(dir, &["build", &filter, &exclude]).is_empty(),
                "{exclude}"
            );
            let scoped = format!("--filter={selector}{{packages/another}}");
            assert_eq!(
                dry_task_ids(dir, &["build", &scoped]),
                HashSet::from(["another#build".to_string()])
            );
        }
        assert_eq!(dry_task_ids(dir, &["build", "--filter=tag:ci."]), expected);
        // A decoded control character and the text of its JSON escape are
        // distinct labels; matching raw package strings would conflate them.
        let escaped_text = format!(
            "--filter=tag:{}",
            serde_json::to_string("line\\n\\t\\u0000").unwrap()
        );
        assert!(dry_task_ids(dir, &["build", &escaped_text]).is_empty());
        // Literal ellipses in the label are distinct from graph modifiers.
        assert_eq!(
            dry_task_ids(dir, &["build", r#"--filter=tag:"ci..."^..."#]),
            HashSet::from(["util#build".to_string()])
        );
        assert_eq!(
            dry_task_ids(dir, &["build", "test", r#"--filter=...tag:"ci...""#]),
            HashSet::from([
                "my-app#build".to_string(),
                "my-app#test".to_string(),
                "util#build".to_string(),
                "another#build".to_string(),
                "another#test".to_string()
            ])
        );
    }
}

#[test]
fn root_package_tags_do_not_leak_into_workspace_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_tag_fixture(dir, false);
    let mut package: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    package["scripts"]["lint"] = "echo root lint".into();
    fs::write(
        dir.join("package.json"),
        serde_json::to_string_pretty(&package).unwrap(),
    )
    .unwrap();
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("turbo.json")).unwrap()).unwrap();
    json["tags"] = serde_json::json!(["root-only"]);
    json["tasks"]["//#lint"] = serde_json::json!({});
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap();
    assert_eq!(
        dry_task_ids(dir, &["lint", "--filter=tag:root-only"]),
        HashSet::from(["//#lint".to_string()])
    );
    assert!(dry_task_ids(dir, &["build", "--filter=tag:root-only"]).is_empty());
}

#[test]
fn tag_exclusion_keeps_required_dependencies_and_qualified_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_tag_fixture(tempdir.path(), false);
    assert_eq!(
        dry_task_ids(
            tempdir.path(),
            &["test", "--filter=another", "--filter=!tag:ci"]
        ),
        HashSet::from(["another#test".to_string(), "another#build".to_string()])
    );
    assert_eq!(
        dry_task_ids(tempdir.path(), &["another#build", "--filter=tag:missing"]),
        HashSet::from(["another#build".to_string()])
    );
}

#[test]
fn adding_tag_filter_preserves_ordinary_package_filter_semantics() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_tag_fixture(tempdir.path(), false);
    for filter in [
        "--filter=...util",
        "--filter=util...",
        "--filter=my-*",
        "--filter=!another",
    ] {
        let normal = dry_task_ids(tempdir.path(), &["build", "test", filter]);
        let with_tag = dry_task_ids(
            tempdir.path(),
            &["build", "test", filter, "--filter=!tag:missing"],
        );
        assert_eq!(normal, with_tag, "{filter}");
    }
}

#[test]
fn adding_tag_filter_preserves_package_affected_semantics() {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    setup_tag_fixture(dir, false);
    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("turbo.json")).unwrap()).unwrap();
    // The changed file is outside build inputs: default --affected must still
    // select the package, rather than implicitly enabling task-input matching.
    json["tasks"]["build"]["inputs"] = serde_json::json!(["src/**"]);
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "tag fixture", "--quiet"]);
    fs::write(dir.join("apps/my-app/README.md"), "changed outside inputs").unwrap();
    let run = |filter: &str| {
        let output = common::run_turbo_with_env(
            dir,
            &["run", "build", "--affected", filter, "--dry=json"],
            &[("TURBO_SCM_BASE", "HEAD")],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        json["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["taskId"].as_str().unwrap().to_string())
            .collect::<HashSet<_>>()
    };
    let normal = run("--filter=my-app");
    assert!(!normal.is_empty());
    assert_eq!(normal, run("--filter=tag:ci"));
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
