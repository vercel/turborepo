mod common;

use std::path::Path;

use common::{mock_turbo_config, turbo_command};

fn run_logout(config_dir: &Path) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = turbo_command(tempdir.path());
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir).arg("logout");
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

    let output1 = run_logout(config_dir.path());
    assert!(output1.status.success());

    let output2 = run_logout(config_dir.path());
    assert!(output2.status.success());
    let stdout = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout.contains("Logged out"),
        "expected 'Logged out' even when already logged out, got: {stdout}"
    );
}
