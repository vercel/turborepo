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

    // Default prefix: should show prefixes
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("app-a:build: cache hit, replaying logs"));
    assert!(stdout.contains("app-a:build: build-app-a"));
}

// --- verbosity.t ---

#[test]
fn test_verbosity_flags() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "-v", "--filter=util", "--force"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("util:build: cache bypass, force executing bf1798d3e46e1b48"));
    assert!(stdout.contains("util:build: building"));

    let output = run_turbo(
        tempdir.path(),
        &["build", "-vv", "--filter=util", "--force"],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(combined.contains("[DEBUG]"));
}

// --- no-cache-and-no-output-logs.t ---

#[test]
fn test_no_cache_and_no_output_logs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--cache=local:,remote:",
            "--output-logs=none",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no caches are enabled") || stdout.contains("no caches are enabled"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_run_prelude_stdout_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("Packages in scope"),
        "prelude should list packages in scope on stdout"
    );
    assert!(
        stdout.contains("Running build in"),
        "prelude should show running tasks on stdout"
    );
    assert!(
        stdout.contains("Remote caching"),
        "prelude should show remote cache status on stdout"
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Packages in scope"),
        "prelude should appear on stdout in --dry text mode"
    );
    assert!(
        stdout.contains("Remote caching"),
        "remote cache status should appear in --dry text mode"
    );
}

#[test]
fn test_prelude_single_package_format() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Running build"),
        "single-package prelude should show 'Running <task>'"
    );
    assert!(
        !stdout.contains("Packages in scope"),
        "single-package prelude must not show 'Packages in scope'"
    );
}
