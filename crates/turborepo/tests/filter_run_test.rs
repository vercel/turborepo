mod common;

use std::fs;

use common::{git, run_turbo, setup};

#[test]
fn test_filter_git_range_no_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=[main]"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 successful, 0 total"));
}

#[test]
fn test_filter_git_range_with_unstaged() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("bar.txt"), "new file contents\n").unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=[main]"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Packages in scope: //"));
}

#[test]
fn test_filter_git_range_committed_change() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let foo_path = tempdir.path().join("foo.txt");
    let mut contents = fs::read_to_string(&foo_path).unwrap_or_default();
    contents.push_str("\nglobal dependency");
    fs::write(&foo_path, contents).unwrap();
    git(
        tempdir.path(),
        &["commit", "-am", "global dependency change", "--quiet"],
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=[HEAD^]", "--output-logs", "none"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 successful, 2 total"),
        "all packages should rebuild after global dep change: {stdout}"
    );
}

#[test]
fn test_filter_nonexistent_package_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=foo", "--output-logs", "none"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No package found with name 'foo'"),
        "expected package not found error: {stderr}"
    );
}
