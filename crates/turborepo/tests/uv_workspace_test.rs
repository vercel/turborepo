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

fn setup_uv_native_tools(dir: &Path) {
    setup::copy_fixture("uv_native_tools", dir).unwrap();
    setup::setup_git(dir).unwrap();
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

fn append_manifest(dir: &Path, relative: &str, contents: &str) {
    use std::io::Write;

    let path = dir.join(relative);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
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
        "uv check --frozen --all-packages"
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
        "uv check --frozen --package=py-app"
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
fn test_uv_root_native_tools_workspace_fanout_and_qualified_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "pyproject.toml",
        r#"

[dependency-groups]
dev = ["Ruff", "black", "mypy", "ty", "pyright"]
"#,
    );
    let lock_before = fs::read(tempdir.path().join("uv.lock")).unwrap();

    let check = dry_run_tasks(tempdir.path(), &["check"]);
    let ids = task_ids(&check);
    for id in [
        "acme#check",
        "acme#check:mypy",
        "acme#check:ty",
        "acme#check:pyright",
    ] {
        assert!(ids.contains(&id.to_string()), "ids: {ids:?}");
    }
    let dependencies: Vec<&str> = find_task(&check, "acme#check")["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(
        dependencies,
        ["acme#check:mypy", "acme#check:pyright", "acme#check:ty"]
    );
    assert_eq!(
        find_task(&check, "acme#check:mypy")["command"],
        "uv run --active --frozen mypy packages/py-app packages/py-lib"
    );

    let format = dry_run_tasks(tempdir.path(), &["format"]);
    assert_eq!(task_ids(&format), vec!["acme#format".to_string()]);
    assert_eq!(
        find_task(&format, "acme#format")["command"],
        "uv run --active --frozen ruff format packages/py-app packages/py-lib"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "format:ruff",
            "--filter=py-app",
            "--dry-run=json",
            "--",
            "--check",
        ],
    );
    assert_command_success(&output, "qualified formatter dry-run");
    let qualified: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(task_ids(&qualified), vec!["py-app#format:ruff".to_string()]);
    assert_eq!(
        find_task(&qualified, "py-app#format:ruff")["command"],
        "uv run --active --frozen ruff format packages/py-app"
    );

    let warning_output = run_turbo(tempdir.path(), &["format", "--dry-run=json"]);
    assert_command_success(&warning_output, "formatter warning dry-run");
    let stderr = String::from_utf8_lossy(&warning_output.stderr);
    assert_eq!(stderr.matches("declares multiple formatters").count(), 1);
    for expected in [
        "scope \"acme\"",
        "ruff, black",
        "selected ruff",
        "Ruff before Black",
        "format:ruff, format:black",
    ] {
        assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
    }
    assert_eq!(
        fs::read(tempdir.path().join("uv.lock")).unwrap(),
        lock_before
    );
}

#[test]
fn test_uv_heterogeneous_tools_use_member_entrypoints() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_native_tools(tempdir.path());

    let format = dry_run_tasks(tempdir.path(), &["format"]);
    let ids = task_ids(&format);
    assert_eq!(
        ids,
        ["py-app#format".to_string(), "py-lib#format".to_string()]
    );
    assert_eq!(
        find_task(&format, "py-app#format")["command"],
        "uv run --active --frozen --package py-app black packages/py-app"
    );
    assert_eq!(
        find_task(&format, "py-lib#format")["command"],
        "uv run --active --frozen ruff format packages/py-lib"
    );

    let check = dry_run_tasks(tempdir.path(), &["check"]);
    let ids = task_ids(&check);
    assert!(
        !ids.iter().any(|id| id.starts_with("acme#")),
        "ids: {ids:?}"
    );
    for id in [
        "py-app#check",
        "py-app#check:pyright",
        "py-lib#check",
        "py-lib#check:mypy",
    ] {
        assert!(ids.contains(&id.to_string()), "ids: {ids:?}");
    }
    for package in ["py-app", "py-lib"] {
        let dependencies = find_task(&check, &format!("{package}#check"))["dependencies"]
            .as_array()
            .unwrap();
        assert!(dependencies.iter().all(|dependency| {
            dependency
                .as_str()
                .is_some_and(|dependency| dependency.starts_with(package))
        }));
    }

    let lint = dry_run_tasks(tempdir.path(), &["lint"]);
    assert!(task_ids(&lint).contains(&"acme#lint".to_string()));
    assert_eq!(
        find_task(&lint, "acme#lint:ruff")["command"],
        "uv run --active --frozen ruff check packages/py-app packages/py-lib"
    );
}

