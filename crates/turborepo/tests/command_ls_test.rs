#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{git, run_turbo, run_turbo_with_env, setup};

fn setup_task_input_affected_ls(dir: &std::path::Path, enabled: bool) {
    fs::write(
        dir.join("package.json"),
        r#"{"name":"repro","private":true,"packageManager":"npm@10.9.2","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "futureFlags": { "affectedUsingTaskInputs": enabled },
            "tasks": { "build": { "dependsOn": ["^build"], "inputs": ["src/**"] } }
        }))
        .unwrap(),
    )
    .unwrap();
    let mut lock_packages = serde_json::Map::new();
    lock_packages.insert(
        String::new(),
        serde_json::json!({ "name": "repro", "workspaces": ["packages/*"] }),
    );
    for name in ["a", "b", "c"] {
        let package_dir = dir.join("packages").join(name);
        fs::create_dir_all(package_dir.join("src")).unwrap();
        let mut manifest = serde_json::json!({ "name": name, "version": "1.0.0" });
        if name != "c" {
            manifest["scripts"] = serde_json::json!({ "build": "echo building" });
        }
        if name == "b" {
            manifest["dependencies"] = serde_json::json!({ "a": "*" });
        }
        if name == "c" {
            // A missing script can still leave an inherited virtual task.
            // Exclude the inherited build to model a package with no tasks.
            fs::write(
                package_dir.join("turbo.json"),
                r#"{"extends":["//"],"tasks":{"build":{"extends":false}}}"#,
            )
            .unwrap();
        }
        fs::write(
            package_dir.join("package.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            package_dir.join("src/index.js"),
            "export const value = 1;\n",
        )
        .unwrap();
        fs::write(package_dir.join("README.md"), "Original documentation\n").unwrap();
        lock_packages.insert(format!("packages/{name}"), manifest);
        lock_packages.insert(
            format!("node_modules/{name}"),
            serde_json::json!({ "resolved": format!("packages/{name}"), "link": true }),
        );
    }
    fs::write(
        dir.join("package-lock.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "repro", "lockfileVersion": 3, "packages": lock_packages
        }))
        .unwrap(),
    )
    .unwrap();
    setup::setup_git(dir).unwrap();
}

