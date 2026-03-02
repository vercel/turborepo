mod common;

use std::path::Path;

use common::turbo_command;

#[test]
fn test_bin_matches_running_binary() {
    let tempdir = tempfile::tempdir().unwrap();
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");

    let output = turbo_command(tempdir.path())
        .args(["--cwd", repo_root.to_str().unwrap(), "bin"])
        .output()
        .expect("failed to execute turbo");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let bin_path = stdout.trim();

    assert!(!bin_path.is_empty(), "turbo bin should output a path");

    let expected = assert_cmd::cargo::cargo_bin("turbo");
    let bin_canon = std::fs::canonicalize(bin_path).unwrap_or_else(|_| bin_path.into());
    let expected_canon = std::fs::canonicalize(&expected).unwrap_or_else(|_| expected.clone());

    assert_eq!(
        bin_canon, expected_canon,
        "turbo bin should match the cargo test binary"
    );
}
