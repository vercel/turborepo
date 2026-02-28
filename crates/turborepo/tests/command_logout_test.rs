mod common;

use std::path::Path;

use common::mock_turbo_config;

fn run_logout(config_dir: &Path) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir)
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path())
        .arg("logout");
    cmd.output().expect("failed to execute turbo")
}

#[test]
fn test_logout_while_logged_in() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_turbo_config(config_dir.path());

    let output = run_logout(config_dir.path());
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Logged out"),
        "expected 'Logged out', got: {stdout}"
    );
}

#[test]
fn test_logout_while_logged_out() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_turbo_config(config_dir.path());

    // First logout removes the token
    let output1 = run_logout(config_dir.path());
    assert!(output1.status.success());

    // Second logout against the same (now token-less) config dir
    let output2 = run_logout(config_dir.path());
    assert!(output2.status.success());
    let stdout = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout.contains("Logged out"),
        "expected 'Logged out' even when already logged out, got: {stdout}"
    );
}
