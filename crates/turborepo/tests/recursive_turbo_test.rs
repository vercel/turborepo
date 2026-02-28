mod common;

use common::{run_turbo, setup};

#[test]
fn test_recursive_turbo_invocation_detected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["something"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");

    assert!(
        combined.contains("recursive_turbo_invocations"),
        "expected recursive turbo invocation error, got: {combined}"
    );
    assert!(
        combined.contains("creating a loop"),
        "expected loop warning, got: {combined}"
    );
}
