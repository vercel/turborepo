mod common;

use common::{run_turbo, setup};

#[test]
fn test_single_missing_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "doesnotexist"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `doesnotexist` in project"),
        "expected missing task error, got: {stderr}"
    );
}

#[test]
fn test_multiple_missing_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "doesnotexist", "alsono"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `doesnotexist` in project"),
        "expected doesnotexist error, got: {stderr}"
    );
    assert!(
        stderr.contains("Could not find task `alsono` in project"),
        "expected alsono error, got: {stderr}"
    );
}

#[test]
fn test_one_good_one_bad_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "doesnotexist"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Could not find task `doesnotexist` in project"),
        "expected missing task error even with valid task present, got: {stderr}"
    );
}

// Running a task named "something" should NOT produce a recursive turbo
// invocation warning when the task doesn't exist.
#[test]
fn test_no_recursive_turbo_warning_for_missing_task() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "something", "--dry"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("looks like it invokes turbo"),
        "should not warn about recursive turbo invocation, got: {combined}"
    );
    assert!(
        !combined.contains("might cause a loop"),
        "should not warn about loops, got: {combined}"
    );
}
