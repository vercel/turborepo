mod common;

use common::mock_turbo_config;

fn run_turbo_with_config(args: &[&str]) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    mock_turbo_config(config_dir.path());

    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path());
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
}

#[test]
fn test_logout_while_logged_in() {
    let output = run_turbo_with_config(&["logout"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Logged out"),
        "expected 'Logged out', got: {stdout}"
    );
}

#[test]
fn test_logout_while_logged_out() {
    // First logout
    run_turbo_with_config(&["logout"]);
    // Second logout should also succeed
    let output = run_turbo_with_config(&["logout"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Logged out"),
        "expected 'Logged out', got: {stdout}"
    );
}
