#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, setup};

#[test]
fn test_continue_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_dependency_error",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter", "my-app...", "--log-order", "grouped"],
    );
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // some-lib fails, my-app never runs (it depends on some-lib)
    assert!(
        stdout.contains("1 successful, 2 total"),
        "expected only 1 success (base-lib), got: {stdout}"
    );
    assert!(
        stdout.contains("Failed:"),
        "expected failure summary, got: {stdout}"
    );
    // my-app should NOT have run
    assert!(
        !stdout.contains("my-app:build"),
        "my-app should not run when dependency fails, got: {stdout}"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "build",
            "--output-logs=errors-only",
            "--filter",
            "my-app...",
            "--log-order",
            "grouped",
        ],
    );
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Only error output should appear
    assert!(stdout.contains("some-lib:build"));
    assert!(stdout.contains("Failed:"));

    let output = run_turbo(
        tempdir.path(),
        &[
            "build",
            "--output-logs=errors-only",
            "--filter",
            "my-app...",
            "--continue",
            "--log-order",
            "grouped",
        ],
    );
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // With --continue, more tasks run despite some-lib failure
    assert!(
        stdout.contains("2 successful, 3 total"),
        "expected 2 successes with --continue, got: {stdout}"
    );

    let output = run_turbo(
        tempdir.path(),
        &[
            "build",
            "--output-logs=errors-only",
            "--continue=dependencies-successful",
            "--log-order",
            "grouped",
        ],
    );
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    // some-lib and other-app both fail
    assert!(
        stdout.contains("some-lib#build"),
        "expected some-lib failure, got: {stdout}"
    );
    assert!(
        stdout.contains("2 successful, 4 total"),
        "expected 2 successes in dependencies-successful mode, got: {stdout}"
    );
}
