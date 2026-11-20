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
    git(
        dir,
        &["commit", "-am", "turbo.json", "--quiet", "--allow-empty"],
    );
}

#[test]
fn test_excluded_dependency_does_not_execute() {
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path();
    setup_with_turbo_json(root, r#"{"tasks":{"build":{"dependsOn":["^build"]}}}"#);

    // my-app -> util -> another: excluding the leaf must not run its script,
    // while util still runs before my-app. No experimental command flag is set.
    for (directory, dependency) in [
        ("packages/another", None),
        ("packages/util", Some("another")),
        ("apps/my-app", Some("util")),
    ] {
        let path = root.join(directory);
        let mut package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path.join("package.json")).unwrap()).unwrap();
        package["scripts"]["build"] = serde_json::json!("node build.js");
        if let Some(dependency) = dependency {
            package["dependencies"] = serde_json::json!({dependency: "*"});
        }
        fs::write(path.join("package.json"), package.to_string()).unwrap();
        let check_order = if directory == "apps/my-app" {
            "if (!fs.existsSync('../../packages/util/ran.txt')) throw new Error('util must build \
             first');"
        } else {
            ""
        };
        fs::write(
            path.join("build.js"),
            format!(
                "const fs = require('fs'); {check_order} fs.writeFileSync('ran.txt', 'built');"
            ),
        )
        .unwrap();
    }
    let local_config = root.join("packages/another/turbo.json");
    for (definition, should_run) in [
        (serde_json::json!({"extends": false}), false),
        (serde_json::json!({"extends": false, "cache": false}), true),
    ] {
        fs::write(
            &local_config,
            serde_json::json!({"extends": ["//"], "tasks": {"build": definition}}).to_string(),
        )
        .unwrap();
        for selector in [None, Some("--filter=my-app")] {
            let mut args = vec!["run", "build", "--force"];
            args.extend(selector);
            let output = run_turbo(root, &args);
            assert!(output.status.success(), "{}", combined_output(&output));
            for directory in ["packages/another", "packages/util", "apps/my-app"] {
                let marker = root.join(directory).join("ran.txt");
                assert_eq!(
                    marker.exists(),
                    directory != "packages/another" || should_run
                );
                if marker.exists() {
                    fs::remove_file(marker).unwrap();
                }
            }
        }
    }
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
