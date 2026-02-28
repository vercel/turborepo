mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

fn git(dir: &std::path::Path, args: &[&str]) {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

fn setup_affected(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
}

#[test]
fn test_nothing_affected_on_new_branch() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 no packages"),
        "nothing should be affected: {stdout}"
    );
}

#[test]
fn test_affected_run_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packages in scope: my-app"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_affected_ls_with_file_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 package"));
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_query_file_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-app");
    assert_eq!(items[0]["reason"]["__typename"], "FileChanged");
}

#[test]
fn test_affected_dependency_changed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Add file to util — should affect both util and my-app
    fs::write(tempdir.path().join("packages/util/new.js"), "hello world").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"util"));
    assert!(names.contains(&"my-app"));

    // Check reasons
    let util_item = items.iter().find(|i| i["name"] == "util").unwrap();
    assert_eq!(util_item["reason"]["__typename"], "FileChanged");
    let app_item = items.iter().find(|i| i["name"] == "my-app").unwrap();
    assert_eq!(app_item["reason"]["__typename"], "DependencyChanged");
}

#[test]
fn test_affected_committed_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Add and commit a package.json change
    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
}

#[test]
fn test_affected_scm_base_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    // Override SCM base to HEAD so nothing is affected
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_BASE", "HEAD")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_BASE=HEAD should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_scm_head_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    fs::write(tempdir.path().join("apps/my-app/new.js"), "foo").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    // Override SCM head to main so nothing is affected
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--affected", "--log-order", "grouped"],
        &[("TURBO_SCM_HEAD", "main")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 successful, 0 total"),
        "SCM_HEAD=main should show no changes: {stdout}"
    );
}

#[test]
fn test_affected_merge_base_diverged() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_affected(tempdir.path());

    // Add change on my-branch and commit
    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["description"] = serde_json::Value::String("foo".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);

    // Add a commit to main so merge base diverges
    git(tempdir.path(), &["checkout", "main", "--quiet"]);
    let index_path = tempdir.path().join("packages/util/index.js");
    let mut idx = fs::read_to_string(&index_path).unwrap_or_default();
    idx.push_str("\nfoo");
    fs::write(&index_path, idx).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "add foo", "--quiet"]);
    git(tempdir.path(), &["checkout", "my-branch", "--quiet"]);

    // Only my-app should be affected (between merge-base and my-branch)
    let output = run_turbo(tempdir.path(), &["ls", "--affected"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app"));
    assert!(
        !stdout.contains("util"),
        "util changed on main, not branch: {stdout}"
    );
}
