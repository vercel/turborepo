#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{git, run_turbo, setup};

#[test]
fn test_query_single_package_affected_tasks_without_flag() {
    assert_single_package_affected_tasks(false);
}

#[test]
fn test_query_single_package_affected_tasks_with_task_inputs_without_flag() {
    assert_single_package_affected_tasks(true);
}

fn assert_single_package_affected_tasks(affected_using_task_inputs: bool) {
    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path();
    fs::create_dir(dir.join("src")).unwrap();
    fs::write(
        dir.join("package.json"),
        r#"{"name":"single-package","version":"1.0.0","packageManager":"npm@10.5.0","scripts":{"build":"echo build"}}"#,
    )
    .unwrap();
    fs::write(
        dir.join("package-lock.json"),
        r#"{"name":"single-package","version":"1.0.0","lockfileVersion":3,"packages":{"":{"name":"single-package","version":"1.0.0"}}}"#,
    )
    .unwrap();
    let mut config = serde_json::json!({
        "agentGuidance": false,
        "tasks": { "build": { "inputs": ["src/**"] } }
    });
    if affected_using_task_inputs {
        config["futureFlags"] = serde_json::json!({ "affectedUsingTaskInputs": true });
    }
    fs::write(dir.join("turbo.json"), config.to_string()).unwrap();
    fs::write(dir.join("src/index.js"), "console.log(1);\n").unwrap();
    fs::write(dir.join("README.md"), "Initial documentation\n").unwrap();
    git(dir, &["init", "--quiet", "--initial-branch=main"]);
    git(dir, &["config", "user.email", "turbo-test@example.com"]);
    git(dir, &["config", "user.name", "Turborepo Test"]);
    git(dir, &["add", "."]);
    git(
        dir,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "Initial"],
    );

    // Single-package tasks must be discovered even with no changes.
    let output = run_turbo(
        dir,
        &[
            "query",
            r#"{ package(name: "//") { tasks { items { fullName } length } } }"#,
        ],
    );
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = &json["data"]["package"]["tasks"];
    assert_eq!(tasks["length"], 1);
    assert_eq!(tasks["items"][0]["fullName"], "//#build");

    for changed_file in [None, Some("README.md"), Some("src/index.js")] {
        if let Some(file) = changed_file {
            fs::write(dir.join(file), "Changed\n").unwrap();
        }
        let expected_count = usize::from(
            changed_file.is_some()
                && (!affected_using_task_inputs || changed_file == Some("src/index.js")),
        );
        for explicit_flag in [false, true] {
            let mut args = vec!["query", "affected", "--base=HEAD", "--exit-code"];
            if explicit_flag {
                args.push("--single-package");
            }
            let output = run_turbo(dir, &args);
            assert_eq!(
                output.status.code(),
                Some(expected_count as i32),
                "affected query failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let affected = &json["data"]["affectedTasks"];
            assert_eq!(affected["length"], expected_count);
            assert_eq!(affected["items"].as_array().unwrap().len(), expected_count);
            if expected_count == 1 {
                assert_eq!(affected["items"][0]["fullName"], "//#build");
                assert_eq!(
                    affected["items"][0]["reason"]["__typename"],
                    if changed_file == Some("src/index.js") {
                        "TaskFileChanged"
                    } else {
                        "TaskAllChanged"
                    }
                );
            }
        }
    }
}

#[test]
fn test_query_from_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    fs::write(
        tempdir.path().join("query.gql"),
        "query { packages { items { name path } } }",
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

    let paths: Vec<&str> = json["data"]["packages"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["", "packages/another", "apps/my-app", "packages/util"]
    );
}

#[test]
fn test_query_inline() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["query", "query { version }"]);
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let version = json["data"]["version"].as_str().unwrap();
    assert!(!version.is_empty(), "version should not be empty");
}

#[test]
fn test_pure_cargo_relationships_exclude_structural_root() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("cargo_pure_workspace", tempdir.path()).unwrap();

    let query = r#"query {
      app: package(name: "app") {
        path
        directDependencies { length items { name path } }
        allDependencies { length items { name path } }
        directDependents { length items { name path } }
        allDependents { length items { name path } }
      }
      leaf: package(name: "lib-a") {
        path
        directDependencies { length items { name path } }
        allDependencies { length items { name path } }
        directDependents { length items { name path } }
        allDependents { length items { name path } }
      }
    }"#;
    let output = run_turbo(tempdir.path(), &["query", query]);
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["app"]["path"], "crates/app");
    assert_eq!(json["data"]["leaf"]["path"], "crates/lib-a");
    assert_eq!(json["data"]["leaf"]["directDependencies"]["length"], 0);
    assert_eq!(json["data"]["leaf"]["allDependencies"]["length"], 0);

    for package in ["app", "leaf"] {
        for relationship in [
            "directDependencies",
            "allDependencies",
            "directDependents",
            "allDependents",
        ] {
            let relation = &json["data"][package][relationship];
            let items = relation["items"].as_array().unwrap();
            assert_eq!(relation["length"].as_u64().unwrap() as usize, items.len());
            assert!(
                items
                    .iter()
                    .all(|item| { item["name"] != "//" && item["path"].as_str().is_some() })
            );
        }
    }
}