#[test]
fn test_ls_affected_using_task_inputs() {
    for enabled in [false, true] {
        for (changed_file, package_names, task_package_names) in [
            (None, vec![], vec![]),
            (
                Some("packages/a/src/index.js"),
                vec!["a", "b"],
                vec!["a", "b"],
            ),
            (Some("packages/a/README.md"), vec!["a", "b"], vec![]),
            (Some("packages/c/src/index.js"), vec!["c"], vec![]),
        ] {
            let tempdir = tempfile::tempdir().unwrap();
            let dir = tempdir.path();
            setup_task_input_affected_ls(dir, enabled);
            if let Some(file) = changed_file {
                fs::write(dir.join(file), "Changed\n").unwrap();
            }
            git(dir, &["add", "."]);
            git(dir, &["commit", "--allow-empty", "-m", "Change", "--quiet"]);
            let expected = if enabled {
                &task_package_names
            } else {
                &package_names
            };
            for command in [vec!["ls"], vec!["query", "ls"]] {
                for filter in [None, Some("b"), Some("!b")] {
                    let mut args = command.clone();
                    args.extend(["--affected", "--output=json"]);
                    if let Some(filter) = filter {
                        args.extend(["--filter", filter]);
                    }
                    let output = run_turbo_with_env(dir, &args, &[("TURBO_SCM_BASE", "HEAD~1")]);
                    assert!(
                        output.status.success(),
                        "{args:?}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    let names = json["packages"]["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|item| item["name"].as_str().unwrap())
                        .collect::<Vec<_>>();
                    let expected = expected
                        .iter()
                        .copied()
                        .filter(|name| match filter {
                            Some("b") => *name == "b",
                            Some("!b") => *name != "b",
                            _ => true,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        names, expected,
                        "flag={enabled}, change={changed_file:?}, args={args:?}"
                    );
                    assert_eq!(json["packages"]["count"], names.len());
                }
            }
            if changed_file == Some("packages/a/src/index.js") {
                let output = run_turbo_with_env(
                    dir,
                    &["run", "build", "--affected", "--dry=json"],
                    &[("TURBO_SCM_BASE", "HEAD~1")],
                );
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                let mut tasks = json["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|task| task["taskId"].as_str().unwrap())
                    .collect::<Vec<_>>();
                tasks.sort();
                assert_eq!(tasks, ["a#build", "b#build"]);
            }
        }
    }
}

#[test]
fn test_ls_task_level_filters() {
    for filter_using_tasks in [false, true] {
        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path();
        setup_task_input_affected_ls(dir, false);
        let config_path = dir.join("turbo.json");
        let mut config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        config["futureFlags"]["filterUsingTasks"] = filter_using_tasks.into();
        config["tasks"]["b#build"] = serde_json::json!({
            "dependsOn": ["^build"], "inputs": ["src/**"], "tags": ["ci"]
        });
        fs::write(config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        fs::write(
            dir.join("packages/a/turbo.json"),
            r#"{"extends":["//"],"tags":["ci"]}"#,
        )
        .unwrap();

        for command in [vec!["ls"], vec!["query", "ls"]] {
            for (filters, expected) in [
                (vec!["tag:ci"], vec!["a", "b"]),
                (vec!["tag:missing"], vec![]),
                (vec!["!tag:missing"], vec!["a", "b"]),
                (vec!["tag:ci", "!b"], vec!["a"]),
                (vec!["b"], vec!["b"]),
                (vec!["b..."], vec!["a", "b"]),
                (vec!["...a"], vec!["a", "b"]),
                (vec!["b^..."], vec!["a"]),
                (vec!["...^a"], vec!["b"]),
                (vec!["!b"], vec!["a", "c"]),
            ] {
                let mut args = command.clone();
                args.push("--output=json");
                for filter in filters {
                    args.extend(["--filter", filter]);
                }
                let output = run_turbo(dir, &args);
                assert!(
                    output.status.success(),
                    "{args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                let names = json["packages"]["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["name"].as_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    names, expected,
                    "filterUsingTasks={filter_using_tasks}, {args:?}"
                );
                assert_eq!(json["packages"]["count"], names.len());
            }
        }

        git(dir, &["add", "."]);
        git(dir, &["commit", "-m", "Configure filters", "--quiet"]);
        fs::write(dir.join("packages/a/src/index.js"), "Changed\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-m", "Change source", "--quiet"]);
        for command in [vec!["ls"], vec!["query", "ls"]] {
            for (selection, expected) in [
                (vec!["--filter=...[HEAD~1]"], vec!["a", "b"]),
                (vec!["--affected"], vec!["a", "b"]),
                (vec!["--affected", "--filter=b"], vec!["b"]),
                (vec!["--affected", "--filter=tag:ci"], vec!["a", "b"]),
                (vec!["--affected", "--filter=tag:missing"], vec![]),
            ] {
                let mut args = command.clone();
                args.extend(selection);
                args.push("--output=json");
                let output = run_turbo_with_env(dir, &args, &[("TURBO_SCM_BASE", "HEAD~1")]);
                assert!(
                    output.status.success(),
                    "{args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                let names = json["packages"]["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["name"].as_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    names, expected,
                    "filterUsingTasks={filter_using_tasks}, {args:?}"
                );
                assert_eq!(json["packages"]["count"], names.len());
            }
        }
    }
}

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

#[test]
fn test_ls_multiple_package_details_json_preserves_order_and_duplicates() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["ls", "util", "my-app", "util", "--output", "json"],
    );
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let names = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| package["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["util", "my-app", "util"]);
    assert_eq!(
        json["packages"][1]["dependencies"],
        serde_json::json!(["util"])
    );
}

#[test]
fn test_ls_multiple_package_details_json_reports_first_missing_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["ls", "util", "missing", "also-missing", "--output", "json"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Package `missing` not found"));
}

#[test]
fn test_ls_multiple_package_details_pretty_preserves_order_and_duplicates() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "util", "my-app", "util"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_util = stdout.find("util depends on:").unwrap();
    let my_app = stdout.find("my-app depends on: util").unwrap();
    let second_util = stdout.rfind("util depends on:").unwrap();
    assert!(first_util < my_app && my_app < second_util);
}

#[test]
fn test_ls_multiple_package_details_pretty_prints_valid_prefix_before_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "util", "missing", "my-app"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("util depends on:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Package `missing` not found"));
}

#[test]
fn test_ls_pretty_reports_missing_first_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "missing", "util"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Package `missing` not found"));
}
