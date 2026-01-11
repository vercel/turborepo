//! Port of turborepo-tests/integration/tests/run/absolute-path-error.t
//!
//! Tests that turbo produces appropriate error messages when absolute paths
//! are used in turbo.json configuration fields that don't support them:
//! - `inputs` cannot contain absolute paths
//! - `outputs` cannot contain absolute paths
//! - `globalDependencies` cannot contain absolute paths
//!
//! Note: Uses platform-specific absolute paths since `/path` is not absolute on
//! Windows.

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Get an absolute path appropriate for the current platform.
/// Uses paths from the original prysk test fixtures.
fn absolute_path() -> &'static str {
    #[cfg(windows)]
    {
        "C:\\another\\absolute\\path"
    }
    #[cfg(not(windows))]
    {
        "/another/absolute/path"
    }
}

/// Get an absolute path for globalDependencies test.
fn global_deps_absolute_path() -> &'static str {
    #[cfg(windows)]
    {
        "C:\\an\\absolute\\path"
    }
    #[cfg(not(windows))]
    {
        "/an/absolute/path"
    }
}

/// Get a relative path appropriate for the current platform.
fn relative_path() -> &'static str {
    #[cfg(windows)]
    {
        "a\\relative\\path"
    }
    #[cfg(not(windows))]
    {
        "a/relative/path"
    }
}

/// Test that absolute paths in `inputs` produce an error.
#[tokio::test]
async fn test_absolute_path_in_inputs() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;

    // Write turbo.json with absolute path in inputs (platform-specific)
    let turbo_json = format!(
        r#"{{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {{
    "build": {{
      "inputs": ["{}", "{}"]
    }}
  }}
}}"#,
        absolute_path(),
        relative_path()
    );
    env.write_file("turbo.json", &turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`inputs` cannot contain an absolute path"),
        "Expected error about absolute path in inputs.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    // Snapshot test only on Unix - Windows has different path format in output
    #[cfg(not(windows))]
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

    // Write turbo.json with absolute path in outputs (platform-specific)
    let turbo_json = format!(
        r#"{{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {{
    "build": {{
      "outputs": ["{}", "{}"]
    }}
  }}
}}"#,
        absolute_path(),
        relative_path()
    );
    env.write_file("turbo.json", &turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`outputs` cannot contain an absolute path"),
        "Expected error about absolute path in outputs.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    // Snapshot test only on Unix - Windows has different path format in output
    #[cfg(not(windows))]
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

    // Write turbo.json with absolute path in globalDependencies (platform-specific)
    // Note: Uses a different path format than inputs/outputs to match original test
    let turbo_json = format!(
        r#"{{
  "$schema": "https://turborepo.com/schema.json",
  "globalDependencies": ["{}", "some/file"]
}}"#,
        global_deps_absolute_path()
    );
    env.write_file("turbo.json", &turbo_json).await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;

    result.assert_failure();
    assert!(
        result.output_contains("`globalDependencies` cannot contain an absolute path"),
        "Expected error about absolute path in globalDependencies.\nstdout: {}\nstderr: {}",
        result.stdout,
        result.stderr
    );

    // Snapshot test only on Unix - Windows has different path format in output
    #[cfg(not(windows))]
    assert_snapshot!(
        "absolute_path_in_global_dependencies",
        redact_output(&result.combined_output())
    );

    Ok(())
}
