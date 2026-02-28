mod common;

use std::path::Path;

use common::{set_find_turbo_link, set_find_turbo_version, setup_find_turbo};

fn turbo_bin() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin("turbo")
}

fn run_turbo_vv(cwd: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(turbo_bin());
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(cwd);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn stdout_last_line(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().last().unwrap_or("").trim().to_string()
}

// self.t: When the local turbo IS the running binary, don't reinvoke.
#[test]
fn test_self_invocation_detected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "self");
    set_find_turbo_link(tempdir.path(), &turbo_bin());

    let output = run_turbo_vv(tempdir.path(), &["--version", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Currently running turbo is local turbo"),
        "expected self-detection message, got: {combined}"
    );
}

// hoisted.t: Hoisted node_modules layout. Old version (< 1.8) gets bare args,
// new version (>= 1.8) gets --skip-infer.
#[test]
fn test_hoisted_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "hoisted");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    // The fake turbo echoes its args; last line should be the args without
    // --skip-infer
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "build --filter foo -vv --",
        "old version should not get --skip-infer: {last_line}"
    );
}

#[test]
fn test_hoisted_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "hoisted");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --",
        "new version should get --skip-infer: {last_line}"
    );
}

// linked.t: pnpm symlinked layout
#[test]
fn test_linked_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "linked");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(last_line.trim(), "build --filter foo -vv --");
}

#[test]
fn test_linked_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "linked");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --"
    );
}

// nested.t: Nested node_modules/turbo/node_modules/ layout
#[test]
fn test_nested_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "nested");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(last_line.trim(), "build --filter foo -vv --");
}

#[test]
fn test_nested_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "nested");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --"
    );
}

// unplugged.t: Yarn PnP default unplugged folder
#[test]
fn test_unplugged_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(last_line.trim(), "build --filter foo -vv --");
}

#[test]
fn test_unplugged_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --"
    );
}

// unplugged-moved.t: Yarn PnP with custom unplugged folder via .yarnrc.yml
#[test]
fn test_unplugged_moved_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged_moved");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(last_line.trim(), "build --filter foo -vv --");
}

#[test]
fn test_unplugged_moved_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged_moved");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let output = run_turbo_vv(tempdir.path(), &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --"
    );
}

// unplugged-env-moved.t: Yarn PnP with custom yarnrc filename via
// YARN_RC_FILENAME env var
#[test]
fn test_unplugged_env_moved_old_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged_env_moved");
    set_find_turbo_version(tempdir.path(), "1.0.0");

    let mut cmd = std::process::Command::new(turbo_bin());
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
        .env("DO_NOT_TRACK", "1")
        .env("YARN_RC_FILENAME", ".notyarnrc.yml")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path())
        .args(["build", "--filter", "foo", "-vv"]);
    let output = cmd.output().unwrap();

    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.0.0"),
        "expected local turbo 1.0.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(last_line.trim(), "build --filter foo -vv --");
}

#[test]
fn test_unplugged_env_moved_new_version() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_find_turbo(tempdir.path(), "unplugged_env_moved");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    let mut cmd = std::process::Command::new(turbo_bin());
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0")
        .env("DO_NOT_TRACK", "1")
        .env("YARN_RC_FILENAME", ".notyarnrc.yml")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path())
        .args(["build", "--filter", "foo", "-vv"]);
    let output = cmd.output().unwrap();

    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0, got: {combined}"
    );
    let last_line = stdout_last_line(&output);
    assert_eq!(
        last_line.trim(),
        "--skip-infer build --filter foo -vv --single-package --"
    );
}

// hard-mode.t: --skip-infer, cwd navigation, and finding repo root from nested
// dirs
#[test]
fn test_hard_mode_skip_infer() {
    let tempdir = tempfile::tempdir().unwrap();
    // hard-mode uses the hoisted fixture in a subdirectory
    let subdir = tempdir.path().join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();
    setup_find_turbo(&subdir, "hoisted");

    let output = run_turbo_vv(&subdir, &["--help", "--skip-infer", "-vv"]);
    let combined = combined_output(&output);
    // With --skip-infer, the shim uses the current binary directly
    assert!(
        combined.contains("Global turbo version:"),
        "expected global turbo log, got: {combined}"
    );
    assert!(
        combined.contains("The build system that makes ship happen"),
        "expected help output, got: {combined}"
    );
}

#[test]
fn test_hard_mode_finds_repo_root() {
    let tempdir = tempfile::tempdir().unwrap();
    let subdir = tempdir.path().join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();
    setup_find_turbo(&subdir, "hoisted");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    // Run from node_modules — shim should find the repo root and local turbo
    let nm_dir = subdir.join("node_modules");
    let output = run_turbo_vv(&nm_dir, &["build", "--filter", "foo", "-vv"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0 from node_modules, got: {combined}"
    );
}

#[test]
fn test_hard_mode_cwd_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    let subdir = tempdir.path().join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();
    setup_find_turbo(&subdir, "hoisted");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    // Run from the parent with --cwd pointing to subdir
    let output = run_turbo_vv(
        tempdir.path(),
        &[
            "build",
            "--filter",
            "foo",
            "-vv",
            "--cwd",
            subdir.to_str().unwrap(),
        ],
    );
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0 via --cwd, got: {combined}"
    );
}

#[test]
fn test_hard_mode_cwd_to_node_modules() {
    let tempdir = tempfile::tempdir().unwrap();
    let subdir = tempdir.path().join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();
    setup_find_turbo(&subdir, "hoisted");
    set_find_turbo_version(tempdir.path(), "1.8.0");

    // Run from parent with --cwd pointing to node_modules inside subdir
    let nm_dir = subdir.join("node_modules");
    let output = run_turbo_vv(
        tempdir.path(),
        &[
            "build",
            "--filter",
            "foo",
            "-vv",
            "--cwd",
            nm_dir.to_str().unwrap(),
        ],
    );
    let combined = combined_output(&output);
    assert!(
        combined.contains("Local turbo version: 1.8.0"),
        "expected local turbo 1.8.0 via --cwd to node_modules, got: {combined}"
    );
}
