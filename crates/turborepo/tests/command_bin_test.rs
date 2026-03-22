mod common;

use common::{run_turbo, setup};

#[test]
fn test_bin_shows_turbo_path() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["bin", "-vvv"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        combined.contains("Global turbo version:"),
        "expected global turbo version log, got: {combined}"
    );
    assert!(
        combined.contains("No local turbo binary found at"),
        "expected no local turbo message, got: {combined}"
    );
    assert!(
        combined.contains("Running command as global turbo"),
        "expected global turbo message, got: {combined}"
    );

    // The output should contain a path to the turbo binary
    let has_turbo_path = combined.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.contains("target") && trimmed.contains("turbo")
    });
    assert!(
        has_turbo_path,
        "expected path to turbo binary in output, got: {combined}"
    );
}
