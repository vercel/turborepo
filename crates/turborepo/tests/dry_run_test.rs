mod common;

use common::{run_turbo, run_turbo_with_env, setup};

#[test]
fn test_dry_run_packages_in_scope() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Packages in Scope"));
    assert!(stdout.contains("another"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
}

#[test]
fn test_dry_run_global_hash_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Global Hash Inputs"));
    assert!(stdout.contains("Global Env Vars"));
    assert!(stdout.contains("SOME_ENV_VAR"));
}

#[test]
fn test_dry_run_task_details() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check my-app task details
    assert!(stdout.contains("my-app#build"));
    assert!(stdout.contains("echo building"));

    // Check util task details
    assert!(stdout.contains("util#build"));
    assert!(stdout.contains("Env Vars"));
    assert!(stdout.contains("NODE_ENV"));
}

#[test]
fn test_dry_run_env_var_not_in_output() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Set NODE_ENV and verify it doesn't leak into the output as "Environment
    // Variables"
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry", "--filter=util"],
        &[("NODE_ENV", "banana")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The grep in prysk exits 1, meaning "Environment Variables" is NOT in the
    // output
    assert!(
        !stdout.contains("Environment Variables"),
        "should not contain 'Environment Variables' header: {stdout}"
    );
}
