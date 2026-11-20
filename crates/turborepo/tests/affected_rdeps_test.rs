#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{git, run_turbo, run_turbo_with_env, setup};

#[test]
fn test_affected_includes_reverse_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

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

#[test]
fn test_staged_rename_affects_source_destination_and_reverse_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup::copy_fixture("basic_monorepo", root).unwrap();
    let package_path = root.join("packages/another/package.json");
    let mut package: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&package_path).unwrap()).unwrap();
    package["scripts"]["build"] = "echo building".into();
    fs::write(package_path, serde_json::to_string(&package).unwrap()).unwrap();
    fs::write(
        root.join("packages/util/index.js"),
        "module.exports = 42;\n",
    )
    .unwrap();
    setup::setup_git(root).unwrap();
    git(root, &["config", "diff.renames", "true"]);
    git(
        root,
        &["mv", "packages/util/index.js", "packages/another/moved.js"],
    );

    for selector in ["--affected", "--filter=...[main]"] {
        let output = run_turbo_with_env(
            root,
            &["run", "build", selector, "--dry=json"],
            &[("TURBO_SCM_BASE", "main")],
        );
        assert!(
            output.status.success(),
            "{selector}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut task_ids: Vec<_> = json["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["taskId"].as_str().unwrap())
            .collect();
        task_ids.sort();
        assert_eq!(task_ids, ["another#build", "my-app#build", "util#build"]);
    }
}
