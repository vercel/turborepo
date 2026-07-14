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

fn cargo_build_hash(dir: &Path, env: &[(&str, &str)]) -> String {
    let output = run_turbo_with_env(dir, &["build", "--filter=app", "--dry-run=json"], env);
    assert!(output.status.success(), "dry-run failed: {output:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("dry-run emits JSON");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#build"))
        .and_then(|task| task["hash"].as_str())
        .expect("app#build has a hash")
        .to_string()
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

fn cargo_build_definition(
    dir: &Path,
    cargo_args: &[&str],
    env: &[(&str, &str)],
) -> serde_json::Value {
    let mut args = vec!["build", "--filter=app", "--dry-run=json"];
    if !cargo_args.is_empty() {
        args.push("--");
        args.extend_from_slice(cargo_args);
    }
    let output = run_turbo_with_env(dir, &args, env);
    assert!(output.status.success(), "dry-run failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("dry-run JSON");
    json["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|task| task["taskId"] == "app#build"))
        .expect("app#build in graph")
        .clone()
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
    assert!(
        stdout.contains("FULL TURBO"),
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
fn test_cargo_semantic_environment_changes_task_hash() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());

    let baseline = cargo_build_hash(tempdir.path(), &[]);
    for (name, value) in [
        ("CARGO_ENCODED_RUSTFLAGS", "--cfg\x1fturbo_env_hash_test"),
        ("RUSTDOCFLAGS", "--cfg turbo_env_hash_test"),
        ("CARGO_PROFILE_DEV_LTO", "true"),
        ("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER", "clang"),
        ("CC_aarch64_unknown_linux_gnu", "clang"),
        ("TARGET_CFLAGS", "-DTURBO_ENV_HASH_TEST"),
        ("CROSS_COMPILE", "aarch64-linux-gnu-"),
        ("WASI_SYSROOT", "/opt/wasi-sysroot"),
        ("WASM_MUSL_SYSROOT", "/opt/wasm-musl-sysroot"),
    ] {
        let hash = cargo_build_hash(tempdir.path(), &[(name, value)]);
        assert_ne!(hash, baseline, "{name} must participate in the task hash");
    }

    let network_only = cargo_build_hash(tempdir.path(), &[("CARGO_HTTP_TIMEOUT", "120")]);
    assert_eq!(
        network_only, baseline,
        "Cargo network settings must not invalidate build outputs"
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
        let task = cargo_build_definition(tempdir.path(), &[], &environment);
        let declared = task["resolvedTaskDefinition"]["env"]
            .as_array()
            .expect("declared task environment");
        for variable in ["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"] {
            assert!(declared.iter().any(|value| value == variable));
        }

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
fn test_cargo_workspace_requires_lockfile() {
    let tempdir = cargo_tempdir();
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
    let tempdir = cargo_tempdir();
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
    let tempdir = cargo_tempdir();
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
    let status = cargo_command(tempdir.path())
        .arg("generate-lockfile")
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
            combined.contains("local")
                && combined.contains("workspace")
                && combined.contains("member")
                && combined.contains("hashed")
                && combined.contains("pruned"),
            "expected actionable path dependency error: {combined}"
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
fn test_custom_profile_outputs_are_exact_and_restore() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let manifest = tempdir.path().join("Cargo.toml");
    let contents = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        format!("{contents}\n[profile.ci]\ninherits = \"dev\"\n"),
    )
    .unwrap();
    let debug = cargo_binary(tempdir.path(), &["target", "debug"]);
    let custom = cargo_binary(tempdir.path(), &["target", "ci"]);

    assert!(run_cargo_build(tempdir.path(), &[], &[]).status.success());
    assert!(
        run_cargo_build(tempdir.path(), &["--profile=ci"], &[])
            .status
            .success()
    );
    fs::remove_file(&debug).unwrap();
    fs::remove_file(&custom).unwrap();
    let output = run_cargo_build(tempdir.path(), &["--profile=ci"], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_command_success(&output, "cache restore");
    assert!(
        stdout.contains("FULL TURBO"),
        "expected cache hit: {stdout}"
    );
    assert!(custom.exists());
    assert!(!debug.exists());
}

#[test]
fn test_cargo_test_and_bench_profile_directories_restore_exactly() {
    assert_isolated_restoration(
        &["--release"],
        &["target", "release"],
        &["--profile=test"],
        &["target", "debug"],
    );
    assert_isolated_restoration(
        &[],
        &["target", "debug"],
        &["--profile=bench"],
        &["target", "release"],
    );
}

#[test]
fn test_cargo_explicit_and_environment_host_targets_restore_exactly() {
    let host = rustc_host_target();
    let target_arg = format!("--target={host}");
    assert_isolated_restoration(
        &[],
        &["target", "debug"],
        &[&target_arg],
        &["target", &host, "debug"],
    );
    assert_isolated_restoration_with_env(
        &[],
        &["target", "debug"],
        &[],
        &["target", &host, "debug"],
        &[],
        &[("CARGO_BUILD_TARGET", &host)],
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"));
    assert!(artifact.exists(), "CLI target did not override environment");
}

#[test]
fn test_cargo_argument_and_environment_target_directories_restore_exactly() {
    assert_isolated_restoration(
        &[],
        &["target", "debug"],
        &["--target-dir=argument-target"],
        &["argument-target", "debug"],
    );
    assert_isolated_restoration_with_env(
        &[],
        &["target", "debug"],
        &[],
        &["environment-target", "debug"],
        &[],
        &[("CARGO_TARGET_DIR", "environment-target")],
    );
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
        stdout.contains("FULL TURBO"),
        "expected cache hit: {stdout}"
    );
    assert!(configured.exists());
    assert!(!default.exists());
}

#[test]
fn test_cargo_target_directory_precedence_restores_only_effective_output() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let cargo_config = tempdir.path().join(".cargo");
    fs::create_dir_all(&cargo_config).unwrap();
    fs::write(
        cargo_config.join("config.toml"),
        "[build]\ntarget-dir = \"config-target\"\n",
    )
    .unwrap();
    let config = cargo_binary(tempdir.path(), &["config-target", "debug"]);
    let environment = cargo_binary(tempdir.path(), &["env-target", "debug"]);
    let cli = cargo_binary(tempdir.path(), &["cli-target", "debug"]);

    let output = run_cargo_build(tempdir.path(), &[], &[]);
    assert_command_success(&output, "metadata target-directory build");
    let output = run_cargo_build(tempdir.path(), &[], &[("CARGO_TARGET_DIR", "env-target")]);
    assert_command_success(&output, "environment target-directory build");
    let output = run_cargo_build(
        tempdir.path(),
        &["--target-dir=cli-target"],
        &[("CARGO_TARGET_DIR", "env-target")],
    );
    assert_command_success(&output, "CLI target-directory build");
    assert!(config.exists() && environment.exists() && cli.exists());
    fs::remove_file(&config).unwrap();
    fs::remove_file(&environment).unwrap();
    fs::remove_file(&cli).unwrap();

    let output = run_cargo_build(tempdir.path(), &[], &[("CARGO_TARGET_DIR", "env-target")]);
    assert_command_success(&output, "environment target-directory restore");
    assert!(String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"));
    assert!(environment.exists());
    assert!(!config.exists() && !cli.exists());
    fs::remove_file(&environment).unwrap();

    let output = run_cargo_build(
        tempdir.path(),
        &["--target-dir=cli-target"],
        &[("CARGO_TARGET_DIR", "env-target")],
    );
    assert_command_success(&output, "CLI target-directory restore");
    assert!(String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"));
    assert!(cli.exists());
    assert!(!config.exists() && !environment.exists());
}

#[cfg(unix)]
#[test]
fn test_cargo_symlink_target_directory_escape_is_uncached() {
    let fixture = cargo_tempdir();
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    setup_cargo_monorepo(&repo);
    configure_build_without_outputs(&repo);
    let outside = fixture.path().join("outside-target");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, repo.join("escape")).unwrap();
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
fn test_external_cargo_home_config_is_uncached_in_strict_mode() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    configure_build_without_outputs(tempdir.path());
    let cargo_home = tempdir.path().join("external-cargo-home");
    fs::create_dir_all(&cargo_home).unwrap();
    fs::write(
        cargo_home.join("config.toml"),
        "[build]\ntarget-dir = \"cargo-home-target\"\n",
    )
    .unwrap();
    let cargo_home = cargo_home.to_string_lossy();

    for _ in 0..2 {
        let output = run_cargo_build(tempdir.path(), &[], &[("CARGO_HOME", cargo_home.as_ref())]);
        assert!(output.status.success(), "build failed: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("cache bypass"),
            "external Cargo config must disable implicit caching"
        );
    }
}

#[test]
fn test_untracked_config_respects_only_explicit_cache_authority() {
    for (task_config, expected_cache) in [
        (r#""cache": true"#, true),
        (r#""cache": false"#, false),
        (r#""outputs": ["../../target/*/app"]"#, false),
    ] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        fs::write(
            tempdir.path().join("turbo.json"),
            format!(
                r#"{{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {{ "experimentalCargoWorkspaces": true }},
  "tasks": {{ "app#build": {{ {task_config} }} }}
}}"#
            ),
        )
        .unwrap();
        let cargo_home = tempdir.path().join("external-cargo-home");
        fs::create_dir_all(&cargo_home).unwrap();
        fs::write(cargo_home.join("config.toml"), "[net]\nretry = 2\n").unwrap();
        let cargo_home = cargo_home.to_string_lossy();

        let task =
            cargo_build_definition(tempdir.path(), &[], &[("CARGO_HOME", cargo_home.as_ref())]);
        assert_eq!(task["resolvedTaskDefinition"]["cache"], expected_cache);
        for run in 0..2 {
            let output =
                run_cargo_build(tempdir.path(), &[], &[("CARGO_HOME", cargo_home.as_ref())]);
            assert!(output.status.success(), "build failed: {output:?}");
            let stdout = String::from_utf8_lossy(&output.stdout);
            if expected_cache && run == 1 {
                assert!(
                    stdout.contains("FULL TURBO"),
                    "expected cache hit: {stdout}"
                );
            } else if !expected_cache {
                assert!(stdout.contains("cache bypass"), "expected bypass: {stdout}");
            }
        }
    }
}

#[test]
fn test_repository_config_layout_controls_are_uncached() {
    let host = rustc_host_target();
    for config in [
        format!("[build]\ntarget = \"{host}\"\n"),
        "[build]\nartifact-dir = \"artifact-copy\"\n".to_string(),
        "[profile.ci]\ninherits = \"dev\"\ndir-name = \"profile-output\"\n".to_string(),
    ] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        configure_build_without_outputs(tempdir.path());
        fs::create_dir_all(tempdir.path().join(".cargo")).unwrap();
        fs::write(tempdir.path().join(".cargo/config.toml"), config).unwrap();
        let task = cargo_build_definition(tempdir.path(), &[], &[]);
        assert_eq!(task["resolvedTaskDefinition"]["cache"], false);
    }

    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    configure_build_without_outputs(tempdir.path());
    fs::create_dir_all(tempdir.path().join(".cargo")).unwrap();
    fs::write(
        tempdir.path().join(".cargo/config.toml"),
        format!("[build]\ntarget = \"{host}\"\n"),
    )
    .unwrap();
    let artifact = cargo_binary(tempdir.path(), &["target", &host, "debug"]);
    for _ in 0..2 {
        let output = run_cargo_build(tempdir.path(), &[], &[]);
        assert!(output.status.success(), "build failed: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("cache bypass"));
        assert!(artifact.exists());
    }
}

#[test]
fn test_manifest_layout_controls_are_uncached() {
    for manifest_control in ["per-package-target", "different-binary-name"] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        configure_build_without_outputs(tempdir.path());
        fs::write(
            tempdir.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"nightly-2026-04-10\"\n",
        )
        .unwrap();
        let manifest = tempdir.path().join("crates/app/Cargo.toml");
        let contents = fs::read_to_string(&manifest).unwrap();
        let contents = if manifest_control == "per-package-target" {
            let host = rustc_host_target();
            format!(
                "cargo-features = [\"per-package-target\"]\n{}",
                contents.replacen(
                    "[package]",
                    &format!("[package]\ndefault-target = \"{host}\""),
                    1,
                )
            )
        } else {
            format!(
                "cargo-features = [\"different-binary-name\"]\n{contents}\n[[bin]]\nname = \
                 \"app\"\npath = \"src/main.rs\"\nfilename = \"renamed-app\"\n"
            )
        };
        fs::write(manifest, contents).unwrap();
        let task = cargo_build_definition(tempdir.path(), &[], &[]);
        assert_eq!(task["resolvedTaskDefinition"]["cache"], false);
    }
}

#[test]
fn test_compiler_and_layout_environment_controls_are_uncached() {
    for (name, value) in [
        ("RUSTC", "rustc"),
        ("CARGO_BUILD_RUSTC", "rustc"),
        ("CARGO_BUILD_TARGET_DIR", "other-target"),
        ("CARGO_BUILD_ARTIFACT_DIR", "artifact-copy"),
        ("CARGO_PROFILE_CI_DIR_NAME", "profile-output"),
    ] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        configure_build_without_outputs(tempdir.path());
        let task = cargo_build_definition(tempdir.path(), &[], &[(name, value)]);
        assert_eq!(task["resolvedTaskDefinition"]["cache"], false, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn test_escaping_repository_config_is_untracked() {
    let fixture = cargo_tempdir();
    let repo = fixture.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    setup_cargo_monorepo(&repo);
    configure_build_without_outputs(&repo);
    let outside_config = fixture.path().join("outside-config.toml");
    fs::write(&outside_config, "[net]\nretry = 2\n").unwrap();
    fs::create_dir_all(repo.join(".cargo")).unwrap();
    std::os::unix::fs::symlink(&outside_config, repo.join(".cargo/config.toml")).unwrap();

    let before = cargo_build_definition(&repo, &[], &[]);
    assert_eq!(before["resolvedTaskDefinition"]["cache"], false);
    let inputs = before["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .expect("resolved inputs");
    assert!(
        inputs.iter().all(|input| !input
            .as_str()
            .is_some_and(|input| input.contains(".cargo/config"))),
        "symlinked config must not be emitted as a trusted input: {inputs:?}"
    );

    fs::write(&outside_config, "[net]\nretry = 3\n").unwrap();
    let after = cargo_build_definition(&repo, &[], &[]);
    assert_eq!(after["resolvedTaskDefinition"]["cache"], false);
    assert_eq!(before["hash"], after["hash"]);
}

#[cfg(unix)]
#[test]
fn test_internal_repository_config_symlink_is_untracked() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    configure_build_without_outputs(tempdir.path());
    let config_dir = tempdir.path().join("config");
    fs::create_dir_all(&config_dir).unwrap();
    let target = config_dir.join("cargo.toml");
    fs::write(&target, "[net]\nretry = 2\n").unwrap();
    fs::create_dir_all(tempdir.path().join(".cargo")).unwrap();
    std::os::unix::fs::symlink(&target, tempdir.path().join(".cargo/config.toml")).unwrap();

    let before = cargo_build_definition(tempdir.path(), &[], &[]);
    assert_eq!(before["resolvedTaskDefinition"]["cache"], false);
    let inputs = before["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .expect("resolved inputs");
    assert!(
        inputs.iter().all(|input| !input
            .as_str()
            .is_some_and(|input| input.contains(".cargo/config"))),
        "symlinked config must not be emitted as a trusted input: {inputs:?}"
    );

    fs::write(target, "[net]\nretry = 3\n").unwrap();
    let after = cargo_build_definition(tempdir.path(), &[], &[]);
    assert_eq!(before["hash"], after["hash"]);
}

#[cfg(unix)]
#[test]
fn test_config_beneath_symlinked_cargo_directory_is_untracked() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    configure_build_without_outputs(tempdir.path());
    let cargo_target = tempdir.path().join("cargo-config");
    fs::create_dir_all(&cargo_target).unwrap();
    let config = cargo_target.join("config.toml");
    fs::write(&config, "[net]\nretry = 2\n").unwrap();
    std::os::unix::fs::symlink(&cargo_target, tempdir.path().join(".cargo")).unwrap();

    let before = cargo_build_definition(tempdir.path(), &[], &[]);
    assert_eq!(before["resolvedTaskDefinition"]["cache"], false);
    let inputs = before["resolvedTaskDefinition"]["inputs"]
        .as_array()
        .expect("resolved inputs");
    assert!(
        inputs.iter().all(|input| !input
            .as_str()
            .is_some_and(|input| input.contains(".cargo/config"))),
        "config beneath a symlink must not be emitted as a trusted input: {inputs:?}"
    );

    fs::write(config, "[net]\nretry = 3\n").unwrap();
    let after = cargo_build_definition(tempdir.path(), &[], &[]);
    assert_eq!(after["resolvedTaskDefinition"]["cache"], false);
    assert_eq!(before["hash"], after["hash"]);
}

#[test]
fn test_unavailable_outputs_preserve_explicit_intent() {
    for cache in [true, false] {
        let tempdir = cargo_tempdir();
        setup_cargo_monorepo(tempdir.path());
        fs::write(
            tempdir.path().join("turbo.json"),
            format!(
                r#"{{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {{ "experimentalCargoWorkspaces": true }},
  "tasks": {{ "app#build": {{ "cache": {cache} }} }}
}}"#
            ),
        )
        .unwrap();
        let environment = [("RUSTC", "rustc")];
        let task = cargo_build_definition(tempdir.path(), &[], &environment);
        assert_eq!(task["resolvedTaskDefinition"]["cache"], cache);
        for run in 0..2 {
            let output = run_cargo_build(tempdir.path(), &[], &environment);
            assert!(output.status.success(), "build failed: {output:?}");
            let stdout = String::from_utf8_lossy(&output.stdout);
            if cache && run == 1 {
                assert!(
                    stdout.contains("FULL TURBO"),
                    "expected cache hit: {stdout}"
                );
            } else if !cache {
                assert!(stdout.contains("cache bypass"), "expected bypass: {stdout}");
            }
        }
    }

    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let output_name = if cfg!(windows) { "app.exe" } else { "app" };
    fs::write(
        tempdir.path().join("turbo.json"),
        format!(
            r#"{{
  "$schema": "https://turborepo.dev/schema.json",
  "futureFlags": {{ "experimentalCargoWorkspaces": true }},
  "tasks": {{ "app#build": {{ "outputs": ["../../other-target/debug/{output_name}"] }} }}
}}"#
        ),
    )
    .unwrap();
    let cargo_args = ["--target-dir=other-target"];
    let output = run_cargo_build(tempdir.path(), &cargo_args, &[]);
    assert!(output.status.success(), "build failed: {output:?}");
    let artifact = cargo_binary(tempdir.path(), &["other-target", "debug"]);
    assert!(artifact.exists());
    fs::remove_file(&artifact).unwrap();
    let output = run_cargo_build(tempdir.path(), &cargo_args, &[]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("FULL TURBO"));
    assert!(artifact.exists());
}

#[test]
fn test_cargo_command_override_uses_only_configured_io() {
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
      "inputs": ["Cargo.toml"],
      "outputs": ["custom-output.txt"],
      "env": ["OVERRIDE_ENV"]
    }
  }
}"#,
    )
    .unwrap();

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
    assert_eq!(definition["inputs"], serde_json::json!(["Cargo.toml"]));
    assert_eq!(
        definition["outputs"],
        serde_json::json!(["custom-output.txt"])
    );
    assert_eq!(definition["env"], serde_json::json!(["OVERRIDE_ENV"]));

    // A stale Cargo deliverable present on the override's cache miss must not
    // become one of that arbitrary command's cached outputs.
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
    assert!(!bin.exists(), "Cargo deliverable must not be restored");
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
fn test_command_override_uses_generic_cache_default_across_toolchains() {
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
    for task_id in ["app#run", "js-pkg#run"] {
        let task = tasks
            .iter()
            .find(|task| task["taskId"] == task_id)
            .unwrap_or_else(|| panic!("{task_id} in graph"));
        assert_eq!(
            task["resolvedTaskDefinition"]["cache"], true,
            "{task_id} should use the generic cache default"
        );
    }

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

/// Docker layout: the json layer carries everything needed to resolve
/// dependencies (manifests + lockfile), the full layer carries sources.
#[test]
fn test_prune_docker_layout_for_cargo() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());
    let status = cargo_command(tempdir.path())
        .arg("generate-lockfile")
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

    let full_lock = fs::read(out.join("full/Cargo.lock")).unwrap();
    let json_lock = fs::read(out.join("json/Cargo.lock")).unwrap();
    assert_eq!(full_lock, json_lock, "docker lockfiles must stay in sync");

    let build = cargo_command(&out.join("full"))
        .args(["build", "--locked", "-p", "app"])
        .output()
        .expect("cargo build runs");
    assert!(
        build.status.success(),
        "docker full workspace must build --locked: {}",
        String::from_utf8_lossy(&build.stderr)
    );
}

