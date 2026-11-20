#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, setup};
use serde_json::{Value, json};

#[test]
fn query_tag_filters_match_decoded_package_labels() {
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
    // Only change the temporary fixture; preserve the JSON escape spellings
    // so the real configuration parser and run adapter are exercised.
    std::fs::write(
        tempdir.path().join("packages/app/turbo.json"),
        format!(r#"{{"extends":["//"],"tags":[{raw_tags}]}}"#),
    )
    .unwrap();
    let mut query = String::from("{ ");
    for (index, (encoded, decoded)) in cases.iter().enumerate() {
        let decoded = serde_json::to_string(decoded).unwrap();
        let encoded = serde_json::to_string(encoded).unwrap();
        query.push_str(&format!(
            r#"
            decoded{index}: packages(filter: {{has: {{field: TAG, value: {decoded}}}}}) {{
                length items {{ name tags
                    tasks(filter: {{has: {{field: TAG, value: {decoded}}}}}) {{
                        length items {{ name tags package {{ tags }} }}
                    }}
                }}
            }}
            encoded{index}: packages(filter: {{has: {{field: TAG, value: {encoded}}}}}) {{ length }}
        "#
        ));
    }
    query.push('}');
    let output = run_turbo(tempdir.path(), &["query", &query]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("errors").is_none(), "{result}");
    let data = &result["data"];
    let decoded_tags = json!(cases.iter().map(|(_, decoded)| decoded).collect::<Vec<_>>());
    for (index, _) in cases.iter().enumerate() {
        let matched = &data[format!("decoded{index}")];
        assert_eq!(matched["length"], 1, "{matched}");
        let package = &matched["items"][0];
        assert_eq!(package["name"], "app");
        assert_eq!(package["tags"], decoded_tags);
        assert_eq!(
            package["tasks"]["length"], 3,
            "package labels must match all own tasks"
        );
        let tasks = package["tasks"]["items"].as_array().unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["build", "test", "untagged"]
        );
        assert_eq!(tasks[0]["tags"], json!(["compile", "compile"]));
        assert_eq!(tasks[1]["tags"], json!(["check"]));
        assert_eq!(tasks[2]["tags"], json!([]));
        for task in tasks {
            assert_eq!(task["package"]["tags"], decoded_tags);
        }
        assert_eq!(
            data[format!("encoded{index}")]["length"],
            0,
            "raw JSON escape spellings must not match decoded labels"
        );
    }
}

#[test]
fn query_tag_filters_match_composed_task_labels() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "task_tags_extends", "npm@10.5.0", false)
        .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            r#"{
        package(name: "app") {
            root: tasks(filter: {has: {field: TAG, value: "root"}}) { length items { name tags } }
            shared: tasks(filter: {has: {field: TAG, value: "shared"}}) { length }
            local: tasks(filter: {has: {field: TAG, value: "local"}}) { length }
            cleared: tasks(filter: {has: {field: TAG, value: "check"}}) { length }
            orphan: tasks(filter: {has: {field: TAG, value: "orphan"}}) { length }
            fresh: tasks(filter: {has: {field: TAG, value: "fresh"}}) { length }
            inheritedFresh: tasks(filter: {has: {field: TAG, value: "inherited-fresh"}}) { length }
            marker: tasks(filter: {has: {field: TAG, value: "$TURBO_EXTENDS$"}}) { length }
        }
    }"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("errors").is_none(), "{result}");
    let app = &result["data"]["package"];
    assert_eq!(app["root"]["length"], 1);
    assert_eq!(app["root"]["items"][0]["name"], "build");
    assert_eq!(
        app["root"]["items"][0]["tags"],
        json!(["root", "repeat", "shared", "repeat", "local", "repeat"])
    );
    for alias in ["shared", "local", "orphan", "fresh"] {
        assert_eq!(app[alias]["length"], 1, "{alias}: {result}");
    }
    for alias in ["cleared", "inheritedFresh", "marker"] {
        assert_eq!(app[alias]["length"], 0, "{alias}: {result}");
    }
}

#[test]
fn query_tag_filters_distinguish_package_and_task_labels() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "query_tags", "npm@10.5.0", false).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            r#"{
        libraries: packages(filter: {has: {field: TAG, value: "library"}}) { length items { name } }
        taskOnly: packages(filter: {has: {field: TAG, value: "compile"}}) { length }
        rootOnly: packages(filter: {has: {field: TAG, value: "root-only"}}) { length items { name tags } }
        package(name: "app") {
            own: tasks(filter: {has: {field: TAG, value: "deploy"}}) {
                length items { name tags package { tags }
                    directDependencies(filter: {has: {field: TAG, value: "library"}}) {
                        length items { fullName tags package { tags } }
                    }
                }
            }
            packageTag: tasks(filter: {has: {field: TAG, value: "application"}}) { length }
            overwritten: tasks(filter: {has: {field: TAG, value: "compile"}}) { length }
            cleared: tasks(filter: {has: {field: TAG, value: "check"}}) { length }
            composed: tasks(filter: {
                or: [{has: {field: TAG, value: "deploy"}}, {equal: {field: NAME, value: "test"}}],
                not: {equal: {field: FULL_NAME, value: "app#test"}}
            }) { length items { name } }
            wrongType: tasks(filter: {has: {field: TAG, value: 42}}) { length }
            caseSensitive: tasks(filter: {has: {field: TAG, value: "Deploy"}}) { length }
            unknown: tasks(filter: {has: {field: TAG, value: "missing"}}) { length }
        }
    }"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("errors").is_none(), "{result}");
    let data = &result["data"];
    assert_eq!(
        data["libraries"],
        json!({"length": 1, "items": [{"name": "lib"}]})
    );
    assert_eq!(data["taskOnly"]["length"], 0);
    assert_eq!(
        data["rootOnly"],
        json!({"length": 1, "items": [{"name": "//", "tags": ["root-only"]}]})
    );
    assert_eq!(
        data["package"]["own"],
        json!({
            "length": 1,
            "items": [{"name": "build", "tags": ["deploy", "shared"],
                "package": {"tags": ["application", "shared"]},
                "directDependencies": {"length": 1, "items": [{"fullName": "lib#build",
                    "tags": ["compile", "compile"], "package": {"tags": ["library", "shared"]}}]}}]
        })
    );
    assert_eq!(data["package"]["packageTag"]["length"], 3);
    assert_eq!(
        data["package"]["composed"],
        json!({"length": 1, "items": [{"name": "build"}]})
    );
    for alias in [
        "overwritten",
        "cleared",
        "wrongType",
        "caseSensitive",
        "unknown",
    ] {
        assert_eq!(data["package"][alias]["length"], 0, "{alias}: {result}");
    }
}
