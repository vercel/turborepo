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
fn query_experimental_ci_equality_uses_resolved_bool_object_and_null_metadata() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("query_experimental_ci", tempdir.path()).unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{
        packages { items { name
            yes: tasks(filter: {equal: {field: EXPERIMENTAL_CI, value: true}}) { items { name experimentalCI } }
            no: tasks(filter: {equal: {field: EXPERIMENTAL_CI, value: false}}) { items { name experimentalCI } }
            unset: tasks(filter: {equal: {field: EXPERIMENTAL_CI, value: null}}) { items { name experimentalCI } }
            omitted: tasks(filter: {equal: {field: EXPERIMENTAL_CI}}) { items { name experimentalCI } }
            nameNull: tasks(filter: {equal: {field: NAME, value: null}}) { length }
            fullNameNull: tasks(filter: {equal: {field: FULL_NAME}}) { length }
            nameNotNull: tasks(filter: {notEqual: {field: NAME, value: null}}) { length }
            fullNameNotNull: tasks(filter: {notEqual: {field: FULL_NAME}}) { length }
            tagNull: tasks(filter: {has: {field: TAG, value: null}}) { length }
            object: tasks(filter: {equal: {field: EXPERIMENTAL_CI, value: {nested: {enabled: false}, jobs: ["test"]}}}) { items { name experimentalCI } }
            notNull: tasks(filter: {notEqual: {field: EXPERIMENTAL_CI, value: null}}) { items { name } }
            notFalse: tasks(filter: {notEqual: {field: EXPERIMENTAL_CI, value: false}}) { items { name } }
            partial: tasks(filter: {equal: {field: EXPERIMENTAL_CI, value: {jobs: ["test"]}}}) { length }
        } }
    }"#,
    );
    let packages = data["packages"]["items"].as_array().unwrap();
    for name in ["blocked", "disabled", "opted-in", "plain", "shared"] {
        let package = packages.iter().find(|pkg| pkg["name"] == name).unwrap();
        let (matched_alias, value) = match name {
            "blocked" | "plain" => ("yes", json!(true)),
            "disabled" | "opted-in" => ("no", json!(false)),
            "shared" => (
                "object",
                json!({"jobs":["test"], "nested":{"enabled":false}}),
            ),
            _ => unreachable!(),
        };
        for alias in ["yes", "no", "object"] {
            let expected = if alias == matched_alias {
                json!([{"name":"check", "experimentalCI":value}])
            } else {
                json!([])
            };
            assert_eq!(
                package[alias]["items"], expected,
                "{name}/{alias}: {package}"
            );
        }
        assert_eq!(
            package["unset"]["items"],
            json!([{"name":"build", "experimentalCI":null}])
        );
        assert_eq!(package["omitted"], package["unset"]);
        for alias in ["nameNull", "fullNameNull", "tagNull"] {
            assert_eq!(package[alias]["length"], 0);
        }
        for alias in ["nameNotNull", "fullNameNotNull"] {
            assert_eq!(package[alias]["length"], 2);
        }
        assert_eq!(package["notNull"]["items"], json!([{"name":"check"}]));
        assert_eq!(
            package["notFalse"]["items"],
            if value == false {
                json!([{"name":"build"}])
            } else {
                json!([{"name":"build"}, {"name":"check"}])
            }
        );
        assert_eq!(package["partial"]["length"], 0);
    }
}

#[test]
fn query_resolved_metadata_selection_excludes_false_and_tags_can_reselect_it() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("query_experimental_ci", tempdir.path()).unwrap();
    let data = query_data(
        tempdir.path(),
        r#"{
        packages { items { name
            legacy: tasks(filter: {and: [
                {notEqual: {field: EXPERIMENTAL_CI, value: null}},
                {notEqual: {field: EXPERIMENTAL_CI, value: false}}
            ]}) { items { fullName } }
            selected: tasks(filter: {
                or: [
                    {and: [
                        {notEqual: {field: EXPERIMENTAL_CI, value: null}},
                        {notEqual: {field: EXPERIMENTAL_CI, value: false}}
                    ]},
                    {has: {field: TAG, value: "ci"}}
                ],
                not: {has: {field: TAG, value: "!ci"}}
            }) { items { fullName experimentalCI directDependencies { items { fullName experimentalCI } } } }
        } }
    }"#,
    );
    let packages = data["packages"]["items"].as_array().unwrap();
    for (name, legacy_enabled, selected) in [
        ("blocked", true, false),
        ("disabled", false, false),
        ("opted-in", false, true),
        ("plain", true, true),
        ("shared", true, true),
    ] {
        let package = packages.iter().find(|pkg| pkg["name"] == name).unwrap();
        assert_eq!(
            package["legacy"]["items"],
            if legacy_enabled {
                json!([{"fullName":format!("{name}#check")}])
            } else {
                json!([])
            },
            "{package}"
        );
        let tasks = package["selected"]["items"].as_array().unwrap();
        assert_eq!(tasks.len(), usize::from(selected), "{package}");
        if selected {
            assert_eq!(tasks[0]["fullName"], format!("{name}#check"));
            assert_eq!(
                tasks[0]["directDependencies"]["items"],
                json!([
                    {"fullName":format!("{name}#build"), "experimentalCI":null}
                ]),
                "unmatched prerequisites must remain available"
            );
        }
        if name == "opted-in" {
            assert_eq!(tasks[0]["experimentalCI"], false);
        }
    }
}