/// A JS-only target in a mixed repo prunes exactly as before: no crates, no
/// Cargo workspace files.
#[test]
fn test_prune_js_target_unaffected_by_cargo() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "js-pkg"]);
    assert!(output.status.success(), "prune failed: {output:?}");

    let out = tempdir.path().join("out");
    assert!(out.join("packages/js-pkg/package.json").exists());
    assert!(!out.join("crates").exists());
    assert!(!out.join("Cargo.toml").exists());
}

#[test]
fn test_prune_js_docker_target_skips_cargo_finalization() {
    let tempdir = cargo_tempdir();
    setup_cargo_monorepo(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "js-pkg", "--docker"]);
    assert!(output.status.success(), "prune failed: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("pruned Cargo.lock"),
        "Cargo should not finalize a JS-only prune: {stderr}"
    );

    let out = tempdir.path().join("out");
    assert!(out.join("full/packages/js-pkg/package.json").exists());
    assert!(out.join("json/packages/js-pkg/package.json").exists());
    assert!(!out.join("full/crates").exists());
    assert!(!out.join("full/Cargo.toml").exists());
    assert!(!out.join("json/Cargo.lock").exists());
}

/// The synthetic workspace package has no directory of its own and is not
/// a pruneable target.
#[test]
fn test_prune_cargo_workspace_package_rejected() {
    let tempdir = cargo_tempdir();
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

/// A pure Cargo workspace with no root package.json builds a task graph:
/// every crate becomes a package and gets its Cargo-derived command, with no
/// JavaScript project involved.
#[test]
fn test_pure_cargo_workspace_dry_run_has_no_package_json() {
    let tempdir = tempfile::tempdir().unwrap();
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
    // Its dependency crate participates in the graph but is a no-op.
    let lib_build = task("lib-a#build").expect("lib-a#build in graph");
    assert_eq!(lib_build["command"], "<NONEXISTENT>");

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
    let tempdir = tempfile::tempdir().unwrap();
    setup_cargo_pure_workspace(tempdir.path());
    fs::write(tempdir.path().join("package.json"), "{").unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--dry-run=json"]);
    assert!(
        !output.status.success(),
        "malformed package.json must not be treated as absent"
    );
}

/// A filtered `turbo run` in a pure Cargo workspace executes cargo, caches
/// the result, and restores it — all without a package.json.
#[test]
fn test_pure_cargo_workspace_filtered_execution() {
    let tempdir = tempfile::tempdir().unwrap();
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

    // Warm: fully cached.
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "second run should be fully cached: {stdout}"
    );

    // Deleting the deliverable and re-running restores it from cache.
    fs::remove_file(&bin).unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=app", "--log-order", "grouped"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "restore run should be fully cached: {stdout}"
    );
    assert!(bin.exists(), "deliverable must be restored from cache");

    assert!(
        !tempdir.path().join("package.json").exists(),
        "turbo must not create a package.json during execution"
    );
}
