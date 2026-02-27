mod common;

use common::mock_telemetry_config;

fn run_telemetry(config_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir)
        .env("DO_NOT_TRACK", "0") // allow telemetry commands to function
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path());
    for arg in args {
        cmd.arg(arg);
    }
    cmd.output().expect("failed to execute turbo")
}

#[test]
fn test_telemetry_status() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_telemetry_config(config_dir.path());

    let output = run_telemetry(config_dir.path(), &["telemetry", "status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Status: Enabled"),
        "expected telemetry enabled, got: {stdout}"
    );
}

#[test]
fn test_telemetry_no_subcommand() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_telemetry_config(config_dir.path());

    let output = run_telemetry(config_dir.path(), &["telemetry"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Status: Enabled"),
        "expected telemetry enabled, got: {stdout}"
    );
}

#[test]
fn test_telemetry_disable() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_telemetry_config(config_dir.path());

    let output = run_telemetry(config_dir.path(), &["telemetry", "disable"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Status: Disabled"),
        "expected telemetry disabled, got: {stdout}"
    );
}

#[test]
fn test_telemetry_enable() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_telemetry_config(config_dir.path());

    // Disable first
    run_telemetry(config_dir.path(), &["telemetry", "disable"]);

    // Re-enable
    let output = run_telemetry(config_dir.path(), &["telemetry", "enable"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Status: Enabled"),
        "expected telemetry re-enabled, got: {stdout}"
    );
}
