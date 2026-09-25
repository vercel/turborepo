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

use common::setup;

const AMBIENT_CARGO_LAYOUT_ENV: &[&str] = &[
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET_DIR",
    "CARGO_BUILD_TARGET",
    "CARGO_BUILD_ARTIFACT_DIR",
    "RUSTC",
    "CARGO_BUILD_RUSTC",
    "HOME",
    "USERPROFILE",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
];

fn ambient_cargo_layout_env_keys() -> Vec<std::ffi::OsString> {
    let mut keys: Vec<_> = AMBIENT_CARGO_LAYOUT_ENV
        .iter()
        .map(std::ffi::OsString::from)
        .collect();
    keys.extend(std::env::vars_os().filter_map(|(name, _)| {
        let name_string = name.to_string_lossy();
        (name_string.starts_with("CARGO_PROFILE_") && name_string.ends_with("_DIR_NAME"))
            .then_some(name)
    }));
    keys
}

fn cargo_ancestors_are_clean(dir: &Path) -> bool {
    dir.ancestors().skip(1).all(|ancestor| {
        ["config.toml", "config"]
            .iter()
            .all(|name| !ancestor.join(".cargo").join(name).exists())
    })
}

fn cargo_tempdir() -> tempfile::TempDir {
    let current = std::env::current_dir().expect("current directory is available");
    let drive_root = current
        .ancestors()
        .last()
        .expect("current directory has a filesystem root");
    if let Ok(tempdir) = tempfile::Builder::new()
        .prefix("turbo-cargo-")
        .tempdir_in(drive_root)
        && cargo_ancestors_are_clean(tempdir.path())
    {
        return tempdir;
    }

    let tempdir = tempfile::tempdir().expect("fallback Cargo fixture root is available");
    assert!(
        cargo_ancestors_are_clean(tempdir.path()),
        "cannot create a Cargo fixture without inherited ancestor config"
    );
    tempdir
}

fn isolated_cargo_environment(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let home = dir.join(".test-home");
    let cargo_home = home.join(".cargo");
    fs::create_dir_all(&cargo_home).unwrap();
    (home, cargo_home)
}

fn active_rustup_toolchain() -> Option<String> {
    let output = std::process::Command::new("rustup")
        .args(["show", "active-toolchain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

fn rustup_home() -> Option<std::path::PathBuf> {
    std::env::var_os("RUSTUP_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".rustup"))
        })
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| std::path::Path::new(&home).join(".rustup"))
        })
}

fn cargo_command(dir: &Path) -> std::process::Command {
    let (home, cargo_home) = isolated_cargo_environment(dir);
    let mut command = std::process::Command::new("cargo");
    for name in ambient_cargo_layout_env_keys() {
        command.env_remove(name);
    }
    command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CARGO_HOME", cargo_home)
        .current_dir(dir);
    if let Some(rustup_home) = rustup_home() {
        command.env("RUSTUP_HOME", rustup_home);
    }
    command
}

fn run_turbo(dir: &Path, args: &[&str]) -> std::process::Output {
    run_turbo_with_env(dir, args, &[])
}

fn run_turbo_with_env(
    dir: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> std::process::Output {
    let (home, cargo_home) = isolated_cargo_environment(dir);
    let config_dir = tempfile::tempdir().expect("failed to create config tempdir");
    let mut command = common::turbo_command(dir);
    for name in ambient_cargo_layout_env_keys() {
        command.env_remove(name);
    }
    command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CARGO_HOME", &cargo_home)
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path());
    if let Some(rustup_home) = rustup_home() {
        command.env("RUSTUP_HOME", rustup_home);
    }
    for (name, value) in environment {
        command.env(name, value);
    }
    command
        .args(args)
        .output()
        .expect("failed to execute turbo")
}

fn setup_cargo_monorepo(dir: &Path) {
    setup::setup_integration_test(dir, "cargo_monorepo", "npm@10.5.0", false).unwrap();
}

/// A pure Cargo workspace: no root package.json and no JavaScript package
/// manager. `setup_integration_test` can't be used because it writes a
/// `packageManager` field into a package.json that does not exist here, so
/// the fixture is copied and committed directly.
fn setup_cargo_pure_workspace(dir: &Path) {
    setup::copy_fixture("cargo_pure_workspace", dir).unwrap();
    setup::setup_git(dir).unwrap();
    assert!(
        !dir.join("package.json").exists(),
        "the pure Cargo fixture must have no package.json"
    );
}

fn setup_cargo_root_package(dir: &Path) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        r#"[package]
name = "root-app"
version = "0.1.0"
edition = "2021"

[workspace]
resolver = "2"

[workspace.metadata]
name = "rust-workspace"
"#,
    )
    .unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        dir.join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": { "build": {} }
}"#,
    )
    .unwrap();
    let output = std::process::Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(dir)
        .output()
        .unwrap();
    assert_command_success(&output, "generate root package lockfile");
    setup::setup_git(dir).unwrap();
}

