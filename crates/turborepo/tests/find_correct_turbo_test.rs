mod common;

use std::path::Path;

#[test]
fn test_bin_matches_running_binary() {
    let tempdir = tempfile::tempdir().unwrap();
    // Point --cwd at the repo root so turbo can find turbo.json
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");

    let mut cmd = assert_cmd::Command::cargo_bin("turbo").expect("turbo binary not found");
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(tempdir.path())
        .args(["--cwd", repo_root.to_str().unwrap(), "bin"]);

    let output = cmd.output().expect("failed to execute turbo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let bin_path = stdout.trim();

    // The `turbo bin` output should resolve to the same binary we're testing
    assert!(!bin_path.is_empty(), "turbo bin should output a path");

    let expected = assert_cmd::cargo::cargo_bin("turbo");
    let bin_canon = std::fs::canonicalize(bin_path).unwrap_or_else(|_| bin_path.into());
    let expected_canon = std::fs::canonicalize(&expected).unwrap_or_else(|_| expected.clone());

    assert_eq!(
        bin_canon, expected_canon,
        "turbo bin should match the cargo test binary"
    );
}
