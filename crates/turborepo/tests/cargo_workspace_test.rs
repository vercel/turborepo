//! End-to-end tests for experimental Cargo workspace support: a mixed
//! npm + Cargo fixture driven through the real turbo binary, covering
//! discovery, execution, caching, invalidation, output restoration, and the
//! opt-in surface (`futureFlags.experimentalCargoWorkspaces`).
//!
//! These tests invoke `cargo build` inside the fixture, so they require a
//! Rust toolchain — which is guaranteed, since the tests themselves are
//! built with one.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{run_turbo, setup};

fn setup_cargo_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "cargo_monorepo", "npm@10.5.0", false).unwrap();
}

/// The fixture's turbo.json opts in via
/// `futureFlags.experimentalCargoWorkspaces`; no environment variable is
/// involved anywhere.
#[test]
fn test_cargo_packages_in_task_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "--dry-run=json"]);
    assert!(output.status.success(), "dry-run failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("dry-run emits JSON");

    let tasks = json["tasks"].as_array().expect("tasks array");
    let task =
        |id: &str| -> Option<&serde_json::Value> { tasks.iter().find(|t| t["taskId"] == id) };

    // The bin crate is an entrypoint: it executes a real cargo command.
    let app_build = task("app#build").expect("app#build in graph");
    assert_eq!(app_build["command"], "cargo build --package=app");
    // Its dependency crate participates in the graph (for --filter/--affected
    // propagation) but is a no-op — cargo builds it implicitly.
    let lib_build = task("lib-a#build").expect("lib-a#build in graph");
    assert_eq!(lib_build["command"], "<NONEXISTENT>");
    // JS packages coexist in the same graph.
    let js_build = task("js-pkg#build").expect("js-pkg#build in graph");
    assert!(
        js_build["command"]
            .as_str()
            .is_some_and(|c| c.contains("echo")),
        "js task keeps its script command, got {js_build:?}"
    );

    // The entrypoint's hash covers its dependency crate's sources and the
    // crate's bin deliverable is the cached output.
    let inputs: Vec<&str> = app_build["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        inputs.iter().any(|i| i.contains("crates/lib-a")),
        "dependency crate sources must be inputs, got {inputs:?}"
    );
    let outputs: Vec<&str> = app_build["resolvedTaskDefinition"]["outputs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        outputs.iter().any(|o| o.ends_with("target/*/app")),
        "bin deliverable must be an output, got {outputs:?}"
    );
}

#[test]
fn test_cargo_build_executes_caches_and_restores() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    // Cold: executes cargo.
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    assert!(output.status.success(), "cold build failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"), "expected miss: {stdout}");
    let bin = tempdir
        .path()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) { "app.exe" } else { "app" });
    assert!(bin.exists(), "cargo build must produce the binary");

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
    // without executing cargo.
    fs::remove_file(&bin).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "restore run should be fully cached: {stdout}"
    );
    assert!(bin.exists(), "deliverable must be restored from cache");
}

#[test]
fn test_dependency_crate_change_invalidates_entrypoint() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "--filter=app"]);
    assert!(output.status.success(), "cold build failed: {output:?}");

    // Content change in the dependency crate must invalidate the
    // entrypoint's task, with no dependsOn wiring in the fixture's
    // turbo.json beyond the default ^build.
    let lib = tempdir.path().join("crates/lib-a/src/lib.rs");
    fs::write(
        &lib,
        "pub fn greeting() -> &'static str {\n    \"changed\"\n}\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "dependency source change must invalidate the entrypoint: {stdout}"
    );
}

/// Prune produces a self-contained Cargo workspace: kept crate dirs, a
/// lockfile subset, and a rewritten root manifest — proven by building the
/// pruned output with `cargo build --locked`.
#[test]
fn test_prune_produces_buildable_cargo_workspace() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    // Prune requires a lockfile; generate it the way a real repo has one.
    let status = std::process::Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(tempdir.path())
        .status()
        .expect("cargo generate-lockfile runs");
    assert!(status.success());

    let output = run_turbo(tempdir.path(), &["prune", "app"]);
    assert!(output.status.success(), "prune failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added app"), "{stdout}");
    assert!(stdout.contains("Added lib-a"), "{stdout}");

    let out = tempdir.path().join("out");
    assert!(out.join("crates/app/src/main.rs").exists());
    assert!(out.join("crates/lib-a/src/lib.rs").exists());
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("Cargo.lock").exists());
    // The JS package is not in app's closure and must not be copied.
    assert!(!out.join("packages/js-pkg").exists());

    // Members are the explicit kept set.
    let manifest = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains(r#"members = ["crates/app", "crates/lib-a"]"#),
        "explicit members expected, got: {manifest}"
    );

    // The decisive assertion: the pruned workspace builds with the pruned
    // lockfile, strictly.
    let build = std::process::Command::new("cargo")
        .args(["build", "--locked", "-p", "app"])
        .current_dir(&out)
        .output()
        .expect("cargo build runs");
    assert!(
        build.status.success(),
        "pruned workspace must build --locked: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(out.join("target/debug/app"))
        .output()
        .expect("pruned binary runs");
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("hello from lib-a"),
        "pruned binary output: {run:?}"
    );
}

/// Docker layout: the json layer carries everything needed to resolve
/// dependencies (manifests + lockfile), the full layer carries sources.
#[test]
fn test_prune_docker_layout_for_cargo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());
    let status = std::process::Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(tempdir.path())
        .status()
        .expect("cargo generate-lockfile runs");
    assert!(status.success());

    let output = run_turbo(tempdir.path(), &["prune", "app", "--docker"]);
    assert!(output.status.success(), "prune --docker failed: {output:?}");

    let out = tempdir.path().join("out");
    for file in [
        "json/Cargo.toml",
        "json/Cargo.lock",
        "json/crates/app/Cargo.toml",
        "json/crates/lib-a/Cargo.toml",
        "full/crates/app/src/main.rs",
        "full/crates/lib-a/src/lib.rs",
        "full/Cargo.toml",
        "full/Cargo.lock",
    ] {
        assert!(out.join(file).exists(), "missing {file} in docker layout");
    }
    // Sources stay out of the json layer.
    assert!(!out.join("json/crates/app/src").exists());
}

/// A JS-only target in a mixed repo prunes exactly as before: no crates, no
/// Cargo workspace files.
#[test]
fn test_prune_js_target_unaffected_by_cargo() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "js-pkg"]);
    assert!(output.status.success(), "prune failed: {output:?}");

    let out = tempdir.path().join("out");
    assert!(out.join("packages/js-pkg/package.json").exists());
    assert!(!out.join("crates").exists());
    assert!(!out.join("Cargo.toml").exists());
}

/// The synthetic `cargo` package has no directory of its own and is not a
/// pruneable target.
#[test]
fn test_prune_cargo_workspace_package_rejected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "cargo"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("has no directory of its own"),
        "expected guard message: {stderr}"
    );
}

#[test]
fn test_filter_hint_when_cargo_disabled() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    // Remove the opt-in: crates vanish from the graph, and filtering for
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
        stderr.contains("experimentalCargoWorkspaces"),
        "expected the opt-in hint: {stderr}"
    );
}