fn cargo_binary(dir: &Path, segments: &[&str]) -> std::path::PathBuf {
    let mut path = dir.to_path_buf();
    path.extend(segments);
    path.push(if cfg!(windows) { "app.exe" } else { "app" });
    path
}

fn rustc_host_target() -> String {
    let output = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc runs");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("rustc output is UTF-8")
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("rustc reports host target")
        .to_string()
}

fn alternate_host_target(host: &str) -> &'static str {
    if host == "x86_64-unknown-linux-gnu" {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn run_cargo_build(dir: &Path, cargo_args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut args = vec!["build", "--filter=app", "--log-order", "grouped"];
    if !cargo_args.is_empty() {
        args.push("--");
        args.extend_from_slice(cargo_args);
    }
    run_turbo_with_env(dir, &args, env)
}

fn create_directory_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();

    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("failed to create Cargo test junction");
        assert!(
            status.success(),
            "failed to create test junction {link:?} -> {target:?}"
        );
    }
}

fn assert_command_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_build_without_outputs(dir: &Path) {
    fs::write(
        dir.join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": { "build": { "dependsOn": ["^build"] } }
}"#,
    )
    .unwrap();
}

fn assert_isolated_restoration(
    first_args: &[&str],
    first_path: &[&str],
    second_args: &[&str],
    second_path: &[&str],
) {
    assert_isolated_restoration_with_env(
        first_args,
        first_path,
        second_args,
        second_path,
        &[],
        &[],
    );
}

fn assert_isolated_restoration_with_env(
    first_args: &[&str],
    first_path: &[&str],
    second_args: &[&str],
    second_path: &[&str],
    first_env: &[(&str, &str)],
    second_env: &[(&str, &str)],
) {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let first = cargo_binary(tempdir.path(), first_path);
    let second = cargo_binary(tempdir.path(), second_path);

    let output = run_cargo_build(tempdir.path(), first_args, first_env);
    assert_command_success(&output, "first build");
    assert!(first.exists(), "first deliverable missing: {first:?}");
    let output = run_cargo_build(tempdir.path(), second_args, second_env);
    assert_command_success(&output, "second build");
    assert!(second.exists(), "second deliverable missing: {second:?}");

    fs::remove_file(&first).unwrap();
    fs::remove_file(&second).unwrap();
    let output = run_cargo_build(tempdir.path(), second_args, second_env);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_command_success(&output, "cache restore");
    // The configured library dependencies run uncached; the app must hit cache.
    assert!(
        stdout.contains("app:build: cache hit"),
        "expected cache hit: {stdout}"
    );
    assert!(second.exists(), "effective deliverable was not restored");
    assert!(
        !first.exists(),
        "cache restored a deliverable from another Cargo layout"
    );
}

/// The fixture's turbo.json opts in via
/// `futureFlags.experimentalCargoWorkspaces`; no environment variable is
/// involved anywhere.
#[test]
fn test_cargo_packages_in_task_graph() {
    let tempdir = cargo_tempdir();
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
    let app_directory = Path::new("crates").join("app");
    assert_eq!(
        app_build["directory"].as_str().map(Path::new),
        Some(app_directory.as_path())
    );
    let app_log = app_directory.join(".turbo").join("turbo-build.log");
    assert_eq!(
        app_build["logFile"].as_str().map(Path::new),
        Some(app_log.as_path())
    );
    // Unfiltered builds select entrypoint crates, but the configured ^build
    // dependency must still include the library task.
    assert!(task("lib-a#build").is_some());
    assert!(
        app_build["dependencies"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("lib-a#build"))
    );
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
    let output_name = if cfg!(windows) { "app.exe" } else { "app" };
    let cargo_outputs: Vec<_> = outputs
        .iter()
        .filter(|output| output.contains("/target/"))
        .copied()
        .collect();
    let expected_output = format!("../../target/debug/{output_name}");
    assert_eq!(cargo_outputs, [expected_output.as_str()]);
    assert!(cargo_outputs.iter().all(|output| !output.contains('*')));
}

#[test]
fn test_cargo_root_package_can_be_filtered_and_built() {
    let tempdir = cargo_tempdir();
    setup_cargo_root_package(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=root-app", "--dry-run=json"],
    );
    assert_command_success(&output, "root package dry run");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["packages"], serde_json::json!(["root-app"]));
    let task = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "root-app#build"))
        .expect("root-app#build in graph");
    assert_eq!(task["command"], "cargo build --package=root-app --locked");

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=root-app"]);
    assert_command_success(&output, "root package build");
    let binary = tempdir
        .path()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) {
            "root-app.exe"
        } else {
            "root-app"
        });
    assert!(
        binary.exists(),
        "root package binary must exist at {binary:?}"
    );

    let output = run_turbo(tempdir.path(), &["prune", "root-app"]);
    assert!(
        !output.status.success(),
        "root package prune must fail closed"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("has no directory of its own"),
        "root package prune must explain the limitation: {output:?}"
    );
}

