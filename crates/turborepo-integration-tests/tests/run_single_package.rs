//! Port of turborepo-tests/integration/tests/run/single-package/*.t
//!
//! Tests single package (non-monorepo) turbo functionality:
//! - Basic runs with caching
//! - Dry run output format
//! - Running without turbo.json configuration
//! - Graph output
//! - Running tasks with dependencies
//! - Various output log levels

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Helper to set up a single_package test environment with packageManager set.
async fn setup_single_package_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("single_package").await?;
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;
    Ok(env)
}

// =============================================================================
// Basic Run Tests (from run.t)
// =============================================================================

/// Test basic single package run with cache miss on first run
#[tokio::test]
async fn test_single_package_run_cache_miss() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    // Verify .turbo/runs/ directory does NOT exist (no run summaries by default)
    assert!(
        !env.dir_exists(".turbo/runs/").await,
        ".turbo/runs/ should not exist without --summarize"
    );

    assert_snapshot!(
        "single_package_run_cache_miss",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that second run gets a cache hit
#[tokio::test]
async fn test_single_package_run_cache_hit() -> Result<()> {
    let env = setup_single_package_env().await?;

    // First run to populate cache
    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    // Second run should hit cache
    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_run_cache_hit",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// Dry Run Tests (from dry-run.t)
// =============================================================================

/// Test dry run output format for single package
#[tokio::test]
async fn test_single_package_dry_run() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "build", "--dry"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_dry_run",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// No Config Tests (from no-config.t)
// =============================================================================

/// Test dry run without turbo.json - shows different hash, no outputs, no log
/// file
#[tokio::test]
async fn test_single_package_no_config_dry_run() -> Result<()> {
    let env = setup_single_package_env().await?;

    // Remove turbo.json and commit the change
    env.remove_file("turbo.json").await?;
    env.git_commit("Delete turbo config").await?;

    let result = env.run_turbo(&["run", "build", "--dry"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_no_config_dry_run",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test graph output without turbo.json
#[tokio::test]
async fn test_single_package_no_config_graph() -> Result<()> {
    let env = setup_single_package_env().await?;

    // Remove turbo.json and commit the change
    env.remove_file("turbo.json").await?;
    env.git_commit("Delete turbo config").await?;

    let result = env.run_turbo(&["run", "build", "--graph"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_no_config_graph",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that without turbo.json, cache is bypassed on first run
#[tokio::test]
async fn test_single_package_no_config_cache_bypass_first_run() -> Result<()> {
    let env = setup_single_package_env().await?;

    // Remove turbo.json and commit the change
    env.remove_file("turbo.json").await?;
    env.git_commit("Delete turbo config").await?;

    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_no_config_cache_bypass_first_run",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that without turbo.json, cache is bypassed on second run too
#[tokio::test]
async fn test_single_package_no_config_cache_bypass_second_run() -> Result<()> {
    let env = setup_single_package_env().await?;

    // Remove turbo.json and commit the change
    env.remove_file("turbo.json").await?;
    env.git_commit("Delete turbo config").await?;

    // First run
    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    // Second run - should still bypass cache
    let result = env.run_turbo(&["run", "build"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_no_config_cache_bypass_second_run",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// Graph Tests (from graph.t)
// =============================================================================

/// Test graph output for single package
#[tokio::test]
async fn test_single_package_graph() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "build", "--graph"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_graph",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test graph output to file
#[tokio::test]
async fn test_single_package_graph_to_file() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["build", "--graph=graph.dot"]).await?;
    result.assert_success();

    // Verify the file was created and contains the expected content
    let graph_content = env.read_file("graph.dot").await?;
    assert!(
        graph_content.contains("[root] build"),
        "Graph should contain build task"
    );
    assert!(
        graph_content.contains("[root] ___ROOT___"),
        "Graph should contain root node"
    );

    Ok(())
}

// =============================================================================
// With Dependencies Run Tests (from with-deps-run.t)
// =============================================================================

/// Test running task with dependencies - cache miss on first run
#[tokio::test]
async fn test_single_package_with_deps_run_cache_miss() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_run_cache_miss",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test running task with dependencies - cache hit on second run
#[tokio::test]
async fn test_single_package_with_deps_run_cache_hit() -> Result<()> {
    let env = setup_single_package_env().await?;

    // First run to populate cache
    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    // Second run should hit cache
    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_run_cache_hit",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test --output-logs=hash-only shows only hashes
#[tokio::test]
async fn test_single_package_with_deps_output_logs_hash_only() -> Result<()> {
    let env = setup_single_package_env().await?;

    // First run to populate cache
    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    // Run with hash-only output
    let result = env
        .run_turbo(&["run", "test", "--output-logs=hash-only"])
        .await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_output_logs_hash_only",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test --output-logs=errors-only shows no output for successful tasks
#[tokio::test]
async fn test_single_package_with_deps_output_logs_errors_only() -> Result<()> {
    let env = setup_single_package_env().await?;

    // First run to populate cache
    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    // Run with errors-only output
    let result = env
        .run_turbo(&["run", "test", "--output-logs=errors-only"])
        .await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_output_logs_errors_only",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test --output-logs=none shows no task output
#[tokio::test]
async fn test_single_package_with_deps_output_logs_none() -> Result<()> {
    let env = setup_single_package_env().await?;

    // First run to populate cache
    let result = env.run_turbo(&["run", "test"]).await?;
    result.assert_success();

    // Run with no output
    let result = env
        .run_turbo(&["run", "test", "--output-logs=none"])
        .await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_output_logs_none",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// With Dependencies Dry Run Tests (from with-deps-dry-run.t)
// =============================================================================

/// Test dry run with dependencies shows both build and test tasks
#[tokio::test]
async fn test_single_package_with_deps_dry_run() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "test", "--dry"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_dry_run",
        redact_output(&result.combined_output())
    );

    Ok(())
}

// =============================================================================
// With Dependencies Graph Tests (from with-deps-graph.t)
// =============================================================================

/// Test graph output with dependencies shows correct task relationships
#[tokio::test]
async fn test_single_package_with_deps_graph() -> Result<()> {
    let env = setup_single_package_env().await?;

    let result = env.run_turbo(&["run", "test", "--graph"]).await?;
    result.assert_success();

    assert_snapshot!(
        "single_package_with_deps_graph",
        redact_output(&result.combined_output())
    );

    Ok(())
}
