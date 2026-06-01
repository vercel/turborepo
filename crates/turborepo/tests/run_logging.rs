#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, run_turbo_with_env, setup};

fn assert_contains_in_order(output: &str, lines: &[&str]) {
    let mut offset = 0;
    for line in lines {
        let Some(index) = output[offset..].find(line) else {
            panic!("expected `{line}` after offset {offset}\n{output}");
        };
        offset += index + line.len();
    }
}

// --- log-order-stream.t ---
// Stream output is non-deterministic in ordering, so we check key lines.

#[test]
fn test_ordered_logging_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--log-order", "stream", "--force"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
    assert!(stdout.contains("0 cached, 2 total"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--log-order", "grouped", "--force"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packages in scope: my-app, util"));
    assert!(stdout.contains("Running build in 2 packages"));
    assert!(stdout.contains("Remote caching disabled"));
    assert_contains_in_order(
        &stdout,
        &[
            "my-app:build: cache bypass, force executing",
            "my-app:build: > build",
            "my-app:build: building",
            "my-app:build: done",
        ],
    );
    assert_contains_in_order(
        &stdout,
        &[
            "util:build: cache bypass, force executing",
            "util:build: > build",
            "util:build: building",
            "util:build: completed",
        ],
    );
    assert!(stdout.contains("2 successful, 2 total"));
    assert!(stdout.contains("0 cached, 2 total"));

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--force"],
        &[("GITHUB_ACTIONS", "1")],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("::group::my-app:build"));
    assert!(stdout.contains("::endgroup::"));
    assert!(stdout.contains("::group::util:build"));
    assert!(stdout.contains("2 successful, 2 total"));

    let output = run_turbo_with_env(
        tempdir.path(),
        &[
            "run",
            "build",
            "--force",
            "--log-prefix=task",
            "--filter=util",
        ],
        &[("GITHUB_ACTIONS", "1")],
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("::group::util:build"));
    assert!(stdout.contains("util:build: cache bypass"));
    assert!(stdout.contains("::endgroup::"));

    let output = run_turbo_with_env(tempdir.path(), &["run", "fail"], &[("GITHUB_ACTIONS", "1")]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("::error::"),
        "stderr should contain ::error:: annotation for GitHub Actions, got: {}",
        &stderr[..stderr.len().min(500)]
    );
}

// --- log-prefix.t ---

#[test]
fn test_log_prefix_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    assert!(stdout.contains("build-app-a"));
    assert!(!stdout.contains("app-a:build:"));

    // Check cached log file doesn't have prefixes
    let log = std::fs::read_to_string(tempdir.path().join("app-a/.turbo/turbo-build.log")).unwrap();
    assert!(log.contains("build-app-a"));
    assert!(!log.contains("app-a:build:"));

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs"));
    assert!(stdout.contains("FULL TURBO"));
    assert!(!stdout.contains("app-a:build:"));

    // Default prefix: should show prefixes
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("app-a:build: cache hit, replaying logs"));
    assert!(stdout.contains("app-a:build: build-app-a"));
}

// --- verbosity.t ---

#[test]
fn test_verbosity_flags() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "-v", "--filter=util", "--force"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("util:build: cache bypass, force executing bf1798d3e46e1b48"));
    assert!(stdout.contains("util:build: building"));

    let output = run_turbo(
        tempdir.path(),
        &["build", "-vv", "--filter=util", "--force"],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(combined.contains("[DEBUG]"));
}

// --- no-cache-and-no-output-logs.t ---

#[test]
fn test_no_cache_and_no_output_logs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--cache=local:,remote:",
            "--output-logs=none",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no caches are enabled") || stdout.contains("no caches are enabled"));
    assert!(stdout.contains("1 successful, 1 total"));
}

// --- errors-only.t ---