#[test]
fn test_cargo_library_build_can_be_filtered() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    configure_build_without_outputs(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=lib-a", "--dry-run=json"],
    );
    assert!(output.status.success(), "dry run failed: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Cargo library artifacts have no stable outputs for Turborepo to restore"),
        "library build must explain why caching is disabled: {output:?}"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "lib-a#build"))
        .expect("lib-a#build in graph");
    assert_eq!(task["command"], "cargo build --package=lib-a --locked");
    assert_eq!(task["resolvedTaskDefinition"]["cache"], false);

    let output = run_turbo(tempdir.path(), &["run", "lib-a#build", "--dry-run=json"]);
    assert!(output.status.success(), "package task failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["tasks"].as_array().is_some_and(|tasks| {
        tasks.iter().any(|task| {
            task["taskId"] == "lib-a#build"
                && task["command"] == "cargo build --package=lib-a --locked"
        })
    }));

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=lib-a",
            "--force",
            "--log-order=grouped",
        ],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "filtered build failed: {combined}");
    assert!(
        combined.contains("lib-a:build"),
        "expected the library build task to execute: {combined}"
    );
    assert!(!combined.contains("No tasks were executed"));
}

#[test]
fn test_unfiltered_cargo_build_falls_back_to_libraries() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::remove_dir_all(tempdir.path().join("crates/app")).unwrap();
    let status = cargo_command(tempdir.path())
        .arg("generate-lockfile")
        .status()
        .unwrap();
    assert!(status.success());

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry-run=json"]);
    assert!(output.status.success(), "dry run failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = json["tasks"].as_array().unwrap();
    let command = |task_id: &str| {
        tasks
            .iter()
            .find(|task| task["taskId"] == task_id)
            .and_then(|task| task["command"].as_str())
    };
    assert_eq!(
        command("lib-a#build"),
        Some("cargo build --package=lib-a --locked")
    );
    assert_eq!(
        command("lib-a-test-util#build"),
        Some("cargo build --package=lib-a-test-util --locked")
    );
}

#[test]
fn test_rustup_selection_reaches_strict_and_loose_execution() {
    let toolchain = active_rustup_toolchain().expect("test toolchain is managed by rustup");
    let rustup_home = rustup_home().expect("rustup home is available");
    let rustup_home = rustup_home.to_string_lossy().into_owned();
    let toolchain_literal = serde_json::to_string(&toolchain).unwrap();
    let home_literal = serde_json::to_string(&rustup_home).unwrap();

    for env_mode in ["strict", "loose"] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        let manifest = tempdir.path().join("crates/app/Cargo.toml");
        let contents = fs::read_to_string(&manifest).unwrap();
        fs::write(
            manifest,
            contents.replacen("[package]", "[package]\nbuild = \"build.rs\"", 1),
        )
        .unwrap();
        fs::write(
            tempdir.path().join("crates/app/build.rs"),
            format!(
                "fn main() {{\n    assert_eq!(std::env::var(\"RUSTUP_TOOLCHAIN\").unwrap(), \
                 {toolchain_literal});\n    assert_eq!(std::env::var(\"RUSTUP_HOME\").unwrap(), \
                 {home_literal});\n}}\n"
            ),
        )
        .unwrap();
        let environment = [
            ("RUSTUP_TOOLCHAIN", toolchain.as_str()),
            ("RUSTUP_HOME", rustup_home.as_str()),
        ];
        let output = run_turbo_with_env(
            tempdir.path(),
            &["build", "--filter=app", "--env-mode", env_mode],
            &environment,
        );
        assert!(
            output.status.success(),
            "{env_mode} build failed: {output:?}"
        );
    }
}

#[test]
fn test_cargo_build_executes_caches_and_restores() {
    let tempdir = cargo_tempdir();
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

    // Warm: the app comes from cache, while its library dependency runs uncached.
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("app:build: cache hit"),
        "second app build should be cached: {stdout}"
    );

    // Deleting the deliverable and re-running restores it from cache
    // without executing the app's cargo build.
    fs::remove_file(&bin).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("app:build: cache hit"),
        "restored app build should be cached: {stdout}"
    );
    assert!(bin.exists(), "deliverable must be restored from cache");
}

#[test]
fn test_cargo_debug_and_release_caches_are_isolated_both_directions() {
    assert_isolated_restoration(
        &[],
        &["target", "debug"],
        &["--release"],
        &["target", "release"],
    );
    assert_isolated_restoration(
        &["--release"],
        &["target", "release"],
        &[],
        &["target", "debug"],
    );
}

