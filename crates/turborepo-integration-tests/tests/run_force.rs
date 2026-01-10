//! Port of turborepo-tests/integration/tests/run/force.t
//!
//! Tests the interaction between TURBO_FORCE env var and --force flag.
//!
//! | env var | flag    | bypass? |
//! | ------- | ------- | ------- |
//! | true    | missing | yes     |
//! | true    | true    | yes     |
//! | true    | false   | no      |
//! | true    | novalue | yes     |
//! | false   | missing | no      |
//! | false   | true    | yes     |
//! | false   | false   | no      |
//! | false   | novalue | yes     |
//! | missing | missing | no      |
//! | missing | true    | yes     |
//! | missing | false   | no      |
//! | missing | novalue | yes     |

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{redact_output, TurboTestEnv};
use insta::assert_snapshot;
use test_case::test_case;

/// Base turbo args for all tests in this file
const BASE_ARGS: &[&str] = &["run", "build", "--output-logs=hash-only", "--filter=my-app"];

/// Represents the --force flag state
#[derive(Debug, Clone, Copy)]
enum ForceFlag {
    /// No --force flag provided
    Missing,
    /// --force (no value)
    NoValue,
    /// --force=true
    True,
    /// --force=false
    False,
}

impl ForceFlag {
    fn as_arg(&self) -> Option<&'static str> {
        match self {
            ForceFlag::Missing => None,
            ForceFlag::NoValue => Some("--force"),
            ForceFlag::True => Some("--force=true"),
            ForceFlag::False => Some("--force=false"),
        }
    }
}

/// Helper to set up a test environment with cache already populated.
///
/// This runs an initial build to populate the cache, so subsequent tests
/// can verify cache hit/bypass behavior.
async fn setup_env_with_cache() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;
    // Prime the cache with initial run
    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();
    Ok(env)
}

/// Run a force test scenario and return the redacted output.
async fn run_force_scenario(
    env: &TurboTestEnv,
    turbo_force_env: Option<&str>,
    force_flag: ForceFlag,
) -> Result<String> {
    let mut args: Vec<&str> = BASE_ARGS.to_vec();
    if let Some(flag) = force_flag.as_arg() {
        args.push(flag);
    }

    let result = match turbo_force_env {
        Some(value) => env.run_turbo_with_env(&args, &[("TURBO_FORCE", value)]).await?,
        None => env.run_turbo(&args).await?,
    };

    result.assert_success();
    Ok(redact_output(&result.combined_output()))
}


#[tokio::test]
async fn test_baseline_cache_miss() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();
    assert_snapshot!("baseline_cache_miss", redact_output(&result.combined_output()));

    Ok(())
}


#[test_case(ForceFlag::Missing, "env_true_flag_missing" ; "env_true_flag_missing_bypasses_cache")]
#[test_case(ForceFlag::True, "env_true_flag_true" ; "env_true_flag_true_bypasses_cache")]
#[test_case(ForceFlag::False, "env_true_flag_false" ; "env_true_flag_false_uses_cache")]
#[test_case(ForceFlag::NoValue, "env_true_flag_novalue" ; "env_true_flag_novalue_bypasses_cache")]
#[tokio::test]
async fn test_force_with_env_true(flag: ForceFlag, snapshot_name: &str) -> Result<()> {
    let env = setup_env_with_cache().await?;
    let output = run_force_scenario(&env, Some("true"), flag).await?;
    assert_snapshot!(snapshot_name, output);
    Ok(())
}


#[test_case(ForceFlag::Missing, "env_false_flag_missing" ; "env_false_flag_missing_uses_cache")]
#[test_case(ForceFlag::True, "env_false_flag_true" ; "env_false_flag_true_bypasses_cache")]
#[test_case(ForceFlag::False, "env_false_flag_false" ; "env_false_flag_false_uses_cache")]
#[test_case(ForceFlag::NoValue, "env_false_flag_novalue" ; "env_false_flag_novalue_bypasses_cache")]
#[tokio::test]
async fn test_force_with_env_false(flag: ForceFlag, snapshot_name: &str) -> Result<()> {
    let env = setup_env_with_cache().await?;
    let output = run_force_scenario(&env, Some("false"), flag).await?;
    assert_snapshot!(snapshot_name, output);
    Ok(())
}

// =============================================================================
// TURBO_FORCE not set (missing) scenarios
// =============================================================================

#[test_case(ForceFlag::Missing, "env_missing_flag_missing" ; "env_missing_flag_missing_uses_cache")]
#[test_case(ForceFlag::True, "env_missing_flag_true" ; "env_missing_flag_true_bypasses_cache")]
#[test_case(ForceFlag::False, "env_missing_flag_false" ; "env_missing_flag_false_uses_cache")]
#[test_case(ForceFlag::NoValue, "env_missing_flag_novalue" ; "env_missing_flag_novalue_bypasses_cache")]
#[tokio::test]
async fn test_force_with_env_missing(flag: ForceFlag, snapshot_name: &str) -> Result<()> {
    let env = setup_env_with_cache().await?;
    let output = run_force_scenario(&env, None, flag).await?;
    assert_snapshot!(snapshot_name, output);
    Ok(())
}
