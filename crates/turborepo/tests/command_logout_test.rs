mod common;

use std::{fs, net::TcpListener, path::Path};

use common::{mock_turbo_config, turbo_command};

fn run_logout(config_dir: &Path) -> std::process::Output {
    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = turbo_command(tempdir.path());
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir)
        .args(["logout", "--invalidate=false"]);
    cmd.output().expect("failed to execute turbo")
}

fn turbo_config_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("turborepo").join("config.json")
}

fn closed_api_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind ephemeral port");
    let port = listener
        .local_addr()
        .expect("failed to read listener addr")
        .port();
    drop(listener);

    format!("http://127.0.0.1:{port}")
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

#[test]
fn test_logout_invalidates_by_default() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_turbo_config(config_dir.path());

    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = turbo_command(tempdir.path());
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("TURBO_API", closed_api_url())
        .arg("logout");

    let output = cmd.output().expect("failed to execute turbo");
    assert!(!output.status.success(), "expected logout to fail");

    let config = fs::read_to_string(turbo_config_path(config_dir.path()))
        .expect("expected config to remain on failed invalidate");
    assert!(
        config.contains("normal-user-token"),
        "expected token to remain after failed invalidate, got: {config}"
    );
}

#[test]
fn test_logout_can_skip_invalidation() {
    let config_dir = tempfile::tempdir().unwrap();
    mock_turbo_config(config_dir.path());

    let tempdir = tempfile::tempdir().unwrap();
    let mut cmd = turbo_command(tempdir.path());
    cmd.env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("TURBO_API", closed_api_url())
        .args(["logout", "--invalidate=false"]);

    let output = cmd.output().expect("failed to execute turbo");
    assert!(output.status.success(), "expected logout to succeed");

    let config =
        fs::read_to_string(turbo_config_path(config_dir.path())).expect("expected config file");
    assert_eq!(config, "{}");
}