#[test]
fn test_uv_compatible_member_tools_use_all_packages_entrypoint() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    for member in ["py-app", "py-lib"] {
        append_manifest(
            tempdir.path(),
            &format!("packages/{member}/pyproject.toml"),
            "\n[dependency-groups]\ndev = [\"ruff\"]\n",
        );
    }

    let lint = dry_run_tasks(tempdir.path(), &["lint"]);
    assert!(task_ids(&lint).contains(&"acme#lint".to_string()));
    assert_eq!(
        find_task(&lint, "acme#lint:ruff")["command"],
        "uv run --active --frozen --all-packages ruff check packages/py-app packages/py-lib"
    );

    let filtered = dry_run_tasks(tempdir.path(), &["lint:ruff", "--filter=py-app"]);
    assert_eq!(task_ids(&filtered), ["py-app#lint:ruff"]);
    assert_eq!(
        find_task(&filtered, "py-app#lint:ruff")["command"],
        "uv run --active --frozen --package py-app ruff check packages/py-app"
    );
}

#[test]
fn test_uv_root_pytest_is_workspace_only_and_member_filter_uses_direct_declaration() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "pyproject.toml",
        "\n[dependency-groups]\ndev = [\"pytest\"]\n",
    );
    append_manifest(
        tempdir.path(),
        "packages/py-app/pyproject.toml",
        "\n[dependency-groups]\ndev = [\"pytest\"]\n",
    );

    let workspace = dry_run_tasks(tempdir.path(), &["test"]);
    assert_eq!(task_ids(&workspace), ["acme#test"]);
    assert_eq!(
        find_task(&workspace, "acme#test")["command"],
        "uv run --active --frozen pytest"
    );

    let app = dry_run_tasks(tempdir.path(), &["test", "--filter=py-app"]);
    assert_eq!(task_ids(&app), ["py-app#test"]);
    assert_eq!(
        find_task(&app, "py-app#test")["command"],
        "uv run --active --frozen --package py-app pytest packages/py-app"
    );

    let lib = dry_run_tasks(tempdir.path(), &["test", "--filter=py-lib"]);
    assert!(task_ids(&lib).is_empty());
}

#[test]
fn test_uv_member_pytest_tasks_run_per_declaring_package() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "packages/py-app/pyproject.toml",
        "\n[dependency-groups]\ndev = [\"pytest\"]\n",
    );
    append_manifest(
        tempdir.path(),
        "packages/py-lib/pyproject.toml",
        "\n[dependency-groups]\ntests = [\"pytest\"]\n\n[tool.uv]\ndefault-groups = []\n",
    );

    let test = dry_run_tasks(tempdir.path(), &["test"]);
    assert_eq!(task_ids(&test), ["py-app#test", "py-lib#test"]);
    assert_eq!(
        find_task(&test, "py-app#test")["command"],
        "uv run --active --frozen --package py-app pytest packages/py-app"
    );
    assert_eq!(
        find_task(&test, "py-lib#test")["command"],
        "uv run --active --frozen --package py-lib --no-default-groups --group tests pytest \
         packages/py-lib"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "test",
            "--filter=py-app",
            "--dry-run=json",
            "--",
            "-k",
            "smoke",
        ],
    );
    assert_command_success(&output, "member pytest pass-through dry-run");
    let filtered: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        find_task(&filtered, "py-app#test")["command"],
        "uv run --active --frozen --package py-app pytest packages/py-app"
    );
}

#[test]
fn test_uv_explicit_test_command_does_not_require_pytest_declaration() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "futureFlags": {
    "experimentalPythonWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "test": {
      "command": { "python": ["echo", "configured-python-test"] }
    }
  }
}"#,
    )
    .unwrap();

    let test = dry_run_tasks(tempdir.path(), &["test", "--filter=py-app"]);
    assert_eq!(task_ids(&test), ["py-app#test"]);
    assert_eq!(
        find_task(&test, "py-app#test")["command"],
        "echo configured-python-test"
    );
}

