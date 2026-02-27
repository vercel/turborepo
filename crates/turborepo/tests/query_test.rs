mod common;

use std::fs;

use common::{run_turbo, setup};

// --- variables.t ---

#[test]
fn test_query_with_inline_variables() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    fs::write(tempdir.path().join("vars.json"), r#"{ "name": "my-app" }"#).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            r#"query($name: String) { package(name: $name) { name } }"#,
            "--variables",
            "vars.json",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["data"]["package"]["name"], "my-app");
}

#[test]
fn test_query_with_file_variables() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    fs::write(tempdir.path().join("vars.json"), r#"{ "name": "my-app" }"#).unwrap();
    fs::write(
        tempdir.path().join("query.gql"),
        r#"query($name: String) { package(name: $name) { name } }"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["query", "query.gql", "--variables", "vars.json"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["data"]["package"]["name"], "my-app");
}

#[test]
fn test_query_variables_without_query_is_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    fs::write(tempdir.path().join("vars.json"), r#"{ "name": "my-app" }"#).unwrap();

    let output = run_turbo(tempdir.path(), &["query", "--variables", "vars.json"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("the following required arguments were not provided"));
    assert!(stderr.contains("<QUERY>"));
}

// --- validation.t ---

#[test]
fn test_validation_persistent_deps_query_bypasses_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "persistent_dependencies/10-too-many",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // Build with concurrency=1 fails
    let output = run_turbo(tempdir.path(), &["run", "build", "--concurrency=1"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid task configuration"));

    // Query succeeds despite the invalid concurrency config
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { packages { items { tasks { items { fullName } } } } }",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    insta::assert_json_snapshot!("validation_persistent_deps_query", json["data"]);
}

#[test]
fn test_validation_invalid_dependency_query_bypasses_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "task_dependencies/invalid-dependency",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // Build fails due to invalid dependency
    let output = run_turbo(tempdir.path(), &["run", "build2"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#"Could not find "app-a#custom""#));

    // Query succeeds
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { packages { items { tasks { items { fullName } } } } }",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    insta::assert_json_snapshot!("validation_invalid_dep_query", json["data"]);
}
