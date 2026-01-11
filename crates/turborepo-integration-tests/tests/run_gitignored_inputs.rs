//! Port of turborepo-tests/integration/tests/run/gitignored-inputs.t
//!
//! Tests that gitignored files explicitly listed in task inputs are still
//! tracked for cache purposes. When a gitignored file is specified as an input,
//! turbo should:
//! 1. Include the file in the task's inputs hash
//! 2. Trigger a cache miss when the file content changes

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Turbo config that sets internal.txt as an input for the build task
const GITIGNORED_INPUTS_TURBO_JSON: &str = r#"{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": {
      "inputs": ["internal.txt"]
    }
  }
}
"#;

/// Base turbo args for running the util package build
const BASE_ARGS: &[&str] = &[
    "run",
    "build",
    "--filter=util",
    "--output-logs=hash-only",
    "--summarize",
];

/// Helper to set up the test environment with gitignored-inputs config.
///
/// This sets up:
/// 1. basic_monorepo fixture
/// 2. Custom turbo.json with internal.txt as input
/// 3. packages/util/internal.txt with initial content
/// 4. .gitignore entry for packages/util/internal.txt
/// 5. Git commit of all changes
async fn setup_gitignored_inputs_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    // Replace turbo.json with gitignored-inputs config
    env.write_file("turbo.json", GITIGNORED_INPUTS_TURBO_JSON)
        .await?;

    // Create internal.txt for the util package
    env.write_file("packages/util/internal.txt", "hello world\n")
        .await?;

    // Add internal.txt to gitignore
    let gitignore_content = env.read_file(".gitignore").await.unwrap_or_default();
    let new_gitignore = format!("{}\npackages/util/internal.txt\n", gitignore_content);
    env.write_file(".gitignore", &new_gitignore).await?;

    // Commit the changes
    env.git_commit("add internal.txt and update turbo.json")
        .await?;

    Ok(env)
}

/// Helper to extract the internal.txt input hash from the run summary.
///
/// Parses the summary JSON to find the util#build task and extracts
/// the hash for internal.txt from its inputs.
async fn get_internal_txt_hash(env: &TurboTestEnv) -> Result<String> {
    // Find the summary JSON file
    let summary_dir = env.workspace_path().join(".turbo/runs");
    let mut entries = tokio::fs::read_dir(&summary_dir).await?;

    let mut summary_path = None;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            summary_path = Some(path);
            break;
        }
    }

    let summary_path = summary_path.ok_or_else(|| anyhow::anyhow!("No summary JSON found"))?;
    let content = tokio::fs::read_to_string(&summary_path).await?;
    let summary: serde_json::Value = serde_json::from_str(&content)?;

    // Find the util#build task
    let tasks = summary["tasks"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No tasks array in summary"))?;

    let util_build = tasks
        .iter()
        .find(|t| t["taskId"].as_str() == Some("util#build"))
        .ok_or_else(|| anyhow::anyhow!("util#build task not found"))?;

    // Extract the internal.txt hash
    let hash = util_build["inputs"]["internal.txt"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("internal.txt not found in inputs"))?;

    Ok(hash.to_string())
}

/// Helper to clean up the .turbo/runs directory.
async fn cleanup_runs_dir(env: &TurboTestEnv) -> Result<()> {
    let runs_dir = env.workspace_path().join(".turbo/runs");
    if runs_dir.exists() {
        tokio::fs::remove_dir_all(&runs_dir).await?;
    }
    Ok(())
}

#[tokio::test]
async fn test_gitignored_input_first_run_is_cache_miss() -> Result<()> {
    let env = setup_gitignored_inputs_env().await?;

    // First run should be a cache miss
    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();

    // Filter output to just the cache line for util:build
    let output = result.combined_output();
    let cache_line = output
        .lines()
        .find(|line| line.contains("util:build: cache"))
        .unwrap_or("");

    assert_snapshot!("first_run_cache_miss", redact_output(cache_line));

    Ok(())
}

#[tokio::test]
async fn test_gitignored_input_is_in_summary() -> Result<()> {
    let env = setup_gitignored_inputs_env().await?;

    // Run turbo to generate summary
    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();

    // Verify internal.txt is tracked in the summary
    let hash = get_internal_txt_hash(&env).await?;

    // The hash should be the SHA1 of "hello world\n"
    // 3b18e512dba79e4c8300dd08aeb37f8e728b8dad
    assert_eq!(
        hash, "3b18e512dba79e4c8300dd08aeb37f8e728b8dad",
        "internal.txt should have correct hash in summary"
    );

    Ok(())
}

#[tokio::test]
async fn test_gitignored_input_change_triggers_cache_miss() -> Result<()> {
    let env = setup_gitignored_inputs_env().await?;

    // First run to populate cache
    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();

    // Get the first hash
    let first_hash = get_internal_txt_hash(&env).await?;

    // Clean up runs directory so we can find the second summary easily
    cleanup_runs_dir(&env).await?;

    // Modify the gitignored file
    env.write_file("packages/util/internal.txt", "hello world\nchanged!\n")
        .await?;

    // Second run should be a cache miss because the file content changed
    let result = env.run_turbo(BASE_ARGS).await?;
    result.assert_success();

    // Verify the hash changed
    let second_hash = get_internal_txt_hash(&env).await?;

    assert_ne!(
        first_hash, second_hash,
        "Hash should change when gitignored input file changes"
    );

    // The new hash should be for "hello world\nchanged!\n"
    // fe9ca9502b0cfe311560aa43d953a88b112609ce
    assert_eq!(
        second_hash, "fe9ca9502b0cfe311560aa43d953a88b112609ce",
        "Updated internal.txt should have correct hash"
    );

    // Verify it was a cache miss (not a hit from the previous run)
    let output = result.combined_output();
    let cache_line = output
        .lines()
        .find(|line| line.contains("util:build: cache"))
        .unwrap_or("");

    assert_snapshot!(
        "second_run_after_change_cache_miss",
        redact_output(cache_line)
    );

    Ok(())
}
