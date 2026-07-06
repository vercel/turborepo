//! End-to-end tests for experimental uv workspace support: a mixed npm + uv
//! fixture driven through the real turbo binary, covering discovery,
//! execution, caching, invalidation, output restoration, pruning, and the
//! two opt-in surfaces (`futureFlags.uvWorkspaces` and
//! `TURBO_EXPERIMENTAL_UV`).
//!
//! Unlike the Cargo suite (whose toolchain is guaranteed because the tests
//! are built with one), uv may be absent from the machine. Tests that need
//! to execute uv skip in that case; graph-only assertions (discovery,
//! filter hints, JS-target pruning) run everywhere.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Command};

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_uv_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "uv_monorepo", "npm@10.5.0", false).unwrap();
}

/// Whether uv is installed; tests that execute uv skip when it isn't.
fn uv_available() -> bool {
    if which::which("uv").is_ok() {
        true
    } else {
        eprintln!("skipping: uv is not installed");
        false
    }
}

fn generate_uv_lock(dir: &Path) {
    let status = Command::new("uv")
        .arg("lock")
        .current_dir(dir)
        .status()
        .expect("uv lock runs");
    assert!(status.success());
}

/// The fixture's turbo.json opts in via `futureFlags.uvWorkspaces`; no
/// environment variable is needed anywhere in these tests unless noted.
#[test]
fn test_uv_packages_in_task_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "--dry-run=json"]);
    assert!(output.status.success(), "dry-run failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("dry-run emits JSON");

    let tasks = json["tasks"].as_array().expect("tasks array");
    let task =
        |id: &str| -> Option<&serde_json::Value> { tasks.iter().find(|t| t["taskId"] == id) };

    // Packaged projects execute real uv commands.
    let app_build = task("app#build").expect("app#build in graph");
    assert_eq!(app_build["command"], "uv build --package=app");
    let lib_build = task("lib-a#build").expect("lib-a#build in graph");
    assert_eq!(lib_build["command"], "uv build --package=lib-a");
    // The virtual member participates in the graph (for --filter/--affected
    // propagation) but is a no-op — there is nothing for uv to build.
    let tools_build = task("tools#build").expect("tools#build in graph");
    assert_eq!(tools_build["command"], "<NONEXISTENT>");
    // JS packages coexist in the same graph.
    let js_build = task("js-pkg#build").expect("js-pkg#build in graph");
    assert!(
        js_build["command"]
            .as_str()
            .is_some_and(|c| c.contains("echo")),
        "js task keeps its script command, got {js_build:?}"
    );

    // The packaged project's hash covers its dependency member's sources
    // and the project's wheel/sdist are the cached outputs.
    let inputs: Vec<&str> = app_build["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        inputs.iter().any(|i| i.contains("python/lib-a")),
        "dependency member sources must be inputs, got {inputs:?}"
    );
    let outputs: Vec<&str> = app_build["resolvedTaskDefinition"]["outputs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        outputs.iter().any(|o| o.ends_with("dist/app-*")),
        "wheel/sdist deliverables must be outputs, got {outputs:?}"
    );
}

#[test]
fn test_uv_build_executes_caches_and_restores() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    // Cold: executes uv.
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    assert!(output.status.success(), "cold build failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"), "expected miss: {stdout}");
    let wheel = tempdir.path().join("dist/app-0.1.0-py3-none-any.whl");
    assert!(wheel.exists(), "uv build must produce the wheel");

    // Warm: everything from cache.
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "second run should be fully cached: {stdout}"
    );

    // Deleting the deliverable and re-running restores it from cache
    // without executing uv.
    fs::remove_file(&wheel).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "restore run should be fully cached: {stdout}"
    );
    assert!(wheel.exists(), "deliverable must be restored from cache");
}

#[test]
fn test_dependency_member_change_invalidates_packaged_project() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "--filter=app"]);
    assert!(output.status.success(), "cold build failed: {output:?}");

    // Content change in the dependency member must invalidate the packaged
    // project's task, with no dependsOn wiring in the fixture's turbo.json
    // beyond the default ^build.
    let lib = tempdir.path().join("python/lib-a/src/lib_a/__init__.py");
    fs::write(&lib, "def greeting() -> str:\n    return \"changed\"\n").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "dependency source change must invalidate the packaged project: {stdout}"
    );
}

/// The synthetic `uv` package hosts workspace-scoped verbs: `uv#sync` runs
/// `uv sync --locked` and materializes the environment.
#[test]
fn test_uv_sync_workspace_task() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());
    generate_uv_lock(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["sync", "--filter=uv", "--log-order", "grouped"],
    );
    assert!(output.status.success(), "sync failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("uv sync --locked") || stdout.contains("Installed"),
        "expected uv sync execution: {stdout}"
    );
    assert!(
        tempdir.path().join(".venv").exists(),
        "uv sync must materialize the environment"
    );
}

