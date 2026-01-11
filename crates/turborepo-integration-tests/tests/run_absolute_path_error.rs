//! Port of turborepo-tests/integration/tests/run/absolute-path-error.t
//!
//! Tests that turbo produces appropriate error messages when absolute paths
//! are used in turbo.json configuration fields that don't support them:
//! - `inputs` cannot contain absolute paths
//! - `outputs` cannot contain absolute paths
//! - `globalDependencies` cannot contain absolute paths

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Test that absolute paths in `inputs` produce an error.
#[tokio::test]
async fn test_absolute_path_in_inputs() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;

    // Write turbo.json with absolute path in inputs
    let turbo_json = r#"{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "inputs": ["/another/absolute/path", "a/relative/path"]
    }
  }
}"#;
    env.write_file("turbo.json", turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`inputs` cannot contain an absolute path"),
        "Expected error about absolute path in inputs.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    assert_snapshot!(
        "absolute_path_in_inputs",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that absolute paths in `outputs` produce an error.
#[tokio::test]
async fn test_absolute_path_in_outputs() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;

    // Write turbo.json with absolute path in outputs
    let turbo_json = r#"{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "outputs": ["/another/absolute/path", "a/relative/path"]
    }
  }
}"#;
    env.write_file("turbo.json", turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`outputs` cannot contain an absolute path"),
        "Expected error about absolute path in outputs.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    assert_snapshot!(
        "absolute_path_in_outputs",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that absolute paths in `globalDependencies` produce an error.
#[tokio::test]
async fn test_absolute_path_in_global_dependencies() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;

    // Write turbo.json with absolute path in globalDependencies
    let turbo_json = r#"{
  "$schema": "https://turborepo.com/schema.json",
  "globalDependencies": ["/an/absolute/path", "some/file"]
}"#;
    env.write_file("turbo.json", turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`globalDependencies` cannot contain an absolute path"),
        "Expected error about absolute path in globalDependencies.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    assert_snapshot!(
        "absolute_path_in_global_dependencies",
        redact_output(&result.combined_output())
    );

    Ok(())
}
