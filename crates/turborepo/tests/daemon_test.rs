mod common;

use common::setup;

fn run_daemon_status(dir: &std::path::Path, env_val: &str, extra_args: &[&str]) -> String {
    let config_dir = tempfile::tempdir().unwrap();
    let mut cmd = assert_cmd::Command::cargo_bin("turbo").unwrap();
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_LOG_VERBOSITY", env_val)
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .args(["daemon", "status"])
        .current_dir(dir);
    for arg in extra_args {
        cmd.arg(arg);
    }
    let output = cmd.output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn test_log_verbosity_debug() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_daemon_status(tempdir.path(), "debug", &[]);
    assert!(output.contains("[DEBUG]"), "expected [DEBUG] in output");
    assert!(
        output.contains("daemon is not running"),
        "expected daemon not running message"
    );
}

#[test]
fn test_v_flag_overrides_global_log_verbosity() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_daemon_status(tempdir.path(), "debug", &["-v"]);
    assert!(
        !output.contains("[DEBUG]"),
        "-v should override global debug setting"
    );
    assert!(
        output.contains("daemon is not running"),
        "expected daemon not running message"
    );
}

#[test]
fn test_package_specific_verbosity_preserved_with_v_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_daemon_status(tempdir.path(), "turborepo_daemon=debug", &["-v"]);
    assert!(
        output.contains("[DEBUG]"),
        "package-specific verbosity should be preserved with -v"
    );
    assert!(
        output.contains("daemon is not running"),
        "expected daemon not running message"
    );
}
