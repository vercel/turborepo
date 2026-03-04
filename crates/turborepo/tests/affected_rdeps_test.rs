mod common;

use std::fs;

use common::{git, run_turbo, setup};

#[test]
fn test_affected_includes_reverse_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    git(tempdir.path(), &["checkout", "-b", "my-branch"]);

    let index_path = tempdir.path().join("packages/util/index.js");
    let mut contents = fs::read_to_string(&index_path).unwrap_or_default();
    contents.push_str("\nfoo");
    fs::write(&index_path, contents).unwrap();

    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--affected", "--dry=json"],
    );
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut task_ids: Vec<String> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["command"].as_str().unwrap_or("") != "<NONEXISTENT>")
        .map(|t| t["taskId"].as_str().unwrap().to_string())
        .collect();
    task_ids.sort();

    assert_eq!(task_ids, vec!["my-app#build", "util#build"]);
}
