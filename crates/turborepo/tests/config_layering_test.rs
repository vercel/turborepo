mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_fixture(dir: &std::path::Path) {
    setup::setup_integration_test(
        dir,
        "persistent_dependencies/10-too-many",
        "npm@10.5.0",
        false,
    )
    .unwrap();
}

fn set_concurrency(dir: &std::path::Path, concurrency: &str) {
    let turbo_json = dir.join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    json["concurrency"] = serde_json::Value::String(concurrency.to_string());
    fs::write(&turbo_json, serde_json::to_string_pretty(&json).unwrap()).unwrap();
}

#[test]
fn test_turbo_json_baseline() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    set_concurrency(tempdir.path(), "1");

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("concurrency of 1"));
}

#[test]
fn test_root_turbo_json_env_overrides_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    set_concurrency(tempdir.path(), "1");

    // Create alt config with concurrency=2
    let alt_json = tempdir.path().join("turbo-alt.json");
    let contents = fs::read_to_string(tempdir.path().join("turbo.json")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    json["concurrency"] = serde_json::Value::String("2".to_string());
    fs::write(&alt_json, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build"],
        &[("TURBO_ROOT_TURBO_JSON", "turbo-alt.json")],
    );
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("concurrency of 2"),
        "TURBO_ROOT_TURBO_JSON should override: {combined}"
    );
}

#[test]
fn test_cli_flag_overrides_root_turbo_json_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    set_concurrency(tempdir.path(), "1");

    let alt_json = tempdir.path().join("turbo-alt.json");
    let contents = fs::read_to_string(tempdir.path().join("turbo.json")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    json["concurrency"] = serde_json::Value::String("2".to_string());
    fs::write(&alt_json, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    // Flag should beat env
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--root-turbo-json=turbo.json"],
        &[("TURBO_ROOT_TURBO_JSON", "turbo-alt.json")],
    );
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("concurrency of 1"),
        "CLI flag should override env: {combined}"
    );
}

#[test]
fn test_env_overrides_local_config() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    set_concurrency(tempdir.path(), "1");

    // Create local config with concurrency=3
    fs::create_dir_all(tempdir.path().join(".turbo")).unwrap();
    fs::write(
        tempdir.path().join(".turbo/config.json"),
        r#"{"concurrency": "3"}"#,
    )
    .unwrap();

    // Env should override local config
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build"],
        &[("TURBO_CONCURRENCY", "1")],
    );
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("concurrency of 1"),
        "env should override local config: {combined}"
    );
}

#[test]
fn test_cli_flag_overrides_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());
    set_concurrency(tempdir.path(), "1");

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--concurrency=3"],
        &[("TURBO_CONCURRENCY", "1")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 successful, 2 total"),
        "CLI concurrency=3 should allow tasks to run: {stdout}"
    );
}
