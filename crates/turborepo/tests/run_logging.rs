#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, run_turbo_with_env, setup};

#[test]
fn test_github_actions_grouped_logging_smoke() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            "--log-prefix=task",
            "--filter=util",
        ],
        &[("GITHUB_ACTIONS", "1")],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("::group::util:build"));
    assert!(stdout.contains("util:build: cache bypass"));
    assert!(stdout.contains("::endgroup::"));

    let output = run_turbo_with_env(tempdir.path(), &["run", "fail"], &[("GITHUB_ACTIONS", "1")]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("::error::"),
        "stderr should contain ::error:: annotation for GitHub Actions, got: {}",
        &stderr[..stderr.len().min(500)]
    );
}

// --- log-prefix.t ---

#[test]
fn test_log_prefix_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    assert!(stdout.contains("build-app-a"));
    assert!(!stdout.contains("app-a:build:"));

    // Check cached log file doesn't have prefixes
    let log = std::fs::read_to_string(tempdir.path().join("app-a/.turbo/turbo-build.log")).unwrap();
    assert!(log.contains("build-app-a"));
    assert!(!log.contains("app-a:build:"));

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs"));
    assert!(stdout.contains("FULL TURBO"));
    assert!(!stdout.contains("app-a:build:"));
}
