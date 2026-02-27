mod common;

use std::{fs, path::Path};

use common::{run_turbo, setup, turbo_output_filters};

fn version_txt() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../version.txt");
    let contents = fs::read_to_string(&path).expect("failed to read version.txt");
    contents
        .lines()
        .next()
        .expect("version.txt is empty")
        .to_string()
}

#[test]
fn test_version_flag_matches_version_txt() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["--version"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.trim();
    assert_eq!(version, version_txt());
}

#[test]
fn test_short_v_flag_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["-v"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("No command specified"),
        "expected 'No command specified' in stderr, got: {stderr}"
    );
}

#[test]
fn test_login_test_run() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["login", "--__test-run"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("Login test run successful"),
        "expected 'Login test run successful' in stdout, got: {stdout}"
    );
}

#[test]
fn test_unlink_test_run() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["unlink", "--__test-run"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("Unlink test run successful"),
        "expected 'Unlink test run successful' in stdout, got: {stdout}"
    );
}

#[test]
fn test_bad_flag_top_level() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["--bad-flag"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("bad_flag_top_level", stderr.to_string());
    });
}

#[test]
fn test_bad_flag_implied_run() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = run_turbo(tempdir.path(), &["build", "--bad-flag"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("bad_flag_implied_run", stderr.to_string());
    });
}

#[test]
fn test_conflicting_daemon_flags() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    let output = run_turbo(tempdir.path(), &["run", "build", "--daemon", "--no-daemon"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the argument '--daemon' cannot be used with '--no-daemon'"),
        "expected conflict error in stderr, got: {stderr}"
    );
}
