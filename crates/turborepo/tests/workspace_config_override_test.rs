mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

// Tests that workspace turbo.json can override outputs, inputs, env,
// outputLogs.

#[test]
fn test_override_values_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"));

    // Second run: cache hit with full output (outputLogs overridden to "full")
    let output2 = run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected cache hit: {stdout2}"
    );
    // outputLogs is overridden to "full", so logs should be replayed
    assert!(
        stdout2.contains("replaying logs"),
        "expected full replay with overridden outputLogs: {stdout2}"
    );
}

#[test]
fn test_override_values_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );

    // Change the workspace input (bar.txt, not foo.txt which is the root input)
    let bar_path = tempdir.path().join("apps/override-values/src/bar.txt");
    let contents = fs::read_to_string(&bar_path).unwrap_or_default();
    fs::write(&bar_path, format!("{contents}\nmore text")).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "workspace input change should miss: {stdout}"
    );
}

#[test]
fn test_override_values_root_input_no_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );

    // Change the ROOT input (foo.txt) — should NOT cause miss because workspace
    // overrides inputs
    let foo_path = tempdir.path().join("apps/override-values/src/foo.txt");
    let mut contents = fs::read_to_string(&foo_path).unwrap();
    contents.push_str("\nmore text");
    fs::write(&foo_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "root input should be overridden, no miss: {stdout}"
    );
}

#[test]
fn test_override_values_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
    );

    // Workspace overrides env to OTHER_VAR (not SOME_VAR from root)
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "override-values-task", "--filter=override-values"],
        &[("OTHER_VAR", "somevalue")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "overridden env var should miss: {stdout}"
    );
}

// Tests that a workspace can add keys when root task has empty config.

#[test]
fn test_add_keys_deps_and_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // dependsOn should cause underlying task to run
    assert!(
        stdout.contains("add-keys-underlying-task"),
        "dependent task should run: {stdout}"
    );
    assert!(stdout.contains("2 successful, 2 total"));
}

#[test]
fn test_add_keys_cache_and_output_logs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    // Prime cache
    run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );

    // Second run: cache hit, outputLogs "new-only" means add-keys-task logs
    // suppressed
    let output = run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected cache hit: {stdout}"
    );
    assert!(
        stdout.contains("add-keys:add-keys-task: cache hit, suppressing logs"),
        "outputLogs new-only should suppress on hit: {stdout}"
    );
}

#[test]
fn test_add_keys_input_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );

    let foo_path = tempdir.path().join("apps/add-keys/src/foo.txt");
    let mut contents = fs::read_to_string(&foo_path).unwrap();
    contents.push_str("\nmore text");
    fs::write(&foo_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 cached, 2 total"),
        "input change should miss: {stdout}"
    );
}

#[test]
fn test_add_keys_env_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "add-keys-task", "--filter=add-keys"],
        &[("SOME_VAR", "somevalue")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("add-keys:add-keys-task: cache miss"),
        "env var should bust add-keys-task cache: {stdout}"
    );
}

// Tests that a workspace can define entirely new tasks.

#[test]
fn test_add_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "added-task", "--filter=add-tasks"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("add-tasks:added-task: cache miss"));
    assert!(stdout.contains("1 successful, 1 total"));
}
