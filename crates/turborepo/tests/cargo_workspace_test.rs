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
    assert_eq!(app_build["command"], "cargo build --package=app --locked");
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
fn test_cargo_workspace_requires_lockfile() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());
    let lockfile = tempdir.path().join("Cargo.lock");
    fs::remove_file(&lockfile).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--filter=app", "--dry-run=json"]);
    assert!(!output.status.success(), "missing lockfile must fail");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Cargo.lock is required for Cargo workspace caching"),
        "expected actionable lockfile error: {combined}"
    );
    assert!(!lockfile.exists(), "turbo must not generate Cargo.lock");
}

#[test]
fn test_cargo_workspace_rejects_stale_lockfile() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());
    let lockfile = tempdir.path().join("Cargo.lock");
    let original_lockfile = fs::read_to_string(&lockfile).unwrap();
    let manifest = tempdir.path().join("crates/app/Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        contents.replace("version = \"0.1.0\"", "version = \"0.2.0\""),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--filter=app", "--dry-run=json"]);
    assert!(!output.status.success(), "stale lockfile must fail");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Cargo.lock is out of date or could not be validated"),
        "expected actionable stale lockfile error: {combined}"
    );
    assert_eq!(
        fs::read_to_string(lockfile).unwrap(),
        original_lockfile,
        "turbo must not update Cargo.lock"
    );
}

#[test]
fn test_cargo_workspace_rejects_excluded_path_dependency() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let root_manifest = tempdir.path().join("Cargo.toml");
    let contents = fs::read_to_string(&root_manifest).unwrap();
    fs::write(
        &root_manifest,
        contents.replace(
            "resolver = \"2\"",
            "exclude = [\"crates/local\"]\nresolver = \"2\"",
        ),
    )
    .unwrap();
    let app_manifest = tempdir.path().join("crates/app/Cargo.toml");
    let contents = fs::read_to_string(&app_manifest).unwrap();
    fs::write(
        &app_manifest,
        format!("{contents}local = {{ path = \"../local\" }}\n"),
    )
    .unwrap();
    let local = tempdir.path().join("crates/local");
    fs::create_dir_all(local.join("src")).unwrap();
    fs::write(
        local.join("Cargo.toml"),
        "[package]\nname = \"local\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(local.join("src/lib.rs"), "pub fn local() {}\n").unwrap();
    let status = std::process::Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(tempdir.path())
        .status()
        .expect("cargo generate-lockfile runs");
    assert!(status.success());

    for args in [
        &["build", "--filter=app", "--dry-run=json"][..],
        &["prune", "app"][..],
    ] {
        let output = run_turbo(tempdir.path(), args);
        assert!(!output.status.success(), "unsupported path must fail");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("is not a workspace member")
                && combined.contains("hashed")
                && combined.contains("pruned safely"),
            "expected actionable path dependency error: {combined}"
        );
    }
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
fn test_cargo_run_and_dev_default_to_uncached() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=app", "--dry-run=json"],
    );
    assert!(output.status.success(), "run dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let run = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#run"))
        .expect("app#run in graph");
    assert_eq!(run["resolvedTaskDefinition"]["cache"], false);

    let output = run_turbo(
        tempdir.path(),
        &["run", "dev", "--filter=app", "--dry-run=json"],
    );
    assert!(output.status.success(), "dev dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let dev = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#dev"))
        .expect("app#dev in graph");
    assert_eq!(dev["resolvedTaskDefinition"]["cache"], false);

    for _ in 0..2 {
        let output = run_turbo(
            tempdir.path(),
            &["run", "run", "--filter=app", "--log-order", "grouped"],
        );
        assert!(output.status.success(), "cargo run failed: {output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("cache bypass"),
            "cargo run must execute every time: {stdout}"
        );
        assert!(
            stdout.contains("hello from lib-a"),
            "cargo run must start the requested process: {stdout}"
        );
    }
}

#[test]
fn test_explicit_cache_overrides_cargo_run_default() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": { "run": { "cache": true } }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "run", "--filter=app", "--dry-run=json"],
    );
    assert!(output.status.success(), "run dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let run = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#run"))
        .expect("app#run in graph");
    assert_eq!(run["resolvedTaskDefinition"]["cache"], true);
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

/// The synthetic workspace package has no directory of its own and is not
/// a pruneable target.
#[test]
fn test_prune_cargo_workspace_package_rejected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "acme"]);
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

/// A `command` override on Cargo packages: replaces the verb table, applies
/// via the `rust` map key, and defines tasks even for library crates.
#[test]
fn test_command_override_on_cargo_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_monorepo(tempdir.path());

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
            "futureFlags": {
                "experimentalCargoWorkspaces": true,
                "experimentalTaskCommand": true
            },
            "tasks": {
                "greet": { "command": { "rust": ["echo", "hello-from-rust-map"] } },
                "acme#test": { "command": ["echo", "replaced-cargo-test"] }
            }
        }"#,
    )
    .unwrap();

    // The rust map key grants `greet` to every Cargo package, libraries
    // included — no verb table involved.
    let output = run_turbo(tempdir.path(), &["run", "greet", "--filter=lib-a"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "greet failed: {combined}");
    assert!(
        combined.contains("hello-from-rust-map"),
        "map default should apply to crates: {combined}"
    );

    // A scoped override on the workspace package replaces `cargo test`.
    let output = run_turbo(tempdir.path(), &["run", "test", "--filter=acme"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "test failed: {combined}");
    assert!(
        combined.contains("replaced-cargo-test"),
        "override should replace the verb table: {combined}"
    );
}
