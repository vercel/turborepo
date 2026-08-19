#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{run_turbo, setup};

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
