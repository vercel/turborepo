mod common;

use common::turbo_command;

#[test]
fn test_cwd_no_value() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = turbo_command(tempdir.path())
        .args(["foo", "bar", "--cwd"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No value assigned to `--cwd` flag"),
        "expected empty cwd error, got: {stderr}"
    );
}

#[test]
fn test_multiple_cwd_flags() {
    let tempdir = tempfile::tempdir().unwrap();
    let output = turbo_command(tempdir.path())
        .args([
            "--cwd", "foo", "--cwd", "--bar", "--cwd", "baz", "--cwd", "qux",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot have multiple `--cwd` flags"),
        "expected multiple cwd error, got: {stderr}"
    );
}
