//! Port of turborepo-tests/integration/tests/run/one-script-error.t
//!
//! Tests error handling behavior in turbo:
//! - Errors are properly reported with exit code 1
//! - Failed tasks are not cached (re-running shows cache miss for error task)
//! - With --continue, errors don't prevent other tasks from running but exit
//!   code is still 1

#![cfg(feature = "integration-tests")]

mod common;

use std::sync::LazyLock;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;
use regex::Regex;

/// Regex to redact file paths in npm error output.
/// Matches patterns like `/path/to/apps/my-app` or `C:\path\to\apps\my-app`.
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:command \(|at location: )[^\)]+").expect("Invalid path regex")
});

/// Regex to redact npm command paths (e.g., `/usr/bin/npm` or `npm.cmd`).
static NPM_CMD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\w/\\\.]+npm(?:\.cmd)?").expect("Invalid npm command regex"));

/// Apply additional redactions specific to error output.
///
/// This handles npm error messages that contain file paths like:
/// - `at location: /path/to/apps/my-app`
/// - `command (/path/to/apps/my-app)`
/// - `/usr/local/bin/npm run error`
fn redact_error_output(output: &str) -> String {
    let output = redact_output(output);
    let output = PATH_RE.replace_all(&output, "[PATH]");
    NPM_CMD_RE.replace_all(&output, "[NPM]").into_owned()
}

/// Set up the test environment with the monorepo_one_script_error fixture.
async fn setup_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_one_script_error").await?;
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;
    Ok(env)
}

/// Test that errors are properly reported.
///
/// The task uses `exit 2`, and turbo propagates the script's exit code.
#[tokio::test]
async fn test_error_is_properly_reported() -> Result<()> {
    let env = setup_env().await?;

    let result = env.run_turbo(&["run", "error"]).await?;

    result.assert_failure();
    assert_snapshot!(
        "error_properly_reported",
        redact_error_output(&result.combined_output())
    );

    Ok(())
}

/// Test that errors are not cached.
///
/// Running `turbo error` twice should show:
/// - First run: cache miss for both `okay` and `error` tasks
/// - Second run: cache hit for `okay`, cache miss for `error` (errors aren't
///   cached)
#[tokio::test]
async fn test_error_is_not_cached() -> Result<()> {
    let env = setup_env().await?;

    // First run - primes cache for successful tasks
    let _first_run = env.run_turbo(&["run", "error"]).await?;

    // Second run - error task should still be cache miss
    let result = env.run_turbo(&["run", "error"]).await?;

    result.assert_failure();

    // Verify the okay task was cached but error task was not
    let output = result.combined_output();
    assert!(
        output.contains("my-app:okay: cache hit"),
        "okay task should be cached"
    );
    assert!(
        output.contains("my-app:error: cache miss"),
        "error task should not be cached"
    );

    assert_snapshot!(
        "error_not_cached",
        redact_error_output(&result.combined_output())
    );

    Ok(())
}

/// Test that --continue allows other tasks to run despite errors.
///
/// The task graph is: okay -> error -> okay2
/// With --continue:
/// - okay runs and succeeds
/// - error runs and fails
/// - okay2 still runs (because of --continue)
/// - Exit code is non-zero (error is not swallowed)
#[tokio::test]
async fn test_continue_runs_other_tasks_but_preserves_error_exit_code() -> Result<()> {
    let env = setup_env().await?;

    // Prime cache for okay task
    let _first_run = env.run_turbo(&["run", "error"]).await?;

    // Run okay2 with --continue
    let result = env.run_turbo(&["run", "okay2", "--continue"]).await?;

    result.assert_failure();

    // Verify okay2 task ran despite error in dependency
    let output = result.combined_output();
    assert!(
        output.contains("my-app:okay2:"),
        "okay2 task should have run with --continue"
    );
    assert!(
        output.contains("my-app#error:"),
        "error should be reported in summary"
    );

    assert_snapshot!(
        "continue_preserves_error_exit_code",
        redact_error_output(&result.combined_output())
    );

    Ok(())
}
