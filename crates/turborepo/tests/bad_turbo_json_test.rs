#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{replace_turbo_json, run_turbo, setup};

// Validation rules are covered in process by turborepo-turbo-json (parser,
// validator, structured inputs, env prefixes, single-package loading) and
// turborepo-engine (interruptible/persistent, dependencyOutputs selection).
// These smokes prove diagnostics render through the CLI and that
// --single-package reaches the single-package loader.

#[test]
fn test_invalid_env_var_prefix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    replace_turbo_json(tempdir.path(), "invalid-env-var.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid_env_prefix"),
        "expected invalid env prefix error: {stderr}"
    );
    assert!(
        stderr.contains("$FOOBAR"),
        "expected $FOOBAR in error: {stderr}"
    );
}

#[test]
fn test_package_task_in_single_package_mode() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    replace_turbo_json(tempdir.path(), "invalid-env-var.json");

    let output = run_turbo(tempdir.path(), &["build", "--single-package"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("package_task_in_single_package_mode"),
        "expected single-package error: {stderr}"
    );
}
