mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn config_json(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("expected valid JSON from turbo config")
}

#[test]
fn test_config_defaults() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["config"]);
    assert!(output.status.success());

    let cfg = config_json(&output);
    assert_eq!(cfg["apiUrl"], "https://vercel.com/api");
    assert_eq!(cfg["loginUrl"], "https://vercel.com");
    assert!(cfg["teamSlug"].is_null());
    assert_eq!(cfg["signature"], false);
    assert_eq!(cfg["timeout"], 30);
    assert_eq!(cfg["uploadTimeout"], 60);
    assert_eq!(cfg["envMode"], "strict");
    assert!(cfg["daemon"].is_null());
    assert!(cfg["concurrency"].is_null());
}

#[test]
fn test_config_api_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["config", "--api", "http://localhost:8000"],
    );
    assert_eq!(config_json(&output)["apiUrl"], "http://localhost:8000");
}

#[test]
fn test_config_team_override() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["config", "--team", "vercel"]);
    assert_eq!(config_json(&output)["teamSlug"], "vercel");
}

#[test]
fn test_config_team_flag_beats_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["config", "--team", "turbo"],
        &[("TURBO_TEAM", "vercel")],
    );
    assert_eq!(config_json(&output)["teamSlug"], "turbo");
}

#[test]
fn test_config_timeout_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["config"],
        &[("TURBO_REMOTE_CACHE_TIMEOUT", "123")],
    );
    assert_eq!(config_json(&output)["timeout"], 123);
}

#[test]
fn test_config_timeout_flag_beats_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["config", "--remote-cache-timeout", "456"],
        &[("TURBO_REMOTE_CACHE_TIMEOUT", "123")],
    );
    assert_eq!(config_json(&output)["timeout"], 456);
}

#[test]
fn test_config_daemon_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let out_true = run_turbo_with_env(tempdir.path(), &["config"], &[("TURBO_DAEMON", "true")]);
    assert_eq!(config_json(&out_true)["daemon"], true);

    let out_false = run_turbo_with_env(tempdir.path(), &["config"], &[("TURBO_DAEMON", "false")]);
    assert_eq!(config_json(&out_false)["daemon"], false);
}

#[test]
fn test_config_env_mode() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Default is strict
    let output = run_turbo(tempdir.path(), &["config"]);
    assert_eq!(config_json(&output)["envMode"], "strict");

    // Override via env
    let output2 = run_turbo_with_env(tempdir.path(), &["config"], &[("TURBO_ENV_MODE", "loose")]);
    assert_eq!(config_json(&output2)["envMode"], "loose");

    // Override via flag
    let output3 = run_turbo(tempdir.path(), &["--env-mode=loose", "config"]);
    assert_eq!(config_json(&output3)["envMode"], "loose");
}

#[test]
fn test_config_scm_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["config"],
        &[("TURBO_SCM_BASE", "HEAD"), ("TURBO_SCM_HEAD", "my-branch")],
    );
    let cfg = config_json(&output);
    assert_eq!(cfg["scmBase"], "HEAD");
    assert_eq!(cfg["scmHead"], "my-branch");
}

#[test]
fn test_config_cache_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Via env
    let output = run_turbo_with_env(
        tempdir.path(),
        &["config"],
        &[("TURBO_CACHE_DIR", "FifthDimension/Nebulo9")],
    );
    let cache_dir = config_json(&output)["cacheDir"]
        .as_str()
        .unwrap()
        .to_string();
    let normalized = cache_dir.replace('\\', "/");
    assert_eq!(normalized, "FifthDimension/Nebulo9");

    // Via flag
    let output2 = run_turbo(
        tempdir.path(),
        &["--cache-dir", "FifthDimension/Nebulo9", "config"],
    );
    let cache_dir2 = config_json(&output2)["cacheDir"]
        .as_str()
        .unwrap()
        .to_string();
    let normalized2 = cache_dir2.replace('\\', "/");
    assert_eq!(normalized2, "FifthDimension/Nebulo9");
}

#[test]
fn test_config_concurrency() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Default is null
    let output = run_turbo(tempdir.path(), &["config"]);
    assert!(config_json(&output)["concurrency"].is_null());

    // Via env
    let output2 = run_turbo_with_env(tempdir.path(), &["config"], &[("TURBO_CONCURRENCY", "5")]);
    let conc2 = &config_json(&output2)["concurrency"];
    assert!(
        conc2 == 5 || conc2 == "5",
        "expected concurrency 5, got: {conc2}"
    );

    // Via flag
    let output3 = run_turbo(tempdir.path(), &["--concurrency=5", "config"]);
    let conc3 = &config_json(&output3)["concurrency"];
    assert!(
        conc3 == 5 || conc3 == "5",
        "expected concurrency 5, got: {conc3}"
    );
}
