#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, setup};
use serde_json::{Value, json};

fn query_data(root: &std::path::Path, query: &str) -> Value {
    let output = run_turbo(root, &["query", query]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("errors").is_none(), "{result}");
    result["data"].clone()
}

#[test]
fn query_catalogue_includes_resolved_scriptless_tasks_and_native_scripts() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("query_scriptless_tasks", tempdir.path()).unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{
            packages { items { name tasks { items { name script command tags
                directDependencies { items { fullName } }
            } } } }
        }"#,
    );
    let packages = data["packages"]["items"].as_array().unwrap();
    for (name, expected) in [
        ("//", vec![]),
        (
            "app",
            vec!["build", "check", "local-command", "shared-check"],
        ),
        ("excluded", vec!["build"]),
        ("plain", vec!["build", "check", "native-only"]),
        ("shared", vec!["build", "check", "shared-check"]),
    ] {
        let package = packages.iter().find(|pkg| pkg["name"] == name).unwrap();
        let tasks = package["tasks"]["items"].as_array().unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected,
            "{name}: {tasks:?}"
        );
        for task in tasks {
            match task["name"].as_str().unwrap() {
                "build" => {
                    assert_eq!(task["script"], format!("echo {name}"));
                    assert_eq!(task["command"], format!("echo {name}"));
                    assert_eq!(task["tags"], json!([]));
                }
                "check" => {
                    assert_eq!(task["script"], Value::Null);
                    assert_eq!(task["command"], Value::Null);
                    assert_eq!(task["tags"], json!(["ci"]));
                    assert_eq!(
                        task["directDependencies"]["items"],
                        json!([
                            {"fullName": format!("{name}#build")}
                        ])
                    );
                }
                "local-command" => {
                    assert_eq!(name, "app");
                    assert_eq!(task["script"], Value::Null);
                    assert_eq!(task["command"], "echo local");
                    assert_eq!(task["tags"], json!(["ci", "local"]));
                }
                "shared-check" => {
                    assert_eq!(task["script"], Value::Null);
                    assert_eq!(task["command"], Value::Null);
                    assert_eq!(task["tags"], json!(["ci", "shared"]));
                    assert_eq!(task["directDependencies"]["items"], json!([]));
                }
                "native-only" => {
                    assert_eq!(task["script"], "echo native");
                    assert_eq!(task["command"], "echo native");
                    assert_eq!(task["tags"], json!([]));
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn query_scriptless_ci_entrypoints_match_dry_run_and_task_filters() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("query_scriptless_tasks", tempdir.path()).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "check", "--filter=app", "--dry=json"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dry: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        dry["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["taskId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["app#build", "app#check"]
    );

    let data = query_data(
        tempdir.path(),
        r#"{
            packages { items { name
                tasks(filter: {has: {field: TAG, value: "ci"}}) {
                    items { fullName tags package { tags }
                        directDependencies { items { fullName tags } }
                    }
                }
            } }
            named: packages(filter: {has: {field: TASK_NAME, value: "check"}}) { items { name } }
            app: package(name: "app") {
                named: tasks(filter: {equal: {field: NAME, value: "check"}}) { items { fullName } }
                full: tasks(filter: {equal: {field: FULL_NAME, value: "app#local-command"}}) { items { fullName } }
                unknown: tasks(filter: {equal: {field: NAME, value: "missing"}}) { length }
            }
        }"#,
    );
    assert_eq!(
        data["named"]["items"],
        json!([{"name":"app"}, {"name":"plain"}, {"name":"shared"}])
    );
    assert_eq!(
        data["app"]["named"]["items"],
        json!([{"fullName":"app#check"}])
    );
    assert_eq!(
        data["app"]["full"]["items"],
        json!([{"fullName":"app#local-command"}])
    );
    assert_eq!(data["app"]["unknown"]["length"], 0);
    let packages = data["packages"]["items"].as_array().unwrap();
    let app = packages.iter().find(|pkg| pkg["name"] == "app").unwrap();
    assert_eq!(
        app["tasks"]["items"],
        json!([
            {"fullName":"app#check", "tags":["ci"], "package":{"tags":["application"]},
                "directDependencies":{"items":[{"fullName":"app#build", "tags":[]}]}},
            {"fullName":"app#local-command", "tags":["ci","local"], "package":{"tags":["application"]},
                "directDependencies":{"items":[]}},
            {"fullName":"app#shared-check", "tags":["ci","shared"], "package":{"tags":["application"]},
                "directDependencies":{"items":[]}}
        ])
    );
    for (name, expected) in [
        ("//", vec![]),
        ("excluded", vec![]),
        ("plain", vec!["plain#check"]),
        ("shared", vec!["shared#check", "shared#shared-check"]),
    ] {
        let package = packages.iter().find(|pkg| pkg["name"] == name).unwrap();
        assert_eq!(
            package["tasks"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|task| task["fullName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected,
            "{name}: {package}"
        );
    }
}