#[test]
fn test_cargo_cli_target_overrides_environment_target() {
    let host = rustc_host_target();
    let lower_target = alternate_host_target(&host);
    let target_arg = format!("--target={host}");
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let artifact = cargo_binary(tempdir.path(), &["target", &host, "debug"]);
    let environment = [("CARGO_BUILD_TARGET", lower_target)];

    let output = run_cargo_build(tempdir.path(), &[&target_arg], &environment);
    assert_command_success(&output, "CLI target precedence build");
    fs::remove_file(&artifact).unwrap();
    let output = run_cargo_build(tempdir.path(), &[&target_arg], &environment);
    assert_command_success(&output, "CLI target precedence restore");
    assert!(String::from_utf8_lossy(&output.stdout).contains("app:build: cache hit"));
    assert!(artifact.exists(), "CLI target did not override environment");
}

#[test]
fn test_cargo_repository_config_target_directory_restores_exactly() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let default = cargo_binary(tempdir.path(), &["target", "debug"]);
    let output = run_cargo_build(tempdir.path(), &[], &[]);
    assert_command_success(&output, "default target-directory build");

    let cargo_config = tempdir.path().join(".cargo");
    fs::create_dir_all(&cargo_config).unwrap();
    fs::write(
        cargo_config.join("config.toml"),
        "[build]\ntarget-dir = \"configured-target\"\n",
    )
    .unwrap();
    let configured = cargo_binary(tempdir.path(), &["configured-target", "debug"]);
    let output = run_cargo_build(tempdir.path(), &[], &[]);
    assert_command_success(&output, "repository target-directory build");
    fs::remove_file(&default).unwrap();
    fs::remove_file(&configured).unwrap();

    let output = run_cargo_build(tempdir.path(), &[], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_command_success(&output, "cache restore");
    assert!(
        stdout.contains("app:build: cache hit"),
        "expected cache hit: {stdout}"
    );
    assert!(configured.exists());
    assert!(!default.exists());
}

#[test]
fn test_cargo_symlink_target_directory_escape_is_uncached() {
    let fixture = cargo_tempdir();
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    setup_cargo_monorepo(&repo);
    configure_build_without_outputs(&repo);
    let outside = fixture.path().join("outside-target");
    fs::create_dir_all(&outside).unwrap();
    create_directory_link(&outside, &repo.join("escape"));
    let artifact = cargo_binary(&outside, &["build", "debug"]);

    for _ in 0..2 {
        let output = run_cargo_build(&repo, &[], &[("CARGO_TARGET_DIR", "escape/build")]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_command_success(&output, "escaping target-directory build");
        assert!(stdout.contains("cache bypass"), "expected bypass: {stdout}");
        assert!(artifact.exists());
    }
}

#[test]
fn test_cargo_command_override_preserves_native_task_contract() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalCargoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "app#build": {
      "command": [
        "node",
        "-e",
        "require('fs').writeFileSync('custom-output.txt', process.env.OVERRIDE_ENV)"
      ],
      "inputs": ["$TURBO_DEFAULT$", "custom-input.txt"],
      "outputs": ["custom-output.txt"],
      "env": ["OVERRIDE_ENV"]
    }
  }
}"#,
    )
    .unwrap();
    fs::write(tempdir.path().join("crates/app/custom-input.txt"), "input").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--dry-run=json"],
    );
    assert!(output.status.success(), "dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let build = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#build"))
        .expect("app#build in graph");
    let definition = &build["resolvedTaskDefinition"];
    let inputs = definition["inputs"].as_array().expect("resolved inputs");
    assert!(
        inputs.iter().any(|input| input == "../../Cargo.toml"),
        "override should preserve the native Cargo workspace inputs: {inputs:?}"
    );
    assert!(
        inputs.iter().any(|input| input == "../../crates/lib-a/**"),
        "override should preserve native dependency inputs: {inputs:?}"
    );
    assert!(
        inputs.iter().any(|input| input == "custom-input.txt"),
        "override should append explicitly configured inputs: {inputs:?}"
    );
    let outputs = definition["outputs"].as_array().expect("resolved outputs");
    assert!(
        outputs.iter().any(|output| output == "custom-output.txt"),
        "override should preserve explicitly configured outputs: {outputs:?}"
    );
    let output_name = if cfg!(windows) { "app.exe" } else { "app" };
    let cargo_output = format!("../../target/debug/{output_name}");
    assert!(
        outputs.iter().any(|output| output == &cargo_output),
        "override should preserve native Cargo outputs: {outputs:?}"
    );
    let env = definition["env"].as_array().expect("resolved environment");
    assert!(env.iter().any(|value| value == "OVERRIDE_ENV"));
    assert!(
        env.iter().any(|value| value == "RUSTFLAGS"),
        "override should preserve native Cargo hash environment: {env:?}"
    );
    assert_eq!(definition["cache"], true);

    // A stale Cargo deliverable present on the override's cache miss becomes
    // part of the task's native output contract.
    let bin = tempdir
        .path()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) { "app.exe" } else { "app" });
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    fs::write(&bin, "stale cargo deliverable").unwrap();

    let run = || {
        run_turbo_with_env(
            tempdir.path(),
            &["run", "build", "--filter=app", "--log-order", "grouped"],
            &[("OVERRIDE_ENV", "configured")],
        )
    };
    let output = run();
    assert!(output.status.success(), "override failed: {output:?}");
    let custom_output = tempdir.path().join("crates/app/custom-output.txt");
    assert_eq!(fs::read_to_string(&custom_output).unwrap(), "configured");

    fs::remove_file(&bin).unwrap();
    fs::remove_file(&custom_output).unwrap();
    let output = run();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "cache restore failed: {output:?}");
    assert!(
        stdout.contains("FULL TURBO"),
        "expected cache hit: {stdout}"
    );
    assert!(custom_output.exists(), "configured output must be restored");
    assert!(bin.exists(), "native Cargo output must be restored");
}

