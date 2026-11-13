#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, setup};

fn setup_workspace(root: &Path, directories: &[&str], explicit_inputs: bool) {
    fs::write(
        root.join("package.json"),
        r#"{"name":"repro","private":true,"packageManager":"pnpm@10.20.0"}"#,
    )
    .unwrap();
    let mut workspace = String::from("packages:\n");
    let mut lockfile = String::from("lockfileVersion: '9.0'\nimporters:\n  .: {}\n");
    for (index, directory) in directories.iter().enumerate() {
        workspace.push_str(&format!("  - {directory}\n"));
        lockfile.push_str(&format!("  {directory}: {{}}\n"));
        let path = root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("package.json"),
            serde_json::json!({
                "name": format!("pkg-{index}"),
                "scripts": { "build": "echo build" }
            })
            .to_string(),
        )
        .unwrap();
        fs::write(path.join("index.js"), "initial\n").unwrap();
    }
    fs::write(root.join("pnpm-workspace.yaml"), workspace).unwrap();
    fs::write(root.join("pnpm-lock.yaml"), lockfile).unwrap();
    let build = if explicit_inputs {
        serde_json::json!({ "inputs": ["**"] })
    } else {
        serde_json::json!({})
    };
    fs::write(
        root.join("turbo.json"),
        serde_json::json!({ "agentGuidance": false, "tasks": { "build": build } }).to_string(),
    )
    .unwrap();
    setup::setup_git(root).unwrap();
}

#[test]
fn test_nested_packages_are_rejected() {
    for explicit_inputs in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        setup_workspace(root, &["apps/a", "apps/a/b"], explicit_inputs);
        git(root, &["checkout", "-b", "change"]);
        fs::write(root.join("apps/a/b/index.js"), "changed\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "Change child package"]);

        for args in [
            vec!["run", "build", "--dry=json"],
            vec!["query", "affected", "--base", "main", "--head", "change"],
        ] {
            let output = run_turbo(root, &args);
            let stderr = String::from_utf8_lossy(&output.stderr).replace('\\', "/");
            assert!(
                !output.status.success(),
                "nested packages must fail for {args:?}, explicit inputs {explicit_inputs}"
            );
            assert!(
                stderr.contains("Nested packages are not supported"),
                "{stderr}"
            );
            assert!(stderr.contains("apps/a/b"), "{stderr}");
            assert!(stderr.contains("\"apps/a\""), "{stderr}");
        }
    }
}

#[test]
fn test_sibling_and_grouped_packages_are_supported() {
    let temp = tempfile::tempdir().unwrap();
    setup_workspace(temp.path(), &["apps/a", "apps/ab", "apps/group/b"], false);
    let output = run_turbo(temp.path(), &["run", "build", "--dry=json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut tasks: Vec<_> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["taskId"].as_str().unwrap())
        .collect();
    tasks.sort();
    assert_eq!(tasks, ["pkg-0#build", "pkg-1#build", "pkg-2#build"]);
}

#[test]
fn test_unnamed_nested_manifests_are_still_ignored() {
    for (unnamed, expected_task) in [("apps/a", "pkg-1#build"), ("apps/a/b", "pkg-0#build")] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        setup_workspace(root, &["apps/a", "apps/a/b"], false);
        fs::write(root.join(unnamed).join("package.json"), "{}").unwrap();
        let output = run_turbo(root, &["run", "build", "--dry=json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let tasks = json["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["taskId"], expected_task);
    }
}
