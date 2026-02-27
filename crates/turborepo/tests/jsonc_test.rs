mod common;

use std::{fs, path::Path};

use common::{run_turbo, setup};

fn git_commit(dir: &Path, msg: &str) {
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-am", msg, "--quiet", "--allow-empty"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

fn basic_with_extends_json() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/fixtures/turbo-configs/basic-with-extends.json")
}

// Test 1: Error when both turbo.json and turbo.jsonc exist in root
#[test]
fn test_both_json_and_jsonc_in_root_is_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::copy(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turbo.jsonc"),
    )
    .unwrap();
    git_commit(tempdir.path(), "add turbo.jsonc");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Remove either turbo.json or turbo.jsonc so there is only one"),
        "expected duplicate config error, got: {stderr}"
    );
}

// Test 2: Only turbo.jsonc in root works
#[test]
fn test_turbo_jsonc_only_in_root() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turbo.jsonc"),
    )
    .unwrap();
    git_commit(tempdir.path(), "rename to turbo.jsonc");

    let output = run_turbo(tempdir.path(), &["build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
}

// Test 3: Root turbo.json + package turbo.jsonc
#[test]
fn test_root_json_package_jsonc() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::copy(
        basic_with_extends_json(),
        tempdir.path().join("apps/my-app/turbo.jsonc"),
    )
    .unwrap();
    git_commit(tempdir.path(), "add package turbo.jsonc");

    let output = run_turbo(tempdir.path(), &["build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
}

// Test 4: Root turbo.jsonc + package turbo.json
#[test]
fn test_root_jsonc_package_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turbo.jsonc"),
    )
    .unwrap();
    // The previous test used turbo.jsonc in the package, this uses turbo.json
    fs::copy(
        basic_with_extends_json(),
        tempdir.path().join("apps/my-app/turbo.json"),
    )
    .unwrap();
    git_commit(tempdir.path(), "jsonc root, json package");

    let output = run_turbo(tempdir.path(), &["build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
}

// Test 5: Both turbo.json and turbo.jsonc in a package is error
#[test]
fn test_both_json_and_jsonc_in_package_is_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let app_dir = tempdir.path().join("apps/my-app");
    fs::copy(basic_with_extends_json(), app_dir.join("turbo.json")).unwrap();
    fs::copy(basic_with_extends_json(), app_dir.join("turbo.jsonc")).unwrap();
    git_commit(tempdir.path(), "both in package");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Remove either turbo.json or turbo.jsonc so there is only one"),
        "expected duplicate config error, got: {stderr}"
    );
}