#[test]
fn test_uv_multi_child_check_rejects_pass_through_args() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "pyproject.toml",
        "\n[dependency-groups]\ndev = [\"mypy\", \"ty\", \"pyright\"]\n",
    );

    let output = run_turbo(tempdir.path(), &["check", "--", "--strict"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("qualified dependency tasks"),
        "stderr: {stderr}"
    );
    for qualified in ["acme#check:mypy", "acme#check:ty", "acme#check:pyright"] {
        assert!(stderr.contains(qualified), "missing {qualified}: {stderr}");
    }
}

#[test]
fn test_uv_native_tools_are_visible_to_query_in_mixed_repo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());
    append_manifest(
        tempdir.path(),
        "pyproject.toml",
        "\n[dependency-groups]\ndev = [\"ruff\", \"mypy\"]\n",
    );

    let lint = dry_run_tasks(tempdir.path(), &["lint"]);
    let ids = task_ids(&lint);
    assert!(ids.contains(&"pyacme#lint".to_string()), "ids: {ids:?}");
    assert!(
        ids.contains(&"pyacme#lint:ruff".to_string()),
        "ids: {ids:?}"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { package(name: \"py-app\") { tasks { items { name } } } }",
        ],
    );
    assert_command_success(&output, "native task query");
    let query = String::from_utf8_lossy(&output.stdout);
    for task in ["lint", "lint:ruff", "format", "format:ruff", "check:mypy"] {
        assert!(
            query.contains(&format!("\"name\": \"{task}\"")),
            "query: {query}"
        );
    }
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
fn test_uv_lock_metadata_only_change_affects_no_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let lock_path = tempdir.path().join("uv.lock");
    let mut contents = fs::read_to_string(&lock_path).unwrap();
    contents.push_str("\n# turbo-test lockfile perturbation\n");
    fs::write(&lock_path, contents).unwrap();

    let json = dry_run_tasks(
        tempdir.path(),
        &["build", "--filter=[HEAD]", "--log-order", "grouped"],
    );
    assert_eq!(task_ids(&json), Vec::<String>::new());
}

#[test]
fn test_uv_lock_change_only_affects_dependency_closure() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "packages/py-app/pyproject.toml",
        "\n[dependency-groups]\ndev = [\"ruff==0.12.0\"]\n",
    );
    let lock = std::process::Command::new("uv")
        .arg("lock")
        .current_dir(tempdir.path())
        .output()
        .expect("uv lock runs");
    assert_command_success(&lock, "initial uv lock");
    let add = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(tempdir.path())
        .output()
        .expect("git add runs");
    assert_command_success(&add, "git add");
    let commit = std::process::Command::new("git")
        .args(["commit", "-m", "add ruff"])
        .current_dir(tempdir.path())
        .output()
        .expect("git commit runs");
    assert_command_success(&commit, "git commit");

    let manifest = tempdir.path().join("packages/py-app/pyproject.toml");
    let contents = fs::read_to_string(&manifest)
        .unwrap()
        .replace("ruff==0.12.0", "ruff==0.12.1");
    fs::write(manifest, contents).unwrap();
    let lock = std::process::Command::new("uv")
        .arg("lock")
        .current_dir(tempdir.path())
        .output()
        .expect("uv lock runs");
    assert_command_success(&lock, "updated uv lock");

    let json = dry_run_tasks(
        tempdir.path(),
        &["build", "--filter=[HEAD]", "--log-order", "grouped"],
    );
    let ids = task_ids(&json);
    assert!(ids.contains(&"py-app#build".to_string()), "ids: {ids:?}");
    assert!(!ids.contains(&"py-lib#build".to_string()), "ids: {ids:?}");
}

#[test]
fn test_uv_build_caches_bundled_backend() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let run = |args: &[&str]| {
        let mut command = common::turbo_command(tempdir.path());
        command
            .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
            .env("UV_NO_CONFIG", "1")
            .args(args)
            .output()
            .expect("failed to execute turbo")
    };
    let output = run(&["build", "--filter=py-app", "--dry-run=json"]);
    assert_command_success(&output, "build dry-run");
    let dry_run: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        find_task(&dry_run, "py-app#build")["resolvedTaskDefinition"]["cache"],
        true
    );

    let output = run(&["build", "--filter=py-app", "--log-order", "grouped"]);
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

    fs::remove_dir_all(tempdir.path().join("dist")).unwrap();
    let output = run(&["build", "--filter=py-app", "--log-order", "grouped"]);
    assert_command_success(&output, "cached build");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected build cache hit: {stdout}"
    );
    assert!(wheel_exists(), "cache hit must restore the wheel");
}

