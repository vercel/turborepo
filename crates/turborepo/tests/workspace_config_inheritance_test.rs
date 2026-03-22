mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

// Tests that root turbo.json config (outputs, inputs, env) is retained
// when a workspace has no turbo.json.

#[test]
fn test_missing_workspace_config_outputs_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    // First run: cache miss
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"));

    // Second run: cache hit (outputLogs is "new-only" from root, so logs
    // suppressed)
    let output2 = run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected cache hit: {stdout2}"
    );
}

#[test]
fn test_missing_workspace_config_inputs_cause_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    // Prime cache
    run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );

    // Change the declared input file
    let foo_path = tempdir
        .path()
        .join("apps/missing-workspace-config/src/foo.txt");
    let mut contents = fs::read_to_string(&foo_path).unwrap();
    contents.push_str("\nmore text");
    fs::write(&foo_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss after input change: {stdout}"
    );
}

#[test]
fn test_missing_workspace_config_non_input_no_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    // Prime cache (two runs to get past initial miss + input change)
    run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );

    // Change a file NOT in inputs
    let bar_path = tempdir
        .path()
        .join("apps/missing-workspace-config/src/bar.txt");
    let mut contents = fs::read_to_string(&bar_path).unwrap_or_default();
    contents.push_str("\nmore text");
    fs::write(&bar_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "non-input change should not bust cache: {stdout}"
    );
}

#[test]
fn test_missing_workspace_config_env_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    // Prime cache
    run_turbo(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
        ],
        &[("SOME_VAR", "somevalue")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "env var should bust cache: {stdout}"
    );
}

#[test]
fn test_missing_workspace_config_cache_false_not_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "cached-task-4", "--filter=missing-workspace-config"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache bypass"),
        "cache:false task should bypass: {stdout}"
    );
}

// Tests that root turbo.json config is retained when workspace defines a task
// but omits all keys.

#[test]
fn test_omit_keys_outputs_cached() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss"));

    // Second run: cache hit
    let output2 = run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected cache hit: {stdout2}"
    );
}

#[test]
fn test_omit_keys_inputs_cause_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );

    let foo_path = tempdir.path().join("apps/omit-keys/src/foo.txt");
    let mut contents = fs::read_to_string(&foo_path).unwrap();
    contents.push_str("\nmore text");
    fs::write(&foo_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected miss after input change: {stdout}"
    );
}

#[test]
fn test_omit_keys_non_input_no_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );

    let bar_path = tempdir.path().join("apps/omit-keys/src/bar.txt");
    let mut contents = fs::read_to_string(&bar_path).unwrap_or_default();
    contents.push_str("\nmore text");
    fs::write(&bar_path, contents).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "non-input change should not bust cache: {stdout}"
    );
}

#[test]
fn test_omit_keys_env_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    run_turbo(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "omit-keys-task", "--filter=omit-keys"],
        &[("SOME_VAR", "somevalue")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "env var should bust cache: {stdout}"
    );
}
