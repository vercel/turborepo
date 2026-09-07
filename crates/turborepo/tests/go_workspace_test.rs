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

fn dry_run_task(output: &std::process::Output, task_id: &str) -> serde_json::Value {
    assert_command_success(output, "Go task dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == task_id))
        .cloned()
        .unwrap_or_else(|| panic!("{task_id} in task graph"))
}

fn task_hash(dir: &Path, package: &str, task: &str) -> String {
    let output = run_turbo(
        dir,
        &[
            "run",
            task,
            &format!("--filter={package}"),
            "--dry-run=json",
        ],
    );
    dry_run_task(&output, &format!("{package}#{task}"))["hash"]
        .as_str()
        .expect("task has a hash")
        .to_string()
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

fn package_task_names(dir: &Path, package: &str) -> Vec<String> {
    let query =
        format!("query {{ package(name: \"{package}\") {{ tasks {{ items {{ name }} }} }} }}");
    let output = run_turbo(dir, &["query", &query]);
    assert_command_success(&output, "Go task catalog query");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("query emits JSON");
    json["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items")
        .iter()
        .map(|task| task["name"].as_str().expect("task name").to_string())
        .collect()
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

#[test]
fn test_go_native_tasks_and_workspace_aggregate() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalGoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "test", "--dry-run=json"]);
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    assert_eq!(json["tasks"].as_array().map(Vec::len), Some(1));
    let task = dry_run_task(&output, "go-workspace#test");
    assert_eq!(task["command"], "go test ./apps/api/... ./packages/lib/...");
    assert_eq!(task["directory"], "");

    let output = run_turbo(tempdir.path(), &["run", "lint", "--dry-run=json"]);
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    assert_eq!(json["tasks"].as_array().map(Vec::len), Some(1));
    let lint = dry_run_task(&output, "go-workspace#lint");
    assert_eq!(lint["command"], "go vet ./apps/api/... ./packages/lib/...");
    assert_eq!(lint["directory"], "");

    let output = run_turbo(
        tempdir.path(),
        &["run", "lint", "--filter=example.com/lib", "--dry-run=json"],
    );
    let lint = dry_run_task(&output, "example.com/lib#lint");
    assert_eq!(lint["command"], "go vet ./...");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/api", "--dry-run=json"],
    );
    let build = dry_run_task(&output, "example.com/api#build");
    assert_eq!(build["command"], "go build -o dist/api .");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], true);
    assert!(
        build["resolvedTaskDefinition"]["outputs"]
            .as_array()
            .is_some_and(|outputs| outputs.iter().any(|output| output == "dist/api"))
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=example.com/lib", "--dry-run=json"],
    );
    let build = dry_run_task(&output, "example.com/lib#build");
    assert_eq!(build["command"], "go build ./...");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], false);

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "dev",
            "--filter=example.com/api",
            "--dry-run=json",
            "--",
            "--port",
            "3000",
        ],
    );
    let dev = dry_run_task(&output, "example.com/api#dev");
    assert_eq!(dev["command"], "go run .");

    let tasks = package_task_names(tempdir.path(), "example.com/api");
    assert!(tasks.iter().any(|task| task == "dev"), "tasks: {tasks:?}");
    assert!(tasks.iter().any(|task| task == "lint"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "run"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "vet"), "tasks: {tasks:?}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=example.com/api", "--dry-run=json"],
    );
    assert!(!output.status.success(), "removed run task must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `run` in project"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn test_go_task_hash_tracks_source_but_not_unrelated_siblings() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let original = task_hash(tempdir.path(), "example.com/api", "build");

    fs::write(tempdir.path().join("unrelated.txt"), "unrelated\n").unwrap();
    assert_eq!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "repository-root siblings outside the module must not affect its hash"
    );

    fs::write(
        tempdir.path().join("apps/api/main.go"),
        "package main\n\nimport \"example.com/lib\"\n\nfunc main() { lib.Greet(); \
         println(\"changed\") }\n",
    )
    .unwrap();
    assert_ne!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "module source changes must affect its task hash"
    );
}

#[test]
fn test_go_native_tasks_are_overrideable_and_excludable() {
    if !go_available() {
        return;
    }

    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "lint": { "command": { "go": ["go", "version"] } },
    "dev": { "command": { "go": ["go", "env", "GOVERSION"] } }
  }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "lint", "--filter=example.com/lib", "--dry-run=json"],
    );
    let lint = dry_run_task(&output, "example.com/lib#lint");
    assert_eq!(lint["command"], "go version");

    let output = run_turbo(
        tempdir.path(),
        &["run", "dev", "--filter=example.com/api", "--dry-run=json"],
    );
    let dev = dry_run_task(&output, "example.com/api#dev");
    assert_eq!(dev["command"], "go env GOVERSION");

    fs::write(
        tempdir.path().join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "dev": { "extends": false }
  }
}"#,
    )
    .unwrap();
    let tasks = package_task_names(tempdir.path(), "example.com/api");
    assert!(tasks.iter().any(|task| task == "lint"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "vet"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "run"), "tasks: {tasks:?}");
    assert!(!tasks.iter().any(|task| task == "dev"), "tasks: {tasks:?}");
}