#[test]
fn test_uv_quality_tasks_are_cacheable_with_toolchain_identity() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    append_manifest(
        tempdir.path(),
        "pyproject.toml",
        "\n[dependency-groups]\ndev = [\"ruff\"]\n",
    );
    let lock = std::process::Command::new("uv")
        .arg("lock")
        .current_dir(tempdir.path())
        .output()
        .expect("uv lock runs");
    assert_command_success(&lock, "uv lock");

    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(tempdir.path());
    let output = command
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("UV_NO_CONFIG", "1")
        .args(["lint:ruff", "--filter=py-lib", "--dry-run=json"])
        .output()
        .expect("failed to execute turbo");
    assert_command_success(&output, "cacheable quality dry-run");
    let dry_run: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        find_task(&dry_run, "py-lib#lint:ruff")["resolvedTaskDefinition"]["cache"],
        true
    );
}

#[test]
fn test_uv_virtual_environment_is_hashed_and_excluded_from_inputs() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let dry_run = |virtual_env: &Path| {
        let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
        let mut command = common::turbo_command(tempdir.path());
        let output = command
            .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
            .env("UV_NO_CONFIG", "1")
            .env("VIRTUAL_ENV", virtual_env)
            .args(["build", "--filter=py-app", "--dry-run=json"])
            .output()
            .expect("failed to execute turbo");
        assert_command_success(&output, "virtual environment dry-run");
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };

    let first_env = tempdir.path().join("envs/first");
    fs::create_dir_all(&first_env).unwrap();
    let first = dry_run(&first_env);
    let first_task = find_task(&first, "py-app#build");
    assert!(
        first_task["environmentVariables"]["specified"]["env"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "VIRTUAL_ENV")
    );
    assert!(
        first_task["resolvedTaskDefinition"]["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input.as_str().is_some_and(|input| {
                input.starts_with('!') && input.ends_with("envs/first/**")
            })),
        "inputs: {}",
        first_task["resolvedTaskDefinition"]["inputs"]
    );

    let second_env = tempdir.path().join("envs/second");
    fs::create_dir_all(&second_env).unwrap();
    let second = dry_run(&second_env);
    assert_ne!(
        first_task["hash"],
        find_task(&second, "py-app#build")["hash"],
        "changing VIRTUAL_ENV must invalidate the task hash"
    );
}

#[test]
fn test_uv_project_environment_is_excluded_from_inputs() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    let project_env = tempdir.path().join("envs/project");
    fs::create_dir_all(&project_env).unwrap();

    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(tempdir.path());
    let output = command
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("UV_NO_CONFIG", "1")
        .env_remove("VIRTUAL_ENV")
        .env("UV_PROJECT_ENVIRONMENT", &project_env)
        .args(["build", "--filter=py-app", "--dry-run=json"])
        .output()
        .expect("failed to execute turbo");
    assert_command_success(&output, "project environment dry-run");
    let dry_run: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task = find_task(&dry_run, "py-app#build");
    assert!(
        task["resolvedTaskDefinition"]["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|input| input.as_str().is_some_and(|input| {
                input.starts_with('!') && input.ends_with("envs/project/**")
            })),
        "inputs: {}",
        task["resolvedTaskDefinition"]["inputs"]
    );
}

#[test]
fn test_uv_virtual_environment_cannot_be_cached_as_output() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "futureFlags": { "experimentalPythonWorkspaces": true },
  "tasks": { "build": { "outputs": ["$TURBO_ROOT$/.venv/**"] } }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=py-app", "--dry-run=json"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot cache Python virtual environment")
            && stderr.contains(".venv")
            && stderr.contains("dist/**"),
        "expected actionable virtual environment output error: {stderr}"
    );
}

#[test]
fn test_uv_prune() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_pure_workspace(tempdir.path());

    let output = common::run_turbo_with_env(
        tempdir.path(),
        &["prune", "py-app"],
        &[("UV_NO_CONFIG", "1")],
    );
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
            .env("UV_NO_CONFIG", "1")
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

    let output = common::run_turbo_with_env(
        tempdir.path(),
        &["prune", "py-lib"],
        &[("UV_NO_CONFIG", "1")],
    );
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
