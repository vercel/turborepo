#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::path::Path;

use common::setup;

fn run_turbo_from(dir: &Path, args: &[&str]) -> std::process::Output {
    let config_dir = tempfile::tempdir().unwrap();
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").unwrap();
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(dir);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().unwrap()
}

// --- has-workspaces-dot-prefix.t ---

#[test]
fn test_has_workspaces_dot_prefix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "inference/has_workspaces_dot_prefix",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // From apps/web: should detect monorepo
    let output = run_turbo_from(&tempdir.path().join("apps/web"), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("web:build:"),
        "should run as monorepo with task prefix"
    );
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter by package name
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=ui"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ui:build:"));
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter with "./" prefix path
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=./packages/ui"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ui:build:"));
    assert!(stdout.contains("1 successful, 1 total"));

    // Filter with "./" prefix for apps/web
    let output = run_turbo_from(tempdir.path(), &["run", "build", "--filter=./apps/web"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("web:build:"));
    assert!(stdout.contains("1 successful, 1 total"));
}
