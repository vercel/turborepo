//! Port of turborepo-tests/integration/tests/run/graph.t
//!
//! Tests the --graph flag for outputting task graph in various formats.
//!
//! | format      | output location | expected content         |
//! | ----------- | --------------- | ------------------------ |
//! | (none)      | stdout          | DOT format               |
//! | .dot        | file            | DOT format               |
//! | .html       | file            | HTML with DOCTYPE        |
//! | .mermaid    | file            | Mermaid flowchart format |
//! | .mdx        | error           | Invalid extension error  |

// Skip on Windows - npm not found in test harness PATH on Windows CI
#![cfg(all(feature = "integration-tests", not(windows)))]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;
use regex::Regex;

/// Base turbo args for all tests in this file
const BASE_ARGS: &[&str] = &["build", "-F", "my-app"];

/// The fixture used by all graph tests
const FIXTURE: &str = "task_dependencies/topological";

/// Default package manager for tests (matches prysk setup)
const PACKAGE_MANAGER: &str = "npm@10.5.0";

/// Set up a test environment with the topological fixture.
async fn setup_env() -> Result<TurboTestEnv> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture(FIXTURE).await?;

    // Add packageManager field to package.json (required by turbo)
    let package_json = env.read_file("package.json").await?;
    let updated = package_json.trim_end().trim_end_matches('}');
    let new_package_json = format!(
        "{},\n  \"packageManager\": \"{}\"\n}}\n",
        updated, PACKAGE_MANAGER
    );
    env.write_file("package.json", &new_package_json).await?;

    env.setup_git().await?;
    Ok(env)
}

#[tokio::test]
async fn test_graph_to_stdout() -> Result<()> {
    let env = setup_env().await?;

    let mut args = BASE_ARGS.to_vec();
    args.push("--graph");

    let result = env.run_turbo(&args).await?;
    result.assert_success();

    assert_snapshot!("graph_stdout", result.stdout);

    Ok(())
}

#[tokio::test]
async fn test_graph_to_dot_file() -> Result<()> {
    let env = setup_env().await?;

    let mut args = BASE_ARGS.to_vec();
    args.push("--graph=graph.dot");

    let result = env.run_turbo(&args).await?;
    result.assert_success();

    // Verify the success message mentions the file
    assert!(
        result.stdout.contains("graph.dot"),
        "Expected stdout to mention graph.dot, got: {}",
        result.stdout
    );

    // Read the generated file and verify DOT format content
    let dot_content = env.read_file("graph.dot").await?;

    // Verify the DOT file contains the expected edges
    assert!(
        dot_content.contains(r#""[root] my-app#build" -> "[root] util#build""#),
        "Expected DOT file to contain my-app#build -> util#build edge"
    );
    assert!(
        dot_content.contains(r#""[root] util#build" -> "[root] ___ROOT___""#),
        "Expected DOT file to contain util#build -> ___ROOT___ edge"
    );

    Ok(())
}

#[tokio::test]
async fn test_graph_to_html_file() -> Result<()> {
    let env = setup_env().await?;

    let mut args = BASE_ARGS.to_vec();
    args.push("--graph=graph.html");

    let result = env.run_turbo(&args).await?;
    result.assert_success();

    // Verify the success message mentions the file
    assert!(
        result.stdout.contains("graph.html"),
        "Expected stdout to mention graph.html, got: {}",
        result.stdout
    );

    // Read the generated file and verify HTML format
    let html_content = env.read_file("graph.html").await?;

    // Verify the HTML file contains DOCTYPE (case-insensitive check)
    assert!(
        html_content.contains("DOCTYPE") || html_content.contains("doctype"),
        "Expected HTML file to contain DOCTYPE declaration"
    );

    Ok(())
}

#[tokio::test]
async fn test_graph_to_mermaid_file() -> Result<()> {
    let env = setup_env().await?;

    let mut args = BASE_ARGS.to_vec();
    args.push("--graph=graph.mermaid");

    let result = env.run_turbo(&args).await?;
    result.assert_success();

    // Verify the success message mentions the file
    assert!(
        result.stdout.contains("graph.mermaid"),
        "Expected stdout to mention graph.mermaid, got: {}",
        result.stdout
    );

    // Read the generated file and verify Mermaid format
    let mermaid_content = env.read_file("graph.mermaid").await?;

    // Verify the Mermaid file starts with graph TD
    assert!(
        mermaid_content.starts_with("graph TD"),
        "Expected Mermaid file to start with 'graph TD', got: {}",
        mermaid_content.lines().next().unwrap_or("")
    );

    // Verify it contains the expected nodes (with randomized IDs)
    // Pattern: XXXX("my-app#build") --> YYYY("util#build")
    let node_pattern = Regex::new(r#"[A-Z]{4}\("my-app#build"\) --> [A-Z]{4}\("util#build"\)"#)
        .expect("Invalid regex");
    assert!(
        node_pattern.is_match(&mermaid_content),
        "Expected Mermaid file to contain my-app#build --> util#build edge"
    );

    let root_pattern = Regex::new(r#"[A-Z]{4}\("util#build"\) --> [A-Z]{4}\("___ROOT___"\)"#)
        .expect("Invalid regex");
    assert!(
        root_pattern.is_match(&mermaid_content),
        "Expected Mermaid file to contain util#build --> ___ROOT___ edge"
    );

    Ok(())
}

#[tokio::test]
async fn test_graph_invalid_extension_error() -> Result<()> {
    let env = setup_env().await?;

    let mut args = BASE_ARGS.to_vec();
    args.push("--graph=graph.mdx");

    let result = env.run_turbo(&args).await?;
    result.assert_failure();

    // Verify the error message mentions the invalid extension
    assert_snapshot!("graph_invalid_extension", redact_output(&result.stderr));

    Ok(())
}
