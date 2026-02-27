mod common;

use common::{run_turbo, run_turbo_with_env, setup, turbo_output_filters};

// --- log-order-stream.t ---
// Stream output is non-deterministic in ordering, so we check key lines.

#[test]
fn test_log_order_stream_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--log-order", "stream", "--force"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
    assert!(stdout.contains("0 cached, 2 total"));
}

#[test]
fn test_log_order_stream_env_var() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--force"],
        &[("TURBO_LOG_ORDER", "stream")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
}

#[test]
fn test_log_order_stream_flag_wins_over_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--log-order", "stream", "--force"],
        &[("TURBO_LOG_ORDER", "grouped")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 successful, 2 total"));
}

// --- log-order-grouped.t ---
// Grouped output IS deterministic.

#[test]
fn test_log_order_grouped_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--log-order", "grouped", "--force"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("log_order_grouped_flag", stdout.to_string());
    });
}

#[test]
fn test_log_order_grouped_env_var() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--force"],
        &[("TURBO_LOG_ORDER", "grouped")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("log_order_grouped_env_var", stdout.to_string());
    });
}

#[test]
fn test_log_order_grouped_flag_wins_over_env() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--log-order", "grouped", "--force"],
        &[("TURBO_LOG_ORDER", "stream")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("log_order_grouped_flag_wins", stdout.to_string());
    });
}

// --- log-order-github.t ---
// Tests ::group::/::endgroup:: output when GITHUB_ACTIONS=1

#[test]
fn test_log_order_github_actions() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

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
}

#[test]
fn test_log_order_github_actions_with_task_prefix() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

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
}

#[test]
fn test_log_order_github_actions_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "ordered", "npm@10.5.0", true).unwrap();

    let output = run_turbo_with_env(tempdir.path(), &["run", "fail"], &[("GITHUB_ACTIONS", "1")]);

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The error task should fail and output should contain the failure
    assert!(stdout.contains("util#fail") || stdout.contains("util:fail"));
    assert!(stdout.contains("failing"));
}

// --- log-prefix.t ---

#[test]
fn test_log_prefix_none_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // No prefix on cache miss line
    assert!(stdout.contains("cache miss, executing 612027951a2848ce"));
    assert!(stdout.contains("build-app-a"));
    assert!(stdout.contains("0 cached, 1 total"));
}

#[test]
fn test_log_prefix_none_cached_log_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // First run to populate cache
    run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);

    // Check cached log file doesn't have prefixes
    let log = std::fs::read_to_string(tempdir.path().join("app-a/.turbo/turbo-build.log")).unwrap();
    assert!(log.contains("build-app-a"));
    assert!(!log.contains("app-a:build:"));
}

#[test]
fn test_log_prefix_none_cache_hit() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // First run
    run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);

    // Second run: cache hit, still no prefixes
    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs 612027951a2848ce"));
    assert!(stdout.contains("1 cached, 1 total"));
    assert!(stdout.contains("FULL TURBO"));
}

#[test]
fn test_log_prefix_default_shows_prefixes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // Warm cache
    run_turbo(tempdir.path(), &["run", "build", "--log-prefix=none"]);

    // Default prefix: should show prefixes
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("app-a:build: cache hit, replaying logs 612027951a2848ce"));
    assert!(stdout.contains("app-a:build: build-app-a"));
}

#[test]
fn test_log_prefix_bogus_value() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix=blah"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value 'blah' for '--log-prefix"));
}

#[test]
fn test_log_prefix_missing_value() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--log-prefix"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("a value is required for '--log-prefix"));
}

// --- verbosity.t ---

#[test]
fn test_verbosity_v_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "-v", "--filter=util", "--force"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("util:build: cache bypass, force executing bf1798d3e46e1b48"));
    assert!(stdout.contains("util:build: building"));
}

#[test]
fn test_verbosity_1_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--verbosity=1", "--filter=util", "--force"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("util:build: cache bypass, force executing bf1798d3e46e1b48"));
}

#[test]
fn test_verbosity_vv_has_debug() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

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

#[test]
fn test_verbosity_2_has_debug() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--verbosity=2", "--filter=util", "--force"],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(combined.contains("[DEBUG]"));
}

#[test]
fn test_verbosity_v_and_verbosity_conflict() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "-v", "--verbosity=1"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be used with"));
}

// --- no-cache-and-no-output-logs.t ---

#[test]
fn test_no_cache_and_no_output_logs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

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
fn test_errors_only_flag_success() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Success: no task output shown
    assert!(!stdout.contains("build-app-a"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_errors_only_turbo_json_success() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // buildsuccess has outputLogs: "errors-only" in turbo.json
    let output = run_turbo(tempdir.path(), &["run", "buildsuccess"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("build-app-a"));
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_errors_only_flag_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "builderror", "--output-logs=errors-only"],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Error: full output shown
    assert!(stdout.contains("error-builderror-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror"));
}

#[test]
fn test_errors_only_turbo_json_error() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "run_logging", "npm@10.5.0", true).unwrap();

    // builderror2 has outputLogs: "errors-only" in turbo.json
    let output = run_turbo(tempdir.path(), &["run", "builderror2"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("error-builderror2-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror2"));
}

// --- errors-only-show-hash.t ---

#[test]
fn test_errors_only_show_hash_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "run_logging_errors_only_show_hash",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    assert!(stdout.contains("(only logging errors)"));
}

#[test]
fn test_errors_only_show_hash_cache_hit() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "run_logging_errors_only_show_hash",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    // Warm cache
    run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );

    // Cache hit
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=errors-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs (no errors)"));
}

#[test]
fn test_errors_only_show_hash_error_shows_full_logs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "run_logging_errors_only_show_hash",
        "npm@10.5.0",
        true,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "builderror"]);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("error-builderror-app-a"));
    assert!(stdout.contains("Failed:    app-a#builderror"));
}

// --- full-cache-hit-output.t ---

#[test]
fn test_full_cache_hit_output() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

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
}
