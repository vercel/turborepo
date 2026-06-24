#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{replace_turbo_json, run_turbo, setup};

#[test]
fn test_package_task_syntax_in_workspace_config() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

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

#[test]
fn test_interruptible_but_not_persistent() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
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
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    replace_turbo_json(tempdir.path(), "syntax-error.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("turbo_json_parse_error"),
        "expected parse error: {stderr}"
    );
}

fn write_turbo_json(dir: &std::path::Path, contents: &str) {
    std::fs::write(dir.join("turbo.json"), contents).unwrap();
}

#[test]
fn test_structured_startup_cannot_mix_with_legacy_startup_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        "$TURBO_DEFAULT$",
        {
          "mode": "startup",
          "globs": ["src/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Legacy input strings normalize to mode \"startup\""),
        "expected duplicate startup normalization error, got: {stderr}"
    );
    assert!(
        stderr.contains("Use either legacy startup inputs")
            && stderr.contains("Or one structured startup input"),
        "expected legacy-or-structured guidance, got: {stderr}"
    );
}

#[test]
fn test_structured_inputs_reject_duplicate_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "jit",
          "globs": ["src/generated/**"]
        },
        {
          "mode": "jit",
          "globs": ["other-generated/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("duplicate") && stderr.contains("jit"),
        "expected duplicate jit mode error, got: {stderr}"
    );
}

#[test]
fn test_structured_inputs_require_mode() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "globs": ["src/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mode"),
        "expected missing structured input mode error, got: {stderr}"
    );
}

#[test]
fn test_structured_inputs_reject_unknown_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "runtime",
          "globs": ["src/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("runtime") && stderr.contains("mode"),
        "expected unknown structured input mode error, got: {stderr}"
    );
}

#[test]
fn test_structured_input_from_only_allowed_for_dependency_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "jit",
          "from": ["codegen"],
          "globs": ["src/generated/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("from") && stderr.contains("dependencyOutputs"),
        "expected from/dependencyOutputs validation error, got: {stderr}"
    );
}

#[test]
fn test_structured_input_with_defaults_only_allowed_for_startup_or_jit() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "withDefaults": true
        }
      ],
      "outputs": ["dist/**"]
    },
    "codegen": {
      "outputs": ["src/generated/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("withDefaults") && stderr.contains("startup") && stderr.contains("jit"),
        "expected withDefaults mode validation error, got: {stderr}"
    );
}

#[test]
fn test_structured_input_globs_reject_sentinels() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "startup",
          "globs": ["$TURBO_DEFAULT$", "$TURBO_EXTENDS$"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("$TURBO_DEFAULT$") || stderr.contains("$TURBO_EXTENDS$"),
        "expected sentinel-in-structured-globs validation error, got: {stderr}"
    );
}

#[test]
fn test_structured_startup_rejects_negative_only_globs_without_defaults() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "startup",
          "globs": ["!src/generated/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("negative") && stderr.contains("withDefaults"),
        "expected negative-only startup glob validation error, got: {stderr}"
    );
}

#[test]
fn test_structured_jit_rejects_negative_only_globs_without_defaults() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    write_turbo_json(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": [
        {
          "mode": "jit",
          "globs": ["!src/generated/**"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("negative") && stderr.contains("withDefaults"),
        "expected negative-only jit glob validation error, got: {stderr}"
    );
}
