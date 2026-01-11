//! Port of turborepo-tests/integration/tests/run/big-status.t
//!
//! Tests that turbo can handle a large git status with many files that have
//! spaces in their names. This verifies:
//! 1. Git can track 10,000 files with spaces in their names
//! 2. Turbo can hash all these files correctly
//! 3. The input files count is correct (original files + new files)

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;
use regex::Regex;

/// Test that turbo can handle a large git status with files containing spaces.
///
/// This test creates 10,000 files with spaces in their names and verifies that
/// turbo can properly hash them during a dry run.
#[tokio::test]
async fn test_big_status_with_spaces_in_filenames() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Create 10,000 files with spaces in their names in packages/util/
    // This tests that turbo can handle large git status output with special
    // characters
    for i in 1..=10000 {
        let filename = format!("packages/util/with spaces {}.txt", i);
        env.write_file(&filename, "new file").await?;
    }

    // Verify git status shows the expected number of files with spaces
    let git_result = env.exec(&["git", "status"]).await?;
    let lines_with_spaces: usize = git_result
        .stdout
        .lines()
        .filter(|line| line.contains("with spaces"))
        .count();
    assert_eq!(
        lines_with_spaces, 10000,
        "Expected 10000 files with 'with spaces' in git status, found {}",
        lines_with_spaces
    );

    // Run turbo dry-run to verify we can hash files with spaces
    let result = env
        .run_turbo(&["run", "build", "--dry", "-F", "util"])
        .await?;
    result.assert_success();

    // Extract the "Inputs Files Considered" line and verify the count
    let re = Regex::new(r"Inputs Files Considered\s+=\s+(\d+)").unwrap();
    let output = result.combined_output();
    let captures = re
        .captures(&output)
        .expect("Expected 'Inputs Files Considered' in output");
    let input_files_count: u32 = captures[1].parse().expect("Expected a number");

    // Should be 10001: original file(s) in util package + 10000 new files
    assert_eq!(
        input_files_count, 10001,
        "Expected 10001 input files considered, got {}",
        input_files_count
    );

    // Also snapshot the full output (with redactions) for regression testing
    assert_snapshot!("big_status_dry_run", redact_output(&output));

    Ok(())
}
