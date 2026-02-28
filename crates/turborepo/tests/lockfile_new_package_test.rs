mod common;

use std::fs;

use common::setup_lockfile_test;

#[test]
fn test_new_package_in_lockfile_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_lockfile_test(tempdir.path(), "pnpm");

    // Add new package with an external dependency
    fs::create_dir_all(tempdir.path().join("apps/c")).unwrap();
    fs::write(
        tempdir.path().join("apps/c/package.json"),
        r#"{"name":"c", "dependencies": {"has-symbols": "^1.0.3"}}"#,
    )
    .unwrap();

    // Update lockfile — tolerate failure
    std::process::Command::new("pnpm")
        .args(["i", "--frozen-lockfile=false"])
        .current_dir(tempdir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok();

    // We need --skip-infer because pnpm install creates a local turbo in
    // node_modules, and without it the shim would delegate to that binary.
    // We also filter out the root package (//) since we only care about c.
    let config_dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(assert_cmd::cargo::cargo_bin("turbo"))
        .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env("MSYS_NO_PATHCONV", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path())
        .args([
            "--skip-infer",
            "build",
            "-F",
            "[HEAD]",
            "-F",
            "!//",
            "--dry=json",
        ])
        .output()
        .expect("failed to execute turbo");

    // Suppress stderr (warnings about experimental features, etc.)
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "expected valid JSON, got error {e}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });

    let packages: Vec<&str> = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    assert_eq!(packages, vec!["c"]);
}
