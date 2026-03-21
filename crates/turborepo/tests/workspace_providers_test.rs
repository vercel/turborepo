mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, setup};

fn setup_provider_fixture(dir: &Path, fixture: &str) {
    setup::copy_fixture(fixture, dir).unwrap();
    setup::setup_git(dir).unwrap();
    git(dir, &["checkout", "-b", "my-branch"]);
}

fn dry_run_task_command(output: &std::process::Output, task_id: &str) -> String {
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("failed to parse dry run json: {e}"));
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == task_id))
        .and_then(|task| task["command"].as_str())
        .unwrap_or_else(|| panic!("missing task command for {task_id} in {}", json))
        .to_string()
}

#[test]
fn test_cargo_provider_infers_build_command() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_provider_fixture(tempdir.path(), "provider_cargo");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=crate-a", "--dry=json"],
    );
    assert!(
        output.status.success(),
        "cargo dry run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        dry_run_task_command(&output, "crate-a#build"),
        "cargo build"
    );
}

#[test]
fn test_uv_provider_infers_build_command() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_provider_fixture(tempdir.path(), "provider_uv");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=py-app", "--dry=json"],
    );
    assert!(
        output.status.success(),
        "uv dry run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(dry_run_task_command(&output, "py-app#build"), "uv build");
}

#[test]
fn test_mixed_provider_prefers_manifest_specific_commands() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_provider_fixture(tempdir.path(), "provider_mixed");

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(
        output.status.success(),
        "mixed dry run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(dry_run_task_command(&output, "web#build"), "echo web-build");
    assert_eq!(
        dry_run_task_command(&output, "rust-app#build"),
        "cargo build"
    );
    assert_eq!(dry_run_task_command(&output, "py-app#build"), "uv build");
}

#[test]
fn test_cargo_lock_change_marks_all_cargo_packages_affected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_provider_fixture(tempdir.path(), "provider_cargo");

    fs::write(
        tempdir.path().join("Cargo.lock"),
        "changed lockfile for provider affected test\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    assert!(
        output.status.success(),
        "cargo affected query failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"crate-a"), "missing crate-a in {names:?}");
    assert!(names.contains(&"crate-b"), "missing crate-b in {names:?}");
}

#[test]
fn test_uv_lock_change_marks_all_uv_packages_affected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_provider_fixture(tempdir.path(), "provider_uv");

    fs::write(
        tempdir.path().join("uv.lock"),
        "changed lockfile for provider affected test\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedPackages { items { name reason { __typename } } } }",
        ],
    );
    assert!(
        output.status.success(),
        "uv affected query failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = json["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"py-app"), "missing py-app in {names:?}");
    assert!(names.contains(&"py-lib"), "missing py-lib in {names:?}");
}
