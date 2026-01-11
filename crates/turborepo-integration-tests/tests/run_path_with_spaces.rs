//! Port of turborepo-tests/integration/tests/run/path-with-spaces.t
//!
//! Tests that turbo can hash files with spaces in their names.

// Skip on Windows - npm not found in test harness PATH on Windows CI
#![cfg(all(feature = "integration-tests", not(windows)))]

mod common;

use anyhow::Result;
use common::TurboTestEnv;

/// Test that turbo can handle files with spaces in their names.
///
/// This verifies that the file hashing system properly handles paths
/// containing spaces, which can be problematic on some systems.
#[tokio::test]
async fn test_path_with_spaces() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Create a file with spaces in the name (mimics the prysk test)
    env.write_file("packages/util/with spaces.txt", "new file")
        .await?;

    // Run a dry run to verify turbo can hash the file with spaces
    let result = env
        .run_turbo(&["run", "build", "--dry", "-F", "util"])
        .await?;
    result.assert_success();

    // Verify the output contains the expected input files count
    // The util package has package.json + "with spaces.txt" = 2 files
    assert!(
        result.stdout_contains("Inputs Files Considered"),
        "Expected 'Inputs Files Considered' in output.\nstdout: {}",
        result.stdout
    );

    // Check that 2 files were considered (package.json + with spaces.txt)
    assert!(
        result.stdout_contains("= 2"),
        "Expected '= 2' input files.\nstdout: {}",
        result.stdout
    );

    Ok(())
}
