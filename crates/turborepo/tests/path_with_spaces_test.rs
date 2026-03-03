mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_files_with_spaces_can_be_hashed() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Create a file with spaces in the name
    fs::write(
        tempdir.path().join("packages/util/with spaces.txt"),
        "new file",
    )
    .unwrap();

    // Dry run should succeed and count the file
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry", "-F", "util"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Inputs Files Considered"),
        "expected dry run output with inputs count, got: {stdout}"
    );
}