#[test]
fn test_cargo_run_and_dev_default_to_uncached() {
    let tempdir = cargo_tempdir();
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
    let tempdir = cargo_tempdir();
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
fn test_command_override_preserves_native_cache_defaults() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {
    "experimentalCargoWorkspaces": true,
    "experimentalTaskCommand": true
  },
  "tasks": {
    "app#run": { "command": ["node", "-e", "console.log('cargo')"] },
    "js-pkg#run": { "command": ["node", "-e", "console.log('js')"] },
    "app#dev": {
      "command": ["node", "-e", "console.log('explicit')"],
      "cache": false
    }
  }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "run",
            "--filter=app",
            "--filter=js-pkg",
            "--dry-run=json",
        ],
    );
    assert!(output.status.success(), "run dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let tasks = json["tasks"].as_array().expect("tasks array");
    let cargo_run = tasks
        .iter()
        .find(|task| task["taskId"] == "app#run")
        .expect("app#run in graph");
    assert_eq!(
        cargo_run["resolvedTaskDefinition"]["cache"], false,
        "the command override should preserve Cargo's uncached run default"
    );
    let js_run = tasks
        .iter()
        .find(|task| task["taskId"] == "js-pkg#run")
        .expect("js-pkg#run in graph");
    assert_eq!(
        js_run["resolvedTaskDefinition"]["cache"], true,
        "the command override should preserve JavaScript's cache default"
    );

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
    assert_eq!(
        dev["resolvedTaskDefinition"]["cache"], false,
        "explicit cache configuration must win"
    );
}

#[test]
fn test_dependency_crate_change_invalidates_entrypoint() {
    let tempdir = cargo_tempdir();
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
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());

    // Prune requires a lockfile; generate it the way a real repo has one.
    let status = cargo_command(tempdir.path())
        .arg("generate-lockfile")
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
    assert!(out.join("crates/lib-a-test-util/src/lib.rs").exists());
    assert!(out.join("Cargo.toml").exists());
    assert!(out.join("Cargo.lock").exists());
    // The JS package is not in app's closure and must not be copied.
    assert!(!out.join("packages/js-pkg").exists());

    // Members are the explicit kept set.
    let manifest = fs::read_to_string(out.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains(r#"members = ["crates/app", "crates/lib-a", "crates/lib-a-test-util"]"#),
        "explicit members expected, got: {manifest}"
    );

    // The decisive assertion: the pruned workspace builds with the pruned
    // lockfile, strictly.
    let build = cargo_command(&out)
        .args(["build", "--locked", "-p", "app"])
        .output()
        .expect("cargo build runs");
    assert!(
        build.status.success(),
        "pruned workspace must build --locked: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(cargo_binary(&out, &["target", "debug"]))
        .output()
        .expect("pruned binary runs");
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("hello from lib-a"),
        "pruned binary output: {run:?}"
    );
}

