#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

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

#[cfg(unix)]
#[test]
fn non_utf8_arguments_never_crash_the_shim() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let tempdir = tempfile::tempdir().unwrap();
    let bad = OsString::from_vec(vec![b'a', 0xff]);
    for (args, expected) in [
        // Forwarded arguments reach the CLI parser intact instead of panicking.
        (
            vec!["run".into(), "build".into(), "--".into(), bad.clone()],
            None,
        ),
        (vec!["run".into(), bad.clone()], Some("must be valid UTF-8")),
        // Diagnostics that echo the command line stay lossy, not fatal.
        (
            vec!["--cwd".into(), "--".into(), bad.clone()],
            Some("No value assigned to `--cwd` flag"),
        ),
    ] {
        let output = turbo_command(tempdir.path()).args(&args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(!stderr.contains("crashed"), "{args:?}: {stderr}");
        if let Some(expected) = expected {
            assert!(stderr.contains(expected), "{args:?}: {stderr}");
        }
    }
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
