#![cfg(unix)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo_with_closed_stderr, run_turbo_with_closed_stdout, setup};

fn basic_monorepo() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    tempdir
}

#[test]
fn test_closed_stdout_exits_quietly() {
    let tempdir = basic_monorepo();

    for args in [&["info"][..], &["--version"]] {
        let (status, stderr) = run_turbo_with_closed_stdout(tempdir.path(), args);
        assert!(
            !stderr.contains("Turbo has crashed") && !stderr.contains("Broken pipe"),
            "turbo {args:?} reported a broken pipe: {stderr}"
        );
        assert_eq!(status.code(), Some(0), "turbo {args:?}: {stderr}");
    }
}

#[test]
fn test_error_with_closed_stderr_keeps_exit_code() {
    let tempdir = basic_monorepo();

    // Like `turbo run nosuchtask 2>&1 | head -1`: the error report can't be
    // written, but turbo still exits 1 instead of panicking (101).
    for args in [
        &["run", "nosuchtask"][..],
        &["run", "build", "--output-logs=full", "--output-logs=none"],
    ] {
        let (status, stdout) = run_turbo_with_closed_stderr(tempdir.path(), args);
        assert_eq!(status.code(), Some(1), "turbo {args:?}, stdout: {stdout}");
    }
}
