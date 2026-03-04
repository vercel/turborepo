mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_framework(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "framework_inference", "npm@10.5.0", false).unwrap();
}

#[test]
fn test_no_inferred_vars_by_default() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = &json["tasks"][0]["environmentVariables"]["inferred"];
    assert_eq!(inferred, &serde_json::json!([]));
}

#[test]
fn test_next_public_var_inferred() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = json["tasks"][0]["environmentVariables"]["inferred"]
        .as_array()
        .unwrap();
    assert_eq!(inferred.len(), 1);
    assert!(
        inferred[0]
            .as_str()
            .unwrap()
            .starts_with("NEXT_PUBLIC_CHANGED=")
    );
}

#[test]
fn test_turbo_ci_vendor_env_key_excludes_var() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry=json"],
        &[
            ("NEXT_PUBLIC_CHANGED", "true"),
            ("NEXT_PUBLIC_IGNORED_VALUE", "true"),
            ("TURBO_CI_VENDOR_ENV_KEY", "NEXT_PUBLIC_IGNORED_"),
        ],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = json["tasks"][0]["environmentVariables"]["inferred"]
        .as_array()
        .unwrap();
    assert_eq!(inferred.len(), 1);
    assert!(
        inferred[0]
            .as_str()
            .unwrap()
            .starts_with("NEXT_PUBLIC_CHANGED=")
    );
}

#[test]
fn test_framework_inference_disabled() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--framework-inference=false", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = &json["tasks"][0]["environmentVariables"]["inferred"];
    assert_eq!(inferred, &serde_json::json!([]));
}

#[test]
fn test_framework_inference_run_summary() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--framework-inference=true", "--dry=json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["frameworkInference"], true);
    assert_eq!(json["tasks"][0]["framework"], "nextjs");
}

#[test]
fn test_framework_inference_disabled_summary() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--framework-inference=false", "--dry=json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["frameworkInference"], false);
    assert_eq!(json["tasks"][0]["framework"], "");
}

#[test]
fn test_env_exclusion_overrides_inference() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    // Add env exclusion pattern to turbo.json
    let turbo_json = tempdir.path().join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    json["tasks"]["build"]["env"] = serde_json::json!(["!NEXT_PUBLIC_*"]);
    fs::write(&turbo_json, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = &result["tasks"][0]["environmentVariables"]["inferred"];
    assert_eq!(inferred, &serde_json::json!([]));

    // Framework is still detected
    assert_eq!(result["tasks"][0]["framework"], "nextjs");
}

#[test]
fn test_global_env_exclusion_overrides_inference() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    // Use globalEnv exclusion instead of task-level
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "globalEnv": ["!NEXT_PUBLIC_*"],
  "globalPassThroughEnv": [],
  "tasks": {
    "build": {}
  }
}"#,
    )
    .unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let inferred = &result["tasks"][0]["environmentVariables"]["inferred"];
    assert_eq!(inferred, &serde_json::json!([]));
}
