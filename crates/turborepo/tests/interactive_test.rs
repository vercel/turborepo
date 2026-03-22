mod common;

use common::{replace_turbo_json, run_turbo, setup};

#[test]
fn test_interactive_cacheable_task_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "interactive.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Tasks cannot be marked as interactive and cacheable"),
        "expected interactive+cacheable error, got: {stderr}"
    );
}
