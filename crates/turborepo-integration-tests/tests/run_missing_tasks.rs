//! Port of turborepo-tests/integration/tests/run/missing-tasks.t
//!
//! Tests error messages when running non-existent tasks:
//! - Single non-existent task produces appropriate error
//! - Multiple non-existent tasks lists all missing tasks
//! - Mix of valid and invalid tasks still errors
//! - Root-level scripts that invoke turbo are detected as potential loops

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Helper to set up a basic test environment with git initialized.
async fn setup_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;
    Ok(env)
}

#[tokio::test]
async fn test_single_nonexistent_task_errors() -> Result<()> {
    let env = setup_env().await?;

    let result = env.run_turbo(&["run", "doesnotexist"]).await?;
    result.assert_exit_code(1);

    assert_snapshot!(
        "single_nonexistent_task",
        redact_output(&result.combined_output())
    );

    Ok(())
}

#[tokio::test]
async fn test_multiple_nonexistent_tasks_error() -> Result<()> {
    let env = setup_env().await?;

    let result = env.run_turbo(&["run", "doesnotexist", "alsono"]).await?;
    result.assert_exit_code(1);

    assert_snapshot!(
        "multiple_nonexistent_tasks",
        redact_output(&result.combined_output())
    );

    Ok(())
}

#[tokio::test]
async fn test_one_good_one_bad_task_errors() -> Result<()> {
    let env = setup_env().await?;

    let result = env.run_turbo(&["run", "build", "doesnotexist"]).await?;
    result.assert_exit_code(1);

    assert_snapshot!(
        "one_good_one_bad_task",
        redact_output(&result.combined_output())
    );

    Ok(())
}

/// Test that running a root-level script that invokes turbo detects the
/// recursive invocation and shows a proper warning.
///
/// The basic_monorepo fixture has a root package.json with:
///   "something": "turbo run build"
///
/// When running `turbo run something`, turbo should:
/// 1. Detect that the root script would recursively invoke turbo
/// 2. Show the recursive_turbo_invocations warning with helpful diagnostics
/// 3. Exit with code 1
///
/// Note: The old prysk test (missing-tasks.t) expected this warning NOT to
/// appear, but turbo has since improved its detection and now properly warns
/// about this potential infinite loop scenario.
#[tokio::test]
async fn test_root_script_with_turbo_invocation_detects_loop_dry() -> Result<()> {
    let env = setup_env().await?;

    // Run with --dry flag
    let result = env.run_turbo(&["run", "something", "--dry"]).await?;
    result.assert_exit_code(1);

    let output = result.combined_output();

    // Should show recursive turbo invocation warning
    assert!(
        output.contains("recursive_turbo_invocations")
            || output.contains("creating a loop of `turbo` invocations"),
        "Expected recursive turbo invocation warning, got: {}",
        output
    );

    // Should mention the problematic script
    assert!(
        output.contains("turbo run build"),
        "Expected warning to mention the script content"
    );

    assert_snapshot!("root_script_recursive_turbo_dry", redact_output(&output));

    Ok(())
}

/// Same test as above but without --dry flag.
/// Verifies the recursive invocation detection works in both modes.
#[tokio::test]
async fn test_root_script_with_turbo_invocation_detects_loop() -> Result<()> {
    let env = setup_env().await?;

    // Run without --dry flag
    let result = env.run_turbo(&["run", "something"]).await?;
    result.assert_exit_code(1);

    let output = result.combined_output();

    // Should show recursive turbo invocation warning
    assert!(
        output.contains("recursive_turbo_invocations")
            || output.contains("creating a loop of `turbo` invocations"),
        "Expected recursive turbo invocation warning, got: {}",
        output
    );

    // Should mention the problematic script
    assert!(
        output.contains("turbo run build"),
        "Expected warning to mention the script content"
    );

    assert_snapshot!("root_script_recursive_turbo", redact_output(&output));

    Ok(())
}
