mod common;

use common::{mock_telemetry_config, turbo_command};

fn run_telemetry(config_dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = turbo_command(tempdir.path());
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir)
        .env("DO_NOT_TRACK", "0"); // override: allow telemetry commands to function
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

    run_telemetry(config_dir.path(), &["telemetry", "disable"]);

    let output = run_telemetry(config_dir.path(), &["telemetry", "enable"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Status: Enabled"),
        "expected telemetry re-enabled, got: {stdout}"
    );
}