/// Task-only edges can cross toolchains in both directions. The package
/// projection is cyclic, but js-pkg#build -> app#build -> js-pkg#prepare is
/// not.
#[test]
fn test_prune_task_aware_cross_toolchain_buildable_output() {
    use serde_json::json;

    for flag in [None, Some(false), Some(true)] {
        let tempdir = cargo_tempdir();
        let dir = tempdir.path();
        setup_cargo_monorepo(dir);
        let mut config = json!({
            "futureFlags": {"experimentalCargoWorkspaces": true},
            "tasks": {
                "build": {"dependsOn": ["^build"]},
                "js-pkg#build": {"dependsOn": ["app#build"]},
                "app#build": {"dependsOn": ["^build", "js-pkg#prepare"]},
                "prepare": {}
            }
        });
        if let Some(flag) = flag {
            config["futureFlags"]["affectedUsingTaskInputs"] = json!(flag);
        }
        fs::write(dir.join("turbo.json"), config.to_string()).unwrap();
        fs::write(
            dir.join("packages/js-pkg/package.json"),
            json!({
                "name": "js-pkg", "version": "1.0.0",
                "scripts": {"build": "echo js-pkg built", "prepare": "echo js-pkg prepared"}
            })
            .to_string(),
        )
        .unwrap();

        // Each scope prunes into its own directory outside the repository.
        // Deleting a pruned output right after building executables in it
        // intermittently fails on Windows (os error 5) while handles are
        // released; the tempdir is cleaned up on drop instead.
        let outputs = cargo_tempdir();

        // There is no package-manifest dependency between js-pkg and app.
        for scope in ["js-pkg", "app"] {
            let out = outputs.path().join(scope);
            let out_dir = out.to_str().expect("tempdir path is UTF-8");
            let output = run_turbo(dir, &["prune", scope, "--out-dir", out_dir]);
            assert_command_success(&output, "cross-toolchain prune");
            let task_aware = flag == Some(true);
            assert_eq!(
                out.join("crates/app/src/main.rs").exists(),
                task_aware || scope == "app"
            );
            assert_eq!(
                out.join("packages/js-pkg/package.json").exists(),
                task_aware || scope == "js-pkg"
            );
            if task_aware {
                assert!(out.join("crates/lib-a/src/lib.rs").exists());
                assert!(out.join("crates/lib-a-test-util/src/lib.rs").exists());
                let build = cargo_command(&out)
                    .args(["build", "--locked", "-p", "app"])
                    .output()
                    .expect("cargo build runs");
                assert_command_success(&build, "task-aware pruned cargo build --locked");
                // Resolve npm.cmd through PATHEXT on Windows.
                let npm = which::which("npm").expect("npm is available on PATH");
                let install = std::process::Command::new(npm)
                    .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
                    .current_dir(&out)
                    .output()
                    .expect("npm ci runs");
                assert_command_success(&install, "task-aware pruned npm ci");
                let build = run_turbo(&out, &["run", "build", "--filter=js-pkg"]);
                assert_command_success(&build, "task-aware pruned cross-toolchain build");
            }
        }
    }
}

#[test]
fn test_filter_hint_when_cargo_disabled() {
    let tempdir = cargo_tempdir();
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
    let tempdir = cargo_tempdir();
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

/// A pure Cargo workspace with no root package.json builds a task graph, with
/// no JavaScript project involved.
#[test]
fn test_pure_cargo_workspace_dry_run_has_no_package_json() {
    let tempdir = cargo_tempdir();
    setup_cargo_pure_workspace(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "--dry-run=json"]);
    assert!(
        output.status.success(),
        "pure Cargo dry-run failed: {output:?}"
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    let tasks = json["tasks"].as_array().expect("tasks array");
    let task =
        |id: &str| -> Option<&serde_json::Value> { tasks.iter().find(|t| t["taskId"] == id) };

    // The bin crate is an entrypoint: it executes a real cargo command.
    let app_build = task("app#build").expect("app#build in graph");
    assert_eq!(app_build["command"], "cargo build --package=app --locked");
    let app_directory = Path::new("crates").join("app");
    assert_eq!(
        app_build["directory"].as_str().map(Path::new),
        Some(app_directory.as_path())
    );
    let app_log = app_directory.join(".turbo").join("turbo-build.log");
    assert_eq!(
        app_build["logFile"].as_str().map(Path::new),
        Some(app_log.as_path())
    );
    assert_eq!(
        json["packages"],
        serde_json::json!(["acme", "app", "lib-a"])
    );
    // Entrypoint selection must not discard the configured ^build dependency.
    assert!(task("lib-a#build").is_some());
    assert_eq!(
        app_build["dependencies"],
        serde_json::json!(["lib-a#build"])
    );

    // The entrypoint's hash still covers its dependency crate's sources even
    // though there is no JavaScript global hash contribution.
    let inputs: Vec<&str> = app_build["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(
        inputs.iter().any(|i| i.contains("crates/lib-a")),
        "dependency crate sources must be inputs, got {inputs:?}"
    );

    // The fixture never had a package.json and turbo must not synthesize one.
    assert!(
        !tempdir.path().join("package.json").exists(),
        "turbo must not create a package.json for a pure Cargo workspace"
    );
    assert!(
        !tempdir.path().join("package-lock.json").exists(),
        "turbo must not synthesize an npm lockfile"
    );
}

#[test]
fn test_pure_cargo_workspace_rejects_malformed_package_json() {
    let tempdir = cargo_tempdir();
    setup_cargo_pure_workspace(tempdir.path());
    fs::write(tempdir.path().join("package.json"), "{").unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--dry-run=json"]);
    let combined = common::combined_output(&output);
    assert!(
        !output.status.success(),
        "malformed package.json must not be treated as absent: {combined}"
    );
    assert!(
        combined.contains("Unable to parse package.json"),
        "expected package.json parse diagnostic, got: {combined}"
    );
}

/// A filtered `turbo run` in a pure Cargo workspace executes cargo, caches
/// the result, and restores it — all without a package.json.
#[test]
fn test_pure_cargo_workspace_filtered_execution() {
    let tempdir = cargo_tempdir();
    setup_cargo_pure_workspace(tempdir.path());

    // Cold: executes cargo and produces the binary.
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--log-order", "grouped"],
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

    // Warm: the app comes from cache, while its library dependency runs uncached.
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("app:build: cache hit"),
        "second app build should be cached: {stdout}"
    );

    // Deleting the deliverable and re-running restores it from cache.
    fs::remove_file(&bin).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("app:build: cache hit"),
        "restored app build should be cached: {stdout}"
    );
    assert!(bin.exists(), "deliverable must be restored from cache");

    assert!(
        !tempdir.path().join("package.json").exists(),
        "turbo must not create a package.json during execution"
    );
}

