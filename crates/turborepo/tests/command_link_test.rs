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
fn test_link_test_run() {
    let output = run_turbo_with_config(&["link", "--__test-run"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Link test run successful"),
        "expected link test run success, got: {stdout}"
    );
}

#[test]
fn test_link_test_run_with_yes() {
    let output = run_turbo_with_config(&["link", "--__test-run", "--yes"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Link test run successful"),
        "expected link test run success, got: {stdout}"
    );
}

#[test]
fn test_link_test_run_with_team_flag_warns() {
    let output = run_turbo_with_config(&["link", "--__test-run", "--team=my-team"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("team flag does not set the scope for linking"),
        "expected team flag warning, got: {combined}"
    );
    assert!(
        combined.contains("Link test run successful"),
        "expected link test run success, got: {combined}"
    );
}
