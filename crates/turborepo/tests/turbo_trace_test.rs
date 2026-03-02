mod common;

use common::{run_turbo, setup};

fn query(dir: &std::path::Path, q: &str) -> serde_json::Value {
    let output = run_turbo(dir, &["query", q]);
    assert!(
        output.status.success(),
        "query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid JSON: {e}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn file_paths(json: &serde_json::Value) -> Vec<String> {
    json["data"]["file"]["dependencies"]["files"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_file_path_query() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "main.ts") { path } }"#,
    );
    assert_eq!(json["data"]["file"]["path"], "main.ts");
}

#[test]
fn test_file_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "main.ts") { path, dependencies { files { items { path } } } } }"#,
    );
    assert_eq!(json["data"]["file"]["path"], "main.ts");
    let deps = file_paths(&json);
    assert_eq!(
        deps,
        vec![
            "bar.js",
            "button.css",
            "button.json",
            "button.tsx",
            "foo.js"
        ]
    );
}

#[test]
fn test_button_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "button.tsx") { path, dependencies { files { items { path } } } } }"#,
    );
    let deps = file_paths(&json);
    assert_eq!(deps, vec!["button.css", "button.json"]);
}

#[test]
fn test_circular_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "circular.ts") { path dependencies { files { items { path } } } } }"#,
    );
    let deps = file_paths(&json);
    assert_eq!(deps, vec!["circular2.ts"]);
}

#[test]
fn test_invalid_import_reports_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "invalid.ts") { path dependencies { files { items { path } } errors { items { message } } } } }"#,
    );
    let deps = file_paths(&json);
    assert_eq!(deps, vec!["button.css", "button.json", "button.tsx"]);

    let errors = json["data"]["file"]["dependencies"]["errors"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(errors.len(), 1);
    let msg = errors[0]["message"].as_str().unwrap();
    assert!(
        msg.contains("failed to resolve import") && msg.contains("non-existent-file.js"),
        "expected non-existent-file error, got: {msg}"
    );
}

#[test]
fn test_ast_query() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "main.ts") { path ast } }"#,
    );
    let ast = &json["data"]["file"]["ast"];
    assert_eq!(ast["type"], "Module");
    assert!(!ast["body"].as_array().unwrap().is_empty());

    // First statement should be an import of Button from ./button.tsx
    let first = &ast["body"][0];
    assert_eq!(first["type"], "ImportDeclaration");
    assert_eq!(first["source"]["value"], "./button.tsx");
}

#[test]
fn test_dependencies_with_depth() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "turbo_trace", "npm@10.5.0", true).unwrap();

    let json = query(
        tempdir.path(),
        r#"query { file(path: "main.ts") { path dependencies(depth: 1) { files { items { path } } } } }"#,
    );
    let deps = file_paths(&json);
    // depth=1 should only return direct imports, not transitive
    assert_eq!(deps, vec!["button.tsx", "foo.js"]);
}
