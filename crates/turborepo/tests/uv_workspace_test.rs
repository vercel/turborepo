//! End-to-end tests for experimental Python (uv) workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::setup;

fn uv_available() -> bool {
    let available = which::which("uv").is_ok();
    if !available {
        eprintln!("skipping: uv is not on PATH");
    }
    available
}

fn setup_uv_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "uv_monorepo", "npm@10.5.0", false).unwrap();
}

fn setup_uv_pure_workspace(dir: &Path) {
    setup::copy_fixture("uv_pure_workspace", dir).unwrap();
    setup::setup_git(dir).unwrap();
    assert!(
        !dir.join("package.json").exists(),
        "the pure uv fixture must have no package.json"
    );
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

fn dry_run_tasks(dir: &Path, args: &[&str]) -> serde_json::Value {
    let mut full_args = args.to_vec();
    full_args.push("--dry-run=json");
    let output = run_turbo(dir, &full_args);
    assert_command_success(&output, "dry-run");
    serde_json::from_slice(&output.stdout).expect("dry-run emits JSON")
}

fn task_ids(json: &serde_json::Value) -> Vec<String> {
    json["tasks"]
        .as_array()
        .expect("tasks array")
        .iter()
        .map(|task| task["taskId"].as_str().expect("taskId").to_string())
        .collect()
}

fn find_task<'a>(json: &'a serde_json::Value, task_id: &str) -> &'a serde_json::Value {
    json["tasks"]
        .as_array()
        .expect("tasks array")
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| panic!("{task_id} in task graph"))
}

#[test]
fn test_uv_packages_in_task_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let json = dry_run_tasks(tempdir.path(), &["build"]);
    let ids = task_ids(&json);
    assert!(ids.contains(&"js-pkg#build".to_string()), "ids: {ids:?}");
    assert!(ids.contains(&"py-app#build".to_string()), "ids: {ids:?}");
    assert!(ids.contains(&"py-lib#build".to_string()), "ids: {ids:?}");

    let py_app = find_task(&json, "py-app#build");
    let dependencies: Vec<&str> = py_app["dependencies"]
        .as_array()
        .expect("dependencies array")
        .iter()
        .filter_map(|dependency| dependency.as_str())
        .collect();
    assert!(
        dependencies.contains(&"py-lib#build"),
        "py-app#build must depend on py-lib#build: {dependencies:?}"
    );
    assert_eq!(py_app["command"], "uv build --package=py-app");
}

#[test]
fn test_pure_uv_workspace_task_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let json = dry_run_tasks(tempdir.path(), &["build"]);
    let ids = task_ids(&json);
    assert!(ids.contains(&"py-app#build".to_string()), "ids: {ids:?}");
    assert!(ids.contains(&"py-lib#build".to_string()), "ids: {ids:?}");

    let json = dry_run_tasks(tempdir.path(), &["format"]);
    assert_eq!(task_ids(&json), vec!["acme#format".to_string()]);
    assert_eq!(
        find_task(&json, "acme#format")["command"],
        "uv format -- packages/py-app packages/py-lib"
    );

    let json = dry_run_tasks(tempdir.path(), &["check"]);
    assert_eq!(task_ids(&json), vec!["acme#check".to_string()]);
    assert_eq!(
        find_task(&json, "acme#check")["command"],
        "uv check --all-packages"
    );
}

#[test]
fn test_uv_filter_by_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let json = dry_run_tasks(tempdir.path(), &["build", "--filter=py-lib"]);
    assert_eq!(task_ids(&json), vec!["py-lib#build".to_string()]);

    let json = dry_run_tasks(tempdir.path(), &["build", "--filter=py-app"]);
    assert_eq!(task_ids(&json), vec!["py-app#build".to_string()]);

    let json = dry_run_tasks(tempdir.path(), &["format", "--filter=py-app"]);
    assert_eq!(task_ids(&json), vec!["py-app#format".to_string()]);
    assert_eq!(
        find_task(&json, "py-app#format")["command"],
        "uv format -- packages/py-app"
    );

    let json = dry_run_tasks(tempdir.path(), &["check", "--filter=py-app"]);
    assert_eq!(task_ids(&json), vec!["py-app#check".to_string()]);
    assert_eq!(
        find_task(&json, "py-app#check")["command"],
        "uv check --package=py-app"
    );

    let json = dry_run_tasks(tempdir.path(), &["build"]);
    let ids = task_ids(&json);
    assert!(ids.contains(&"py-app#build".to_string()));
    assert!(ids.contains(&"py-lib#build".to_string()));
    assert!(
        !ids.contains(&"acme#build".to_string()),
        "the workspace aggregate does not build: {ids:?}"
    );
}

