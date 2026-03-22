mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_query_from_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("query.gql"),
        "query { packages { items { name } } }",
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
}

#[test]
fn test_query_inline() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["query", "query { version }"]);
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let version = json["data"]["version"].as_str().unwrap();
    assert!(!version.is_empty(), "version should not be empty");
}
