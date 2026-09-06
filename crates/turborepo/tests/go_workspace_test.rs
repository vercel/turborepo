//! End-to-end tests for experimental Go workspace support.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::setup;

const AMBIENT_GO_ENV: &[&str] = &[
    "GO111MODULE",
    "GOARCH",
    "GOENV",
    "GOEXPERIMENT",
    "GOFLAGS",
    "GOOS",
    "GOTOOLCHAIN",
    "GOWORK",
];

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
    for name in AMBIENT_GO_ENV {
        command.env_remove(name);
    }
    command
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path());
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

fn task_hash(dir: &Path, package: &str, task: &str) -> String {
    let filter = format!("--filter={package}");
    let output = run_turbo(dir, &["run", task, &filter, "--dry-run=json"]);
    assert_command_success(&output, "task dry run");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON");
    let task_id = format!("{package}#{task}");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == task_id))
        .and_then(|task| task["hash"].as_str())
        .expect("task has a hash")
        .to_string()
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
fn test_native_go_tasks_execute_and_restore_cached_binary() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());

    for task in ["test", "vet"] {
        let output = run_turbo(
            tempdir.path(),
            &["run", task, "--filter=go-workspace", "--log-order=grouped"],
        );
        assert_command_success(&output, &format!("workspace {task}"));
    }

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "first Go build");
    let binary =
        tempdir
            .path()
            .join("apps/api/dist")
            .join(if cfg!(windows) { "api.exe" } else { "api" });
    assert!(binary.exists(), "native build must produce {binary:?}");

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "cached Go build");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"),
        "second build must hit cache: {output:?}"
    );

    fs::remove_file(&binary).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=example.com/api",
            "--log-order=grouped",
        ],
    );
    assert_command_success(&output, "restored Go build");
    assert!(binary.exists(), "cache hit must restore the binary");

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=example.com/api", "--", "argument"],
    );
    assert_command_success(&output, "native Go run with pass-through argument");
}

#[test]
fn test_go_task_hash_tracks_dependencies_but_not_unrelated_files() {
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
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() { println(\"changed\") }\n",
    )
    .unwrap();
    assert_ne!(
        original,
        task_hash(tempdir.path(), "example.com/api", "build"),
        "internal dependency sources must affect dependent module hashes"
    );
}

#[test]
fn test_affected_go_tasks_follow_internal_module_relationships() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("packages/lib/lib.go"),
        "package lib\n\nfunc Greet() { println(\"affected\") }\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"build\"]) { items { name package { name } } } }",
        ],
    );
    assert_command_success(&output, "Go affected task query");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("affected query JSON");
    let tasks = json["data"]["affectedTasks"]["items"]
        .as_array()
        .expect("affected tasks");
    for package in ["example.com/lib", "example.com/api"] {
        assert!(
            tasks
                .iter()
                .any(|task| task["name"] == "build" && task["package"]["name"] == package),
            "{package}#build must be affected: {tasks:?}"
        );
    }
}

#[test]
fn test_query_exposes_go_tasks_and_package_exclusion() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let query = |dir: &Path| {
        let output = run_turbo(
            dir,
            &[
                "query",
                "query { package(name: \"example.com/api\") { tasks { items { name command } } } }",
            ],
        );
        assert_command_success(&output, "Go task query");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("query JSON")
    };
    let tasks = query(tempdir.path());
    let items = tasks["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items");
    assert!(items.iter().any(|task| {
        task["name"] == "build"
            && task["command"]
                .as_str()
                .is_some_and(|command| command.contains("go build -o dist/api"))
    }));
    assert!(items.iter().any(|task| task["name"] == "run"));
    assert!(items.iter().any(|task| task["name"] == "vet"));

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalGoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "build": { "command": { "go": ["go", "version"] } }
  }
}"#,
    )
    .unwrap();
    let tasks = query(tempdir.path());
    let build = tasks["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("task items")
        .iter()
        .find(|task| task["name"] == "build")
        .expect("overridden build");
    assert_eq!(build["command"], "go version");

    fs::write(
        tempdir.path().join("apps/api/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": { "build": { "extends": false } }
}"#,
    )
    .unwrap();
    let tasks = query(tempdir.path());
    assert!(
        !tasks["data"]["package"]["tasks"]["items"]
            .as_array()
            .expect("task items")
            .iter()
            .any(|task| task["name"] == "build")
    );
}

#[test]
fn test_go_prune_produces_valid_buildable_workspace() {
    if !go_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_go_pure_workspace(tempdir.path());
    let output = run_turbo(tempdir.path(), &["prune", "example.com/api"]);
    assert_command_success(&output, "Go prune");

    let pruned = tempdir.path().join("out");
    let go_work = fs::read_to_string(pruned.join("go.work")).expect("pruned go.work");
    assert!(go_work.contains("./apps/api"));
    assert!(go_work.contains("./packages/lib"));
    assert!(pruned.join("packages/lib/go.mod").exists());

    let output = std::process::Command::new("go")
        .args(["test", "./..."])
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .current_dir(pruned.join("apps/api"))
        .output()
        .expect("go test runs in pruned module");
    assert_command_success(&output, "pruned Go module tests");

    let output = run_turbo(&pruned, &["run", "build", "--filter=example.com/api"]);
    assert_command_success(&output, "native build in pruned workspace");
}
