//! Port of turborepo-tests/integration/tests/run/missing-tasks.t
//!
//! Tests error messages when running non-existent tasks.

// Skip on Windows - npm not found in test harness PATH on Windows CI
#![cfg(all(feature = "integration-tests", not(windows)))]

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
