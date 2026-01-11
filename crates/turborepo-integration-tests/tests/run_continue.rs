//! Port of turborepo-tests/integration/tests/run/continue.t
//!
//! Tests the --continue flag behavior when tasks fail:
//! - Without --continue: stops at first error
//! - With --continue: continues past errors
//! - With --continue=dependencies-successful: only runs tasks whose deps
//!   succeeded
//!
//! Uses the monorepo_dependency_error fixture which has:
//! - base-lib: builds successfully, no deps
//! - some-lib: depends on base-lib, fails with exit 2
//! - my-app: depends on some-lib, builds successfully
//! - yet-another-lib: depends on base-lib, builds successfully
//! - other-app: depends on yet-another-lib, fails with exit 3
#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;
use regex::Regex;

/// Additional redaction for paths in error messages.
/// Normalizes dynamic paths for stable snapshots.
/// Note: Temp paths are already redacted by common::redact_output.
fn redact_paths(output: &str) -> String {
    // Redact npm executable paths (e.g., /Users/.../fnm_multishells/.../npm)
    let npm_path_re =
        Regex::new(r"(?:[A-Za-z]:)?(?:/[^/\s]+)+/npm(?:\.cmd)?").expect("Invalid npm path regex");
    let output = npm_path_re.replace_all(output, "[NPM]");

    // Redact command paths like "command (/path/to/...)"
    let cmd_path_re = Regex::new(r"command \([^)]+\)").expect("Invalid cmd path regex");
    let output = cmd_path_re.replace_all(&output, "command ([PATH])");

    output.into_owned()
}

/// Filter out the lockfile warning from the output.
/// This warning is expected since we don't run npm install in tests.
fn filter_lockfile_warning(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let mut result = Vec::new();
    let mut skip_until_empty = false;

    for line in lines {
        if line.contains("WARNING") && line.contains("package-lock.json") {
            skip_until_empty = true;
            continue;
        }
        if skip_until_empty {
            if line.trim().is_empty() {
                skip_until_empty = false;
            }
            continue;
        }
        result.push(line);
    }

    result.join("\n")
}

/// Apply both standard and path redactions, and filter lockfile warning.
fn redact_continue_output(output: &str) -> String {
    let output = filter_lockfile_warning(&redact_paths(&redact_output(output)));
    normalize_task_order(&output)
}

/// Normalize task output order for deterministic snapshots.
///
/// Turbo runs tasks in parallel, so the order of output blocks can vary between
/// runs. This function collects all lines belonging to each task and outputs
/// them in sorted order by task name.
fn normalize_task_order(output: &str) -> String {
    use std::collections::BTreeMap;

    let lines: Vec<&str> = output.lines().collect();

    // Find where task output starts (after header) and ends (before summary)
    let task_start = lines
        .iter()
        .position(|line| {
            // Task output lines start with package:task pattern
            line.contains(":build:") || line.contains("#build:")
        })
        .unwrap_or(0);

    let task_end = lines
        .iter()
        .rposition(|line| line.contains(":build:") || line.contains("#build:"))
        .map(|i| i + 1)
        .unwrap_or(lines.len());

    // Extract sections
    let header = &lines[..task_start];
    let task_section = &lines[task_start..task_end];
    let footer = &lines[task_end..];

    // Collect all lines for each task
    let mut task_lines: BTreeMap<String, Vec<&str>> = BTreeMap::new();

    for line in task_section {
        // Extract task name from line
        let task_name = extract_task_name(line).unwrap_or_else(|| "zzz_unknown".to_string());
        task_lines.entry(task_name).or_default().push(line);
    }

    // Reassemble output with tasks in sorted order
    let mut result: Vec<&str> = header.to_vec();
    for (_task, lines) in task_lines {
        result.extend(lines);
    }
    result.extend(footer);

    result.join("\n")
}

/// Extract task name from a log line.
/// Returns Some("task-name") for lines like "task-name:build: ..." or
/// "task-name#build: ..."
fn extract_task_name(line: &str) -> Option<String> {
    // Match patterns like "some-lib:build:" or "other-app#build:"
    let task_re = Regex::new(r"^([a-zA-Z0-9_-]+)[::#]build").ok()?;
    task_re
        .captures(line)
        .map(|c| c.get(1).unwrap().as_str().to_string())
}

/// Set up the test environment with monorepo_dependency_error fixture.
/// Adds packageManager field to match the prysk test setup.
async fn setup_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("monorepo_dependency_error").await?;

    // Add packageManager field to package.json (matches setup_package_manager.sh)
    let pkg_json = env.read_file("package.json").await?;
    let mut pkg: serde_json::Value = serde_json::from_str(&pkg_json)?;
    pkg["packageManager"] = serde_json::Value::String("npm@10.5.0".to_string());
    env.write_file("package.json", &serde_json::to_string_pretty(&pkg)?)
        .await?;

    env.setup_git().await?;
    Ok(env)
}

#[tokio::test]
async fn test_without_continue_stops_at_first_error() -> Result<()> {
    let env = setup_env().await?;

    let result = env
        .run_turbo(&["run", "build", "--filter", "my-app..."])
        .await?;

    result.assert_failure();
    assert_snapshot!(
        "without_continue",
        redact_continue_output(&result.combined_output())
    );

    Ok(())
}

#[tokio::test]
async fn test_without_continue_errors_only() -> Result<()> {
    let env = setup_env().await?;

    // First run to populate cache for base-lib
    env.run_turbo(&["run", "build", "--filter", "my-app..."])
        .await?;

    // Second run with errors-only should show cached base-lib and failed some-lib
    let result = env
        .run_turbo(&[
            "run",
            "build",
            "--output-logs=errors-only",
            "--filter",
            "my-app...",
        ])
        .await?;

    result.assert_failure();
    assert_snapshot!(
        "without_continue_errors_only",
        redact_continue_output(&result.combined_output())
    );

    Ok(())
}

#[tokio::test]
async fn test_with_continue_continues_past_errors() -> Result<()> {
    let env = setup_env().await?;

    // First run to populate cache for base-lib
    env.run_turbo(&["run", "build", "--filter", "my-app..."])
        .await?;

    // Run with --continue should continue past some-lib failure and run my-app
    let result = env
        .run_turbo(&[
            "run",
            "build",
            "--output-logs=errors-only",
            "--filter",
            "my-app...",
            "--continue",
        ])
        .await?;

    result.assert_failure();
    assert_snapshot!(
        "with_continue",
        redact_continue_output(&result.combined_output())
    );

    Ok(())
}

#[tokio::test]
async fn test_with_continue_dependencies_successful() -> Result<()> {
    let env = setup_env().await?;

    // First run to populate cache (with --continue to finish all tasks even if
    // some fail, and --concurrency=1 for determinism)
    env.run_turbo(&["run", "build", "--continue", "--concurrency=1"])
        .await?;

    // Run with --continue=dependencies-successful
    // This should:
    // - Run base-lib (success, cached)
    // - Run some-lib (fails)
    // - Skip my-app (because some-lib failed)
    // - Run yet-another-lib (success, cached)
    // - Run other-app (fails)
    //
    // We use --concurrency=1 to ensure deterministic task ordering for snapshot
    // testing
    let result = env
        .run_turbo(&[
            "run",
            "build",
            "--output-logs=errors-only",
            "--continue=dependencies-successful",
            "--concurrency=1",
        ])
        .await?;

    result.assert_failure();
    assert_snapshot!(
        "with_continue_dependencies_successful",
        redact_continue_output(&result.combined_output())
    );

    Ok(())
}
