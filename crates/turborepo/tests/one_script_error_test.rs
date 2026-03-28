mod common;

use common::{run_turbo, setup};

#[test]
fn test_script_error_reported_with_exit_code() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_one_script_error",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["error"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("my-app:error"),
        "expected error task output"
    );
    assert!(
        stdout.contains("1 successful, 2 total"),
        "expected 1 success out of 2 tasks, got: {stdout}"
    );
    assert!(
        stdout.contains("Failed:"),
        "expected Failed summary, got: {stdout}"
    );
}

#[test]
fn test_script_error_not_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_one_script_error",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // First run
    run_turbo(tempdir.path(), &["error"]);

    // Second run: error should not be cached, but okay should be
    let output = run_turbo(tempdir.path(), &["error"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("my-app:okay: cache hit"),
        "okay task should be cached, got: {stdout}"
    );
    assert!(
        stdout.contains("my-app:error: cache miss"),
        "error task should not be cached, got: {stdout}"
    );
}

#[test]
fn test_continue_preserves_error_exit_code() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_one_script_error",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["okay2", "--continue"]);
    assert!(
        !output.status.success(),
        "should still fail with --continue"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("my-app:okay2"),
        "okay2 should have run with --continue, got: {stdout}"
    );
    assert!(
        stdout.contains("2 successful, 3 total"),
        "expected 2 success out of 3 tasks, got: {stdout}"
    );
}
