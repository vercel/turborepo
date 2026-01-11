//! Port of turborepo-tests/integration/tests/run/profile.t
//!
//! Tests that --profile generates a valid JSON trace file.

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::TurboTestEnv;

/// Test that --profile generates a valid JSON trace file.
///
/// This verifies that turbo correctly generates a Chrome-compatible
/// trace file when the --profile flag is used.
#[tokio::test]
async fn test_profile_generates_valid_json_trace() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Run turbo with --profile flag
    let result = env
        .run_turbo(&["run", "build", "--profile=build.trace"])
        .await?;
    result.assert_success();

    // Verify the trace file was created
    assert!(
        env.file_exists("build.trace").await,
        "Profile trace file should be created"
    );

    // Read and parse the trace file as JSON
    let trace_content = env.read_file("build.trace").await?;
    let parsed: serde_json::Value =
        serde_json::from_str(&trace_content).expect("Profile trace should be valid JSON");

    // Basic validation - the trace should be an array (Chrome trace format)
    // or an object with a "traceEvents" field
    assert!(
        parsed.is_array() || parsed.get("traceEvents").is_some(),
        "Profile trace should be in Chrome trace format (array or object with traceEvents)"
    );

    Ok(())
}
