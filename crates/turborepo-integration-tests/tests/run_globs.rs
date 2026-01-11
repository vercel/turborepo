//! Port of turborepo-tests/integration/tests/run/globs.t
//!
//! Tests that:
//! - Input directory changes cause cache misses
//! - Cache restores output files correctly
#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;
use regex::Regex;

/// Additional redaction for paths split across lines in error messages.
/// The common redact_output handles most temp paths, but errors can split like:
///   `-> Lockfile not found at /private/var/
///       folders/.../T/.tmpXXX/file
/// This catches the "/private/var/" prefix left on its own line.
fn redact_globs_output(output: &str) -> String {
    let output = redact_output(output);
    // Catch any remaining "/private/var/" prefix left on its own
    let var_prefix_re = Regex::new(r"(?:/private)?/var/\s*$").expect("Invalid var prefix regex");
    var_prefix_re
        .replace_all(&output, "[TEMP_DIR]")
        .into_owned()
}

/// Base turbo args for all tests in this file
const BASE_ARGS: &[&str] = &["run", "build", "--filter=util", "--output-logs=hash-only"];

/// Test that adding a file to an input directory causes a cache miss.
///
/// This verifies that turbo correctly detects changes in input directories
/// specified with glob patterns like "src/".
#[tokio::test]
async fn test_input_directory_change_causes_cache_miss() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("dir_globs").await?;
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    // First run - should be a cache miss
    let result1 = env.run_turbo(BASE_ARGS).await?;
    result1.assert_success();
    assert_snapshot!(
        "input_dir_first_run",
        redact_globs_output(&result1.combined_output())
    );

    // Add a new file to the input directory
    env.touch_file("packages/util/src/oops.txt").await?;

    // Second run - should be a cache miss due to new file
    let result2 = env.run_turbo(BASE_ARGS).await?;
    result2.assert_success();
    assert_snapshot!(
        "input_dir_after_touch",
        redact_globs_output(&result2.combined_output())
    );

    // Verify the output contains cache miss
    assert!(
        result2.output_contains("cache miss"),
        "Expected cache miss after adding file to input directory"
    );

    Ok(())
}

/// Test that cache restores output files when they are deleted.
///
/// This verifies that turbo correctly restores outputs from cache when
/// the output files are missing but the inputs haven't changed.
#[tokio::test]
async fn test_cache_restores_output_files() -> Result<()> {
    let mut env = TurboTestEnv::new().await?;
    env.copy_fixture("dir_globs").await?;
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    // First run - build and populate cache
    let result1 = env.run_turbo(BASE_ARGS).await?;
    result1.assert_success();

    // Verify output file exists with expected content
    let content = env.read_file("packages/util/dist/hello.txt").await?;
    assert_eq!(
        content.trim(),
        "world",
        "Output file should contain 'world'"
    );

    // Delete the output file
    env.remove_file("packages/util/dist/hello.txt").await?;
    assert!(
        !env.file_exists("packages/util/dist/hello.txt").await,
        "File should be deleted"
    );

    // Run turbo again - should be a cache hit and restore the file
    let result2 = env.run_turbo(BASE_ARGS).await?;
    result2.assert_success();
    assert_snapshot!(
        "cache_restore_output",
        redact_globs_output(&result2.combined_output())
    );

    // Verify cache hit
    assert!(
        result2.output_contains("cache hit") || result2.output_contains("FULL TURBO"),
        "Expected cache hit after deleting output file"
    );

    // Verify the file was restored from cache
    assert!(
        env.file_exists("packages/util/dist/hello.txt").await,
        "Output file should be restored from cache"
    );
    let restored_content = env.read_file("packages/util/dist/hello.txt").await?;
    assert_eq!(
        restored_content.trim(),
        "world",
        "Restored file should contain 'world'"
    );

    Ok(())
}
