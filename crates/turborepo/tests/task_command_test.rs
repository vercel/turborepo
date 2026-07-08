#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! End-to-end tests for the task `command` override
//! (`futureFlags.experimentalTaskCommand`): the argv in turbo.json replaces
//! the toolchain's own resolution.

mod common;

use std::fs;

use common::{combined_output, git, run_turbo, setup};

fn setup_with_turbo_json(dir: &std::path::Path, turbo_json: &str) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();
    fs::write(dir.join("turbo.json"), turbo_json).unwrap();
    git(dir, &["commit", "-am", "turbo.json", "--quiet", "--allow-empty"]);
}

#[test]
fn test_command_replaces_script() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": {
                "build": { "outputs": [] },
                "util#build": { "command": ["echo", "from-override"] }
            }
        }"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(output.status.success(), "run failed: {combined}");
    assert!(
        combined.contains("from-override"),
        "override argv should run: {combined}"
    );
    assert!(
        !combined.contains("echo building"),
        "the package.json script must not run: {combined}"
    );
}

#[test]
fn test_command_defines_task_without_script() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": {
                "util#saluton": { "command": ["echo", "saluton-mondo"] }
            }
        }"#,
    );

    // No package.json script named `saluton` exists anywhere: the command
    // is the definition.
    let output = run_turbo(tempdir.path(), &["run", "saluton", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(output.status.success(), "run failed: {combined}");
    assert!(
        combined.contains("saluton-mondo"),
        "command-only task should execute: {combined}"
    );
}

#[test]
fn test_command_opt_out_is_a_no_op() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": {
                "build": { "outputs": [] },
                "util#build": { "command": null }
            }
        }"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(output.status.success(), "run failed: {combined}");
    assert!(
        !combined.contains("building"),
        "opted-out task must not run its script: {combined}"
    );
}

#[test]
fn test_unscoped_command_shadowed_by_script() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": {
                "build": { "command": ["echo", "from-default"], "outputs": [] }
            }
        }"#,
    );

    // util authors a `build` script; the unscoped default must lose to it.
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(output.status.success(), "run failed: {combined}");
    assert!(
        combined.contains("building"),
        "package-authored script wins over the unscoped default: {combined}"
    );
    assert!(
        !combined.contains("from-default"),
        "unscoped default must not run where a script exists: {combined}"
    );
}

#[test]
fn test_per_toolchain_map_fans_out() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "futureFlags": { "experimentalTaskCommand": true },
            "tasks": {
                "greet": { "command": { "javascript": ["echo", "from-map"] } }
            }
        }"#,
    );

    // The javascript key grants `greet` to every JS package.
    let output = run_turbo(tempdir.path(), &["run", "greet", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(output.status.success(), "run failed: {combined}");
    assert!(
        combined.contains("from-map"),
        "map default should apply to JS packages: {combined}"
    );
}

#[test]
fn test_command_requires_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_with_turbo_json(
        tempdir.path(),
        r#"{
            "tasks": {
                "util#build": { "command": ["echo", "nope"] }
            }
        }"#,
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(!output.status.success(), "should fail without the flag");
    assert!(
        combined.contains("experimentalTaskCommand"),
        "the error should point at the flag: {combined}"
    );
}

#[test]
fn test_command_change_invalidates_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    let turbo_json = |arg: &str| {
        format!(
            r#"{{
                "futureFlags": {{ "experimentalTaskCommand": true }},
                "tasks": {{
                    "util#build": {{ "command": ["echo", "{arg}"], "outputs": [] }}
                }}
            }}"#
        )
    };
    setup_with_turbo_json(tempdir.path(), &turbo_json("first"));

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    assert!(output.status.success());

    // Same command: cache hit.
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("cache hit"),
        "unchanged command should hit cache: {combined}"
    );

    // Changed argv: the task hash must change.
    fs::write(tempdir.path().join("turbo.json"), turbo_json("second")).unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "change", "--quiet", "--allow-empty"],
    );
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=util"]);
    let combined = combined_output(&output);
    assert!(
        combined.contains("cache miss") && combined.contains("second"),
        "changed command must miss cache and run: {combined}"
    );
}
