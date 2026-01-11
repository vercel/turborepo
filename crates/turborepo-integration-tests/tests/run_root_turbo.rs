//! Port of turborepo-tests/integration/tests/run/no-root-turbo.t and
//! turborepo-tests/integration/tests/run/allow-no-root-turbo.t
//!
//! Tests for root turbo.json configuration options:
//! - `--root-turbo-json` flag to specify an alternate config file
//! - `TURBO_ROOT_TURBO_JSON` env var for the same purpose
//! - `--experimental-allow-no-turbo-json` flag to run without a config
//! - `TURBO_ALLOW_NO_TURBO_JSON` env var for the same purpose

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

// =============================================================================
// no-root-turbo.t tests: Using alternate config file names
// =============================================================================

/// Test that running without turbo.json fails when the file is renamed.
#[tokio::test]
async fn test_no_turbo_json_fails() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Rename turbo.json to turborepo.json
    env.rename_file("turbo.json", "turborepo.json").await?;

    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_failure();

    assert_snapshot!(
        "no_turbo_json_fails",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that `--root-turbo-json` flag allows specifying an alternate config.
#[tokio::test]
async fn test_root_turbo_json_flag() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Rename turbo.json to turborepo.json
    env.rename_file("turbo.json", "turborepo.json").await?;

    let result = env
        .run_turbo(&[
            "run",
            "build",
            "--filter=my-app",
            "--root-turbo-json=turborepo.json",
        ])
        .await?;
    result.assert_success();

    assert_snapshot!(
        "root_turbo_json_flag",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that `TURBO_ROOT_TURBO_JSON` env var allows specifying an alternate
/// config.
#[tokio::test]
async fn test_root_turbo_json_env_var() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Rename turbo.json to turborepo.json
    env.rename_file("turbo.json", "turborepo.json").await?;

    // First run to prime the cache
    let result = env
        .run_turbo_with_env(
            &["run", "build", "--filter=my-app"],
            &[("TURBO_ROOT_TURBO_JSON", "turborepo.json")],
        )
        .await?;
    result.assert_success();

    assert_snapshot!(
        "root_turbo_json_env_var",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// allow-no-root-turbo.t tests: Running without any turbo.json
// =============================================================================

/// Test that running without turbo.json and without the allow flag fails.
#[tokio::test]
async fn test_allow_no_turbo_json_fails_without_flag() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_no_turbo_json").await?;
    // The fixture has an invalid packageManager field ("bower"), fix it
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_failure();

    assert_snapshot!(
        "allow_no_turbo_json_fails",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that `--experimental-allow-no-turbo-json` allows running without
/// config.
#[tokio::test]
async fn test_allow_no_turbo_json_flag() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_no_turbo_json").await?;
    // The fixture has an invalid packageManager field ("bower"), fix it
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    let result = env
        .run_turbo_with_env(
            &["run", "test", "--experimental-allow-no-turbo-json"],
            &[("MY_VAR", "foo")],
        )
        .await?;
    result.assert_success();

    assert_snapshot!(
        "allow_no_turbo_json_flag",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that caching is disabled when running without turbo.json.
/// Running the same command twice should result in "cache bypass" both times.
#[tokio::test]
async fn test_allow_no_turbo_json_caching_disabled() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_no_turbo_json").await?;
    // The fixture has an invalid packageManager field ("bower"), fix it
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    // First run
    let result1 = env
        .run_turbo_with_env(
            &["run", "test", "--experimental-allow-no-turbo-json"],
            &[("MY_VAR", "foo")],
        )
        .await?;
    result1.assert_success();

    // Second run should still show "cache bypass", not "cache hit"
    let result2 = env
        .run_turbo_with_env(
            &["run", "test", "--experimental-allow-no-turbo-json"],
            &[("MY_VAR", "foo")],
        )
        .await?;
    result2.assert_success();

    // Verify it says "cache bypass" and not "cache hit"
    assert!(
        result2.output_contains("cache bypass"),
        "Expected 'cache bypass' in output, caching should be disabled"
    );
    assert!(
        !result2.output_contains("cache hit"),
        "Unexpected 'cache hit', caching should be disabled without turbo.json"
    );

    assert_snapshot!(
        "allow_no_turbo_json_caching_disabled",
        redact_output(&result2.combined_output())
    );

    Ok(())
}

/// Test that `TURBO_ALLOW_NO_TURBO_JSON` env var allows running without config.
/// Also tests that turbo can discover tasks from package.json scripts.
#[tokio::test]
async fn test_allow_no_turbo_json_env_var() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_no_turbo_json").await?;
    // The fixture has an invalid packageManager field ("bower"), fix it
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    let result = env
        .run_turbo_with_env(
            &["run", "build", "test", "--dry=json"],
            &[("TURBO_ALLOW_NO_TURBO_JSON", "true")],
        )
        .await?;
    result.assert_success();

    // Parse the JSON output to verify tasks were discovered
    let output = result.stdout.trim();
    assert!(
        output.contains("my-app#build"),
        "Expected my-app#build task to be discovered"
    );
    assert!(
        output.contains("my-app#test"),
        "Expected my-app#test task to be discovered"
    );
    assert!(
        output.contains("util#build"),
        "Expected util#build task to be discovered"
    );

    Ok(())
}
