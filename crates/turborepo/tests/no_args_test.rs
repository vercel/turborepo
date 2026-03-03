mod common;

use common::{run_turbo, run_turbo_with_env, setup, turbo_output_filters};

#[test]
fn test_no_args_prints_help() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &[]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let first_line = stderr.lines().next().unwrap_or("");
    assert_eq!(
        first_line, "The build system that makes ship happen",
        "expected help text as first line of stderr, got: {first_line}"
    );
}

#[test]
fn test_run_no_tasks_shows_potential_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("run_no_tasks", stdout.to_string());
    });
}

#[test]
fn test_run_no_tasks_with_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "--filter", "my-app"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("run_no_tasks_filtered", stdout.to_string());
    });
}

#[test]
fn test_watch_no_tasks_shows_potential_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["watch"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("watch_no_tasks", stdout.to_string());
    });
}

#[test]
fn test_env_var_for_run_does_not_change_no_args() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(tempdir.path(), &[], &[("TURBO_LOG_ORDER", "stream")]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let first_line = stderr.lines().next().unwrap_or("");
    assert_eq!(
        first_line, "The build system that makes ship happen",
        "expected help text even with TURBO_LOG_ORDER set, got: {first_line}"
    );
}

#[test]
fn test_composable_config_run_no_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("composable_config_run_no_tasks", stdout.to_string());
    });
}

#[test]
fn test_composable_config_watch_no_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["watch"]);
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("composable_config_watch_no_tasks", stdout.to_string());
    });
}