#[test]
fn test_filter_hint_when_uv_disabled() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    // Remove the opt-in: members vanish from the graph, and filtering for
    // one should point the user at the flag.
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{ "tasks": { "build": { "dependsOn": ["^build"] } } }"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--filter=app"]);
    assert!(!output.status.success(), "filter miss must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No package found with name 'app'"),
        "expected filter miss: {stderr}"
    );
    assert!(
        stderr.contains("uvWorkspaces"),
        "expected the opt-in hint: {stderr}"
    );
}

/// Prune produces a self-contained uv workspace: kept member dirs, a
/// lockfile subset, and a rewritten root manifest — proven by syncing the
/// pruned output with `uv sync --locked` and running the application.
#[test]
fn test_prune_produces_syncable_uv_workspace() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());
    generate_uv_lock(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "app"]);
    assert!(output.status.success(), "prune failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added app"), "{stdout}");
    assert!(stdout.contains("Added lib-a"), "{stdout}");

    let out = tempdir.path().join("out");
    assert!(out.join("python/app/src/app/__init__.py").exists());
    assert!(out.join("python/lib-a/src/lib_a/__init__.py").exists());
    assert!(out.join("pyproject.toml").exists());
    assert!(out.join("uv.lock").exists());
    // Neither the JS package nor the virtual member is in app's closure;
    // they must not be copied.
    assert!(!out.join("packages/js-pkg").exists());
    assert!(!out.join("python/tools").exists());

    // Members are the explicit kept set.
    let manifest = fs::read_to_string(out.join("pyproject.toml")).unwrap();
    assert!(
        manifest.contains(r#"members = ["python/app", "python/lib-a"]"#),
        "explicit members expected, got: {manifest}"
    );

    // The decisive assertion: the pruned workspace syncs with the pruned
    // lockfile, strictly, and the application runs.
    let sync = Command::new("uv")
        .args(["sync", "--locked"])
        .current_dir(&out)
        .output()
        .expect("uv sync runs");
    assert!(
        sync.status.success(),
        "pruned workspace must sync --locked: {}",
        String::from_utf8_lossy(&sync.stderr)
    );
    let run = Command::new("uv")
        .args(["run", "app"])
        .current_dir(&out)
        .output()
        .expect("uv run runs");
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("hello from lib-a"),
        "pruned application output: {run:?}"
    );
}

/// Docker layout: the json layer carries everything needed to resolve
/// dependencies (manifests + lockfile), the full layer carries sources.
#[test]
fn test_prune_docker_layout_for_uv() {
    if !uv_available() {
        return;
    }
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());
    generate_uv_lock(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "app", "--docker"]);
    assert!(output.status.success(), "prune --docker failed: {output:?}");

    let out = tempdir.path().join("out");
    for file in [
        "json/pyproject.toml",
        "json/uv.lock",
        "json/python/app/pyproject.toml",
        "json/python/lib-a/pyproject.toml",
        "full/python/app/src/app/__init__.py",
        "full/python/lib-a/src/lib_a/__init__.py",
        "full/pyproject.toml",
        "full/uv.lock",
    ] {
        assert!(out.join(file).exists(), "missing {file} in docker layout");
    }
    // Sources stay out of the json layer.
    assert!(!out.join("json/python/app/src").exists());
}

/// A JS-only target in a mixed repo prunes exactly as before: no members,
/// no uv workspace files.
#[test]
fn test_prune_js_target_unaffected_by_uv() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "js-pkg"]);
    assert!(output.status.success(), "prune failed: {output:?}");

    let out = tempdir.path().join("out");
    assert!(out.join("packages/js-pkg/package.json").exists());
    assert!(!out.join("python").exists());
    assert!(!out.join("pyproject.toml").exists());
}

/// The synthetic `uv` package is not a pruneable target.
#[test]
fn test_prune_uv_workspace_package_rejected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "uv"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("synthetic uv workspace package"),
        "expected guard message: {stderr}"
    );
}

/// The environment variable is the other opt-in surface: without the
/// future flag in turbo.json, `TURBO_EXPERIMENTAL_UV=1` produces the same
/// uv-aware graph.
#[test]
fn test_env_var_opt_in_builds_uv_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_uv_monorepo(tempdir.path());

    // Remove the config opt-in.
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{ "tasks": { "build": { "dependsOn": ["^build"] } } }"#,
    )
    .unwrap();

    // Without either surface, the member is invisible.
    let output = run_turbo(tempdir.path(), &["build", "--filter=app"]);
    assert!(!output.status.success());

    // With the env var, the member task exists and carries its command.
    let output = run_turbo_with_env(
        tempdir.path(),
        &["build", "--filter=app", "--dry-run=json"],
        &[("TURBO_EXPERIMENTAL_UV", "1")],
    );
    assert!(output.status.success(), "env opt-in failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("dry-run emits JSON");
    let command = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|t| t["taskId"] == "app#build"))
        .map(|t| t["command"].clone())
        .expect("app#build in graph");
    assert_eq!(command, "uv build --package=app");
}