#[test]
fn test_unfiltered_cargo_verification_runs_once_at_workspace_scope() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let turbo_json_path = tempdir.path().join("turbo.json");
    let mut turbo_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&turbo_json_path).unwrap()).unwrap();
    turbo_json["futureFlags"]["strictTaskEntrypointSelection"] = true.into();
    fs::write(
        turbo_json_path,
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "test", "--dry-run=json"]);
    assert!(output.status.success(), "dry run failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = json["tasks"].as_array().unwrap();
    let rust_tests: Vec<_> = tasks
        .iter()
        .filter(|task| {
            task["taskId"]
                .as_str()
                .is_some_and(|task_id| task_id.ends_with("#test"))
                && task["command"]
                    .as_str()
                    .is_some_and(|command| command.starts_with("cargo test"))
        })
        .collect();
    assert_eq!(
        rust_tests.len(),
        1,
        "expected one Cargo test: {rust_tests:?}"
    );
    assert_eq!(rust_tests[0]["taskId"], "acme#test");
    assert_eq!(rust_tests[0]["command"], "cargo test --workspace --locked");
}

#[test]
fn test_cargo_library_test_can_be_filtered() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "test",
            "--filter=lib-a",
            "--force",
            "--log-order=grouped",
        ],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "filtered test failed: {combined}");
    assert!(
        combined.contains("lib-a:test"),
        "expected the library test task to execute: {combined}"
    );
    assert!(
        !combined.contains("No tasks were executed"),
        "filtered library tests must not be a no-op: {combined}"
    );
}

#[test]
fn test_cargo_format_formats_selected_crate() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("crates/lib-a/src/lib.rs"),
        "pub fn greeting()->&'static str{\"hello from lib-a\"}\n",
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["format", "--filter=lib-a"]);
    assert_command_success(&output, "cargo format");
    assert_eq!(
        fs::read_to_string(tempdir.path().join("crates/lib-a/src/lib.rs")).unwrap(),
        "pub fn greeting() -> &'static str {\n    \"hello from lib-a\"\n}\n"
    );
}

#[test]
fn test_implicit_cargo_tasks_are_package_aware_and_configurable() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": { "build": { "cache": false } }
}"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--dry-run=json"],
    );
    assert!(
        output.status.success(),
        "configured build failed: {output:?}"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let build = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#build"))
        .expect("app#build in graph");
    assert_eq!(build["command"], "cargo build --package=app --locked");
    assert_eq!(build["resolvedTaskDefinition"]["cache"], false);

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=js-pkg", "--dry-run=json"],
    );
    assert!(
        output.status.success(),
        "filtered JS build failed: {output:?}"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["tasks"].as_array().is_some_and(Vec::is_empty));

    let output = run_turbo(tempdir.path(), &["run", "biuld", "--filter=app"]);
    assert!(!output.status.success(), "unknown task must fail");
}

#[test]
fn test_query_package_config_can_disable_implicit_cargo_task() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("crates/app/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": { "build": { "extends": false } }
}"#,
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "{ package(name: \"app\") { tasks { items { name } } } }",
        ],
    );
    assert_command_success(&output, "query package-specific Cargo override");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let names = json["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("app task list");
    assert!(!names.iter().any(|task| task["name"] == "build"));
    assert!(names.iter().any(|task| task["name"] == "run"));
}

