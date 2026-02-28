mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_affected_includes_reverse_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Create a new branch
    std::process::Command::new("git")
        .args(["checkout", "-b", "my-branch"])
        .current_dir(tempdir.path())
        .output()
        .unwrap();

    // Edit a file in util package
    let index_path = tempdir.path().join("packages/util/index.js");
    let mut contents = fs::read_to_string(&index_path).unwrap_or_default();
    contents.push_str("\nfoo");
    fs::write(&index_path, contents).unwrap();

    // Commit the change
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(tempdir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "add foo", "--quiet"])
        .current_dir(tempdir.path())
        .output()
        .unwrap();

    // --affected should include util AND my-app (which depends on util)
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