#[test]
fn test_uv_flag_disabled_hints_at_opt_in() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": { "build": {} }
}"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("package.json"),
        r#"{"name": "root", "packageManager": "npm@10.5.0"}"#,
    )
    .unwrap();
    fs::write(tempdir.path().join("package-lock.json"), "{}").unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--filter=py-app"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("experimentalPythonWorkspaces"),
        "stderr should point at the flag: {stderr}"
    );
}

#[test]
fn test_uv_lock_change_affects_all_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let base = dry_run_tasks(
        tempdir.path(),
        &["build", "--filter=[HEAD]", "--log-order", "grouped"],
    );
    assert_eq!(task_ids(&base), Vec::<String>::new());

    let lock_path = tempdir.path().join("uv.lock");
    let mut contents = fs::read_to_string(&lock_path).unwrap();
    contents.push_str("\n# turbo-test lockfile perturbation\n");
    fs::write(&lock_path, contents).unwrap();

    let json = dry_run_tasks(
        tempdir.path(),
        &["build", "--filter=[HEAD]", "--log-order", "grouped"],
    );
    let ids = task_ids(&json);
    assert!(ids.contains(&"py-app#build".to_string()), "ids: {ids:?}");
    assert!(ids.contains(&"py-lib#build".to_string()), "ids: {ids:?}");
}

#[test]
fn test_uv_build_executes_without_caching() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let dry_run = dry_run_tasks(tempdir.path(), &["build", "--filter=py-app"]);
    assert_eq!(
        find_task(&dry_run, "py-app#build")["resolvedTaskDefinition"]["cache"],
        false
    );

    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=py-app", "--log-order", "grouped"],
    );
    assert_command_success(&output, "first build");
    let wheel_exists = || {
        fs::read_dir(tempdir.path().join("dist"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.file_name().to_string_lossy().ends_with(".whl"))
            })
            .unwrap_or(false)
    };
    assert!(wheel_exists(), "uv build must produce a wheel in dist/");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("FULL TURBO"),
        "build must be uncached: {stdout}"
    );
}

#[test]
fn test_uv_prune() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "py-app"]);
    assert_command_success(&output, "turbo prune");
    let out = tempdir.path().join("out");

    assert!(out.join("packages/py-app/pyproject.toml").exists());
    assert!(out.join("packages/py-lib/pyproject.toml").exists());

    let lock = fs::read_to_string(out.join("uv.lock")).unwrap();
    assert!(lock.contains("name = \"py-app\""));
    assert!(lock.contains("name = \"py-lib\""));

    let manifest = fs::read_to_string(out.join("pyproject.toml")).unwrap();
    assert!(
        manifest.contains("packages/py-app") && manifest.contains("packages/py-lib"),
        "workspace members must be rewritten to the kept set: {manifest}"
    );
    assert!(manifest.contains(r#"name = "acme""#));

    if uv_available() {
        let check = std::process::Command::new("uv")
            .args(["lock", "--check"])
            .current_dir(&out)
            .output()
            .expect("uv runs");
        assert!(
            check.status.success(),
            "pruned workspace must pass `uv lock --check`\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
    }
}

#[test]
fn test_uv_prune_mixed_repo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "py-lib"]);
    assert_command_success(&output, "turbo prune");
    let out = tempdir.path().join("out");

    assert!(out.join("python/py-lib/pyproject.toml").exists());
    assert!(!out.join("python/py-app").exists());
    let lock = fs::read_to_string(out.join("uv.lock")).unwrap();
    assert!(lock.contains("name = \"py-lib\""));
    assert!(!lock.contains("name = \"py-app\""));
    let manifest = fs::read_to_string(out.join("pyproject.toml")).unwrap();
    assert!(manifest.contains("python/py-lib"));
    assert!(!manifest.contains("python/py-app"));
    assert!(out.join("package.json").exists());
}