#[test]
fn test_ls_and_query_show_implicit_cargo_task_commands() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "app", "--output", "json"]);
    assert!(output.status.success(), "ls failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    insta::assert_json_snapshot!("cargo_native_tasks_ls", json["packages"][0]["tasks"]);

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { package(name: \"app\") { tasks { items { name script command } } } }",
        ],
    );
    assert!(output.status.success(), "query failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    insta::assert_json_snapshot!("cargo_native_tasks_query", json["data"]["package"]["tasks"]);

    // Retain a binary smoke for the aggregate's task registration and CLI
    // projection; crate contracts inject task IDs and resolved definitions.
    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "{ package(name: \"acme\") { tasks { items { name command } } } }",
        ],
    );
    assert_command_success(&output, "aggregate query");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = json["data"]["package"]["tasks"]["items"]
        .as_array()
        .expect("aggregate task list");
    assert!(tasks.iter().any(|task| {
        task["name"] == "test" && task["command"] == "cargo test --workspace --locked"
    }));

    let output = run_turbo(
        tempdir.path(),
        &["run", "test", "--filter=acme", "--dry-run=json"],
    );
    assert_command_success(&output, "aggregate dry-run");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task = json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "acme#test"))
        .expect("aggregate task in dry-run");
    assert_eq!(task["command"], "cargo test --workspace --locked");
    assert_eq!(task["directory"], "");
    let expected_log = Path::new(".turbo").join("turbo-test-acme-c7aba2810dce6e39.log");
    assert_eq!(
        task["logFile"].as_str().map(Path::new),
        Some(expected_log.as_path())
    );
}

#[test]
fn test_query_affected_packages_task_inputs_cross_toolchain() {
    use serde_json::json;

    for flag in [None, Some(false), Some(true)] {
        let tempdir = cargo_tempdir();
        let dir = tempdir.path();
        setup_cargo_monorepo(dir);
        // js-pkg has no manifest dependency on app, only an explicit task edge.
        // app's unchanged Cargo prerequisites must not become affected owners.
        let mut config = json!({
            "futureFlags": {"experimentalCargoWorkspaces": true},
            "tasks": {
                "build": {"dependsOn": ["^build"]},
                "js-pkg#build": {"dependsOn": ["app#build"]}
            }
        });
        if let Some(flag) = flag {
            config["futureFlags"]["affectedUsingTaskInputs"] = json!(flag);
        }
        fs::write(dir.join("turbo.json"), config.to_string()).unwrap();
        let gitignore = fs::read_to_string(dir.join(".gitignore")).unwrap();
        fs::write(
            dir.join(".gitignore"),
            format!("{gitignore}\n.test-home/\n"),
        )
        .unwrap();
        let commit = std::process::Command::new("git")
            .args([
                "commit",
                "-am",
                "Configure cross-toolchain query",
                "--quiet",
            ])
            .current_dir(dir)
            .output()
            .unwrap();
        assert_command_success(&commit, "commit query configuration before source change");
        fs::write(
            dir.join("crates/app/src/main.rs"),
            "fn main() { println!(\"changed\"); }\n",
        )
        .unwrap();
        let diff = std::process::Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert_command_success(&diff, "check source-only query diff");
        assert_eq!(
            String::from_utf8_lossy(&diff.stdout).trim(),
            "crates/app/src/main.rs"
        );

        let output = run_turbo(
            dir,
            &[
                "query",
                r#"{ affectedPackages(base: "HEAD") { length items { name } } }"#,
            ],
        );
        assert_command_success(&output, "query cross-toolchain affected packages");
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(result.get("errors").is_none(), "{result}");
        // The Cargo workspace container is affected by its member's change.
        let mut expected = vec![json!({"name": "acme"}), json!({"name": "app"})];
        if flag == Some(true) {
            expected.push(json!({"name": "js-pkg"}));
        }
        // Exact membership also excludes lib-a, lib-a-test-util, and the JS root.
        assert_eq!(
            result["data"]["affectedPackages"],
            json!({"length": expected.len(), "items": expected}),
            "affectedUsingTaskInputs={flag:?}"
        );
    }
}

#[test]
fn test_affected_includes_cargo_dev_dependency_cycles() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("crates/lib-a-test-util/src/lib.rs"),
        "pub fn expected_greeting() -> &'static str { lib_a::greeting() }\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"test\"]) { items { name package { name } } } }",
        ],
    );
    assert!(output.status.success(), "query failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(
        tasks
            .iter()
            .any(|task| task["name"] == "test" && task["package"]["name"] == "lib-a"),
        "lib-a#test must be affected by its cycle-closing dev dependency: {tasks:?}"
    );
}

#[test]
fn test_affected_tasks_include_implicit_cargo_commands() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": { "experimentalCargoWorkspaces": true },
  "tasks": {}
}"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("crates/app/src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "query",
            "query { affectedTasks(tasks: [\"build\", \"lint\"]) { items { name package { name } \
             } } }",
        ],
    );
    assert!(output.status.success(), "query failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let tasks = json["data"]["affectedTasks"]["items"].as_array().unwrap();
    assert!(
        tasks
            .iter()
            .any(|task| task["name"] == "build" && task["package"]["name"] == "app")
    );
    assert!(
        tasks
            .iter()
            .any(|task| task["name"] == "lint" && task["package"]["name"] == "app")
    );
}
