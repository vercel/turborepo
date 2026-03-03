mod common;

use std::fs;

use common::{run_turbo_with_env, setup};

fn setup_otel_fixture(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Enable experimentalObservability by inserting futureFlags after the opening
    // brace
    let turbo_json = dir.join("turbo.json");
    let contents = fs::read_to_string(&turbo_json).unwrap();
    let new_contents = contents.replacen(
        "{",
        "{\n  \"futureFlags\": {\"experimentalObservability\": true},",
        1,
    );
    fs::write(&turbo_json, new_contents).unwrap();
}

#[test]
fn test_otel_env_vars_do_not_break_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_otel_fixture(tempdir.path());

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[
            ("TURBO_EXPERIMENTAL_OTEL_ENABLED", "1"),
            ("TURBO_EXPERIMENTAL_OTEL_ENDPOINT", "https://localhost:4318"),
            ("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel"),
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 successful, 1 total"));
}

#[test]
fn test_otel_cli_flags_do_not_break_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_otel_fixture(tempdir.path());

    // Prime the cache first
    run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[
            ("TURBO_EXPERIMENTAL_OTEL_ENABLED", "1"),
            ("TURBO_EXPERIMENTAL_OTEL_ENDPOINT", "https://localhost:4318"),
            ("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel"),
        ],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &[
            "run",
            "build",
            "--filter=my-app",
            "--experimental-otel-enabled",
            "--experimental-otel-endpoint=https://localhost:4318",
        ],
        &[("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel")],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FULL TURBO"));
}

#[test]
fn test_otel_disabled_does_not_break_run() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_otel_fixture(tempdir.path());

    // Prime cache
    run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel")],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[
            ("TURBO_EXPERIMENTAL_OTEL_ENABLED", "0"),
            ("TURBO_EXPERIMENTAL_OTEL_ENDPOINT", "https://localhost:4318"),
            ("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel"),
        ],
    );
    assert!(output.status.success());
}

#[test]
fn test_otel_enabled_without_endpoint_is_noop() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_otel_fixture(tempdir.path());

    // Prime cache
    run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel")],
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--filter=my-app"],
        &[
            ("TURBO_EXPERIMENTAL_OTEL_ENABLED", "1"),
            ("TURBO_CACHE_DIR", ".turbo/cache-experimental-otel"),
        ],
    );
    assert!(output.status.success());
}