#[test]
fn test_errors_only_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", false).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Success: no task output shown
    assert!(!stdout.contains("build-app-a"));
    assert!(stdout.contains("1 successful, 1 total"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "builderror", "--output-logs=errors-only"],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Error: full output shown
    assert!(stdout.contains("error-builderror-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror"));

    // buildsuccess has outputLogs: "errors-only" in turbo.json
    let output = run_turbo(tempdir.path(), &["run", "buildsuccess"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("build-app-a"));
    assert!(stdout.contains("1 successful, 1 total"));

    // builderror2 has outputLogs: "errors-only" in turbo.json
    let output = run_turbo(tempdir.path(), &["run", "builderror2"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("error-builderror2-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror2"));

    // nocachebuild has cache:false in turbo.json
    let output = run_turbo(
        tempdir.path(),
        &["run", "nocachebuild", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Success with errors-only: task output should be suppressed
    assert!(!stdout.contains("nocachebuild-app-a"));
    assert!(stdout.contains("1 successful, 1 total"));

    // nocachebuilderror has cache:false in turbo.json and exits 1
    let output = run_turbo(
        tempdir.path(),
        &["run", "nocachebuilderror", "--output-logs=errors-only"],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Failure with errors-only: full output should be shown
    assert!(stdout.contains("nocachebuilderror-app-a"));
    assert!(stdout.contains("Failed:    app-a#nocachebuilderror"));
}

// --- errors-only-show-hash.t ---

#[test]
fn test_errors_only_show_hash_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "run_logging_errors_only_show_hash",
        "npm@10.5.0",
        false,
    )
    .unwrap();

    // Warm cache
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    assert!(stdout.contains("(only logging errors)"));

    // Cache hit
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs (no errors)"));

    let output = run_turbo(tempdir.path(), &["run", "builderror"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("error-builderror-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror"));
}

// --- full-cache-hit-output.t ---

#[test]
fn test_basic_monorepo_output_modes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Warm cache
    run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);

    // --output-logs=full: should show cached output
    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=full"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let cache_hit_count = stdout.matches("cache hit, replaying logs").count();
    assert_eq!(cache_hit_count, 2, "expected 2 cache hit messages");

    let building_count = stdout
        .lines()
        .filter(|l| l.ends_with(":build: building") || l.ends_with(":build: building\r"))
        .count();
    assert!(
        building_count >= 1,
        "expected build output to be shown on cache replay"
    );

    assert!(stdout.contains("FULL TURBO"));

    // --output-logs=hash-only
    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=hash-only"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let suppressed_count = stdout.matches("cache hit, suppressing logs").count();
    assert_eq!(suppressed_count, 2, "expected 2 suppressed log messages");

    // --output-logs=none
    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 cached, 2 total"));
    assert!(stdout.contains("FULL TURBO"));

    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Packages in scope"),
        "prelude should list packages in scope on stdout"
    );
    assert!(
        stdout.contains("Running build in"),
        "prelude should show running tasks on stdout"
    );
    assert!(
        stdout.contains("Remote caching"),
        "prelude should show remote cache status on stdout"
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Packages in scope"),
        "prelude should appear on stdout in --dry text mode"
    );
    assert!(
        stdout.contains("Remote caching"),
        "remote cache status should appear in --dry text mode"
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // stdout must be valid JSON — no prelude text mixed in.
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout should be valid JSON, got parse error: {:?}\nstdout: {}",
        parsed.err(),
        &stdout[..stdout.len().min(200)]
    );
    assert!(
        !stdout.contains("Packages in scope"),
        "prelude must not appear on stdout in --dry=json mode"
    );
}

#[test]
fn test_prelude_absent_from_graph_stdout() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "task_dependencies/topological",
        "npm@10.5.0",
        false,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("digraph {"),
        "stdout should contain DOT graph"
    );
    assert!(
        !stdout.contains("Packages in scope"),
        "prelude must not appear on stdout in --graph mode"
    );
    assert!(
        !stdout.contains("Remote caching"),
        "remote cache status must not appear on stdout in --graph mode"
    );
}

#[test]
fn test_prelude_single_package_format() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Running build"),
        "single-package prelude should show 'Running <task>'"
    );
    assert!(
        !stdout.contains("Packages in scope"),
        "single-package prelude must not show 'Packages in scope'"
    );
}
