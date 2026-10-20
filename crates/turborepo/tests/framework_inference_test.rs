#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{run_turbo, run_turbo_with_env, setup};

fn setup_framework(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "framework_inference", "npm@10.5.0", false).unwrap();
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

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--framework-inference=false", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["frameworkInference"], false);
    assert_eq!(json["tasks"][0]["framework"], "");
    assert_eq!(
        json["tasks"][0]["environmentVariables"]["inferred"],
        serde_json::json!([])
    );
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

#[test]
fn test_global_env_exclusion_keeps_task_env_in_hash() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_framework(tempdir.path());

    // Opt out of inferred NEXT_PUBLIC_* globally, but declare one of them on
    // the task. The task can read it through the builtin NEXT_* pass-through,
    // so it has to stay in the hash.
    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "globalEnv": ["!NEXT_PUBLIC_*"],
  "tasks": {
    "build": { "env": ["NEXT_PUBLIC_API_URL"] }
  }
}"#,
    )
    .unwrap();

    let dry_run = |api_url: &str| -> serde_json::Value {
        let output = run_turbo_with_env(
            tempdir.path(),
            &["run", "build", "--dry=json"],
            &[
                ("NEXT_PUBLIC_API_URL", api_url),
                ("NEXT_PUBLIC_CHANGED", "true"),
            ],
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let preview = dry_run("https://preview.example.com");
    let production = dry_run("https://example.com");

    let task = &preview["tasks"][0];
    assert_eq!(task["framework"], "nextjs");
    assert_eq!(
        task["environmentVariables"]["inferred"],
        serde_json::json!([])
    );
    let configured = task["environmentVariables"]["configured"]
        .as_array()
        .unwrap();
    assert_eq!(configured.len(), 1);
    assert!(
        configured[0]
            .as_str()
            .unwrap()
            .starts_with("NEXT_PUBLIC_API_URL=")
    );
    assert_ne!(task["hash"], production["tasks"][0]["hash"]);
}
