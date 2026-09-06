//! End-to-end tests for experimental Go workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::setup;

fn go_available() -> bool {
    let available = which::which("go").is_ok();
    if !available {
        eprintln!("skipping: go is not on PATH");
    }
    available
}

fn setup_go_pure_workspace(dir: &Path) {
    setup::copy_fixture("go_pure_workspace", dir).unwrap();
    setup::setup_git(dir).unwrap();
    assert!(
        !dir.join("package.json").exists(),
        "the pure Go fixture must have no package.json"
    );
}

fn setup_go_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "go_monorepo", "npm@10.5.0", false).unwrap();
}

fn run_turbo(dir: &Path, args: &[&str]) -> std::process::Output {
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(dir);
    command.env("TURBO_CONFIG_DIR_PATH", config_dir.path());
    command
        .args(args)
        .output()
        .expect("failed to execute turbo")
}

fn assert_command_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn package_names(dir: &Path) -> Vec<String> {
    let output = run_turbo(dir, &["ls", "--output=json"]);
    assert_command_success(&output, "turbo ls");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("ls emits JSON");
    json["packages"]["items"]
        .as_array()
        .expect("packages items")
        .iter()
        .map(|package| package["name"].as_str().expect("name").to_string())
        .collect()
}

fn query_packages(dir: &Path) -> serde_json::Value {
    let output = run_turbo(
        dir,
        &[
            "query",
            "query { packages { items { name directDependencies { items { name } } } } }",
        ],
    );
    assert_command_success(&output, "turbo query");
    serde_json::from_slice(&output.stdout).expect("query emits JSON")
}

#[test]
fn test_pure_go_workspace_lists_modules() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let names = package_names(tempdir.path());
    assert!(
        names.contains(&"example.com/api".to_string()),
        "names: {names:?}"
    );
    assert!(
        names.contains(&"example.com/lib".to_string()),
        "names: {names:?}"
    );
}

#[test]
fn test_mixed_go_workspace_lists_js_and_go_packages() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());

    let names = package_names(tempdir.path());
    assert!(names.contains(&"js-pkg".to_string()), "names: {names:?}");
    assert!(
        names.contains(&"example.com/api".to_string()),
        "names: {names:?}"
    );
    assert!(
        names.contains(&"example.com/lib".to_string()),
        "names: {names:?}"
    );
}

#[test]
fn test_go_workspace_query_reports_internal_dependencies() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let query = query_packages(tempdir.path());
    let packages = query["data"]["packages"]["items"]
        .as_array()
        .expect("packages array");
    let api = packages
        .iter()
        .find(|package| package["name"] == "example.com/api")
        .expect("api package");
    let dependencies: Vec<&str> = api["directDependencies"]["items"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str())
        .collect();
    assert_eq!(dependencies, ["example.com/lib"]);
}

#[test]
fn test_go_filter_by_module_path() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["ls", "--output=json", "--filter=example.com/lib"],
    );
    assert_command_success(&output, "filtered ls");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("ls emits JSON");
    let names = json["packages"]["items"]
        .as_array()
        .expect("packages items")
        .iter()
        .map(|package| package["name"].as_str().expect("name").to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["example.com/lib".to_string()]);
}

#[test]
fn test_disabled_go_workspace_points_to_feature_flag() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_monorepo(tempdir.path());
    let turbo_json = tempdir.path().join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    let updated = contents.replace(
        "\"experimentalGoWorkspaces\": true",
        "\"experimentalGoWorkspaces\": false",
    );
    fs::write(&turbo_json, updated).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "--filter=example.com/api"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("experimentalGoWorkspaces"),
        "expected disabled-flag guidance, stderr:\n{stderr}"
    );
}

#[test]
fn test_pure_go_workspace_has_no_package_json() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert_command_success(&output, "turbo ls");
    assert!(
        !tempdir.path().join("package.json").exists(),
        "turbo must not create a package.json for a pure Go workspace"
    );
}
