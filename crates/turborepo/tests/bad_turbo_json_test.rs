mod common;

use common::{replace_turbo_json, run_turbo, setup};

#[test]
fn test_package_task_syntax_in_workspace_config() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let app_dir = tempdir.path().join("apps/my-app");
    replace_turbo_json(&app_dir, "package-task.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unnecessary_package_task_syntax"),
        "expected package task error: {stderr}"
    );
}

#[test]
fn test_invalid_env_var_prefix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
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
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "invalid-env-var.json");

    let output = run_turbo(tempdir.path(), &["build", "--single-package"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("package_task_in_single_package_mode"),
        "expected single-package error: {stderr}"
    );
}

#[test]
fn test_interruptible_but_not_persistent() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "interruptible-but-not-persistent.json");

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Interruptible tasks must be persistent"),
        "expected interruptible error: {stderr}"
    );
}

#[test]
fn test_syntax_error_in_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "syntax-error.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("turbo_json_parse_error"),
        "expected parse error: {stderr}"
    );
}
