mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_large_git_status_with_spaces() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let util_dir = tempdir.path().join("packages/util");
    for i in 1..=10000 {
        fs::write(util_dir.join(format!("with spaces {i}.txt")), "new file").unwrap();
    }

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry", "-F", "util"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Inputs Files Considered        = 10001"),
        "expected 10001 inputs (10000 new + 1 existing), got: {stdout}"
    );
}
