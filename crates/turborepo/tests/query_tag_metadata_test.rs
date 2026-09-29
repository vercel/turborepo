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
fn query_tags_decodes_escaped_package_and_task_labels_once() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "query_tags", "npm@10.5.0", false).unwrap();
    let cases = [
        (r#"quoted\"label"#, r#"quoted"label"#),
        (r"path\\label", r"path\label"),
        (r"line\nlabel", "line\nlabel"),
        (r"caf\u00e9", "café"),
        (r"\uD83D\uDE80", "🚀"),
        // A literal backslash followed by 'n' must not become a newline.
        (r"literal\\n", r"literal\n"),
    ];
    let raw_tags = cases
        .iter()
        .map(|(encoded, _)| format!("\"{encoded}\""))
        .collect::<Vec<_>>()
        .join(",");
    // Preserve the JSON escape spellings to exercise the real config parser.
    std::fs::write(
        tempdir.path().join("packages/app/turbo.json"),
        format!(
            r#"{{"extends":["//"],"tags":[{raw_tags}],"tasks":{{"build":{{"tags":[{raw_tags}]}}}}}}"#
        ),
    )
    .unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{ package(name: "app") { tags tasks { items { name tags package { tags } } } } }"#,
    );
    let decoded_tags = json!(cases.iter().map(|(_, decoded)| decoded).collect::<Vec<_>>());
    assert_eq!(data["package"]["tags"], decoded_tags);
    let tasks = data["package"]["tasks"]["items"].as_array().unwrap();
    assert_eq!(tasks[0]["name"], "build");
    assert_eq!(tasks[0]["tags"], decoded_tags);
    assert_eq!(tasks[1]["tags"], json!(["check"]));
    assert_eq!(tasks[2]["tags"], json!([]));
    for task in tasks {
        assert_eq!(task["package"]["tags"], decoded_tags);
    }
}

#[test]
fn query_tags_composes_task_labels_and_never_exposes_the_extends_marker() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "task_tags_extends", "npm@10.5.0", false)
        .unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{
            package(name: "app") { tasks { items { name tags } } }
            plain: package(name: "plain") { tasks { items { name tags } } }
        }"#,
    );
    let tasks = data["package"]["tasks"]["items"].as_array().unwrap();
    for (name, expected) in [
        (
            "build",
            json!(["root", "repeat", "shared", "repeat", "local", "repeat"]),
        ),
        ("test", json!([])),
        ("untagged", json!(["orphan"])),
        ("fresh", json!(["fresh"])),
        ("markerOnly", json!([])),
    ] {
        let task = tasks.iter().find(|task| task["name"] == name).unwrap();
        assert_eq!(task["tags"], expected, "{task}");
    }
    let plain_tasks = data["plain"]["tasks"]["items"].as_array().unwrap();
    assert_eq!(
        plain_tasks
            .iter()
            .find(|task| task["name"] == "build")
            .unwrap()["tags"],
        json!(["root", "repeat"]),
        "root markers must be stripped without a parent"
    );
    for task in tasks.iter().chain(plain_tasks) {
        assert!(
            !task["tags"]
                .as_array()
                .unwrap()
                .contains(&json!("$TURBO_EXTENDS$"))
        );
    }
}

#[test]
fn query_tags_exposes_package_and_resolved_task_labels_separately() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "query_tags", "npm@10.5.0", false).unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{
            packages { items { name tags tasks { items { name tags package { tags }
                directDependencies { items { fullName tags package { tags } } }
            } } } }
        }"#,
    );
    let packages = data["packages"]["items"].as_array().unwrap();
    let root = packages.iter().find(|pkg| pkg["name"] == "//").unwrap();
    assert_eq!(root["tags"], json!(["root-only"]));
    for (name, package_tags, expected_build, expected_test) in [
        (
            "app",
            json!(["application", "shared"]),
            json!(["deploy", "shared"]),
            json!([]),
        ),
        (
            "lib",
            json!(["library", "shared"]),
            json!(["compile", "compile"]),
            json!(["check"]),
        ),
        (
            "plain",
            json!([]),
            json!(["compile", "compile"]),
            json!(["check"]),
        ),
    ] {
        let package = packages.iter().find(|pkg| pkg["name"] == name).unwrap();
        assert_eq!(package["tags"], package_tags);
        let tasks = package["tasks"]["items"].as_array().unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["build", "test", "untagged"]
        );
        assert_eq!(tasks[0]["tags"], expected_build);
        assert_eq!(tasks[1]["tags"], expected_test);
        assert_eq!(tasks[2]["tags"], json!([]));
        for task in tasks {
            assert_eq!(task["package"]["tags"], package_tags);
        }
        if name == "app" {
            assert_eq!(
                tasks[0]["directDependencies"]["items"],
                json!([{"fullName": "lib#build", "tags": ["compile", "compile"],
                    "package": {"tags": ["library", "shared"]}}])
            );
        }
    }
}
