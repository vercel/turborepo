mod common;

use common::{run_turbo, setup};

#[test]
fn test_ls_all_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("3 packages"));
    assert!(stdout.contains("another"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
}

#[test]
fn test_ls_with_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "-F", "my-app..."]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 packages"));
    assert!(stdout.contains("my-app"));
    assert!(stdout.contains("util"));
    assert!(!stdout.contains("another"));
}

#[test]
fn test_ls_package_detail() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "my-app"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-app depends on: util"));
    assert!(stdout.contains("build: echo building"));
}

#[test]
fn test_ls_package_no_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["ls", "another"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("another depends on: <no packages>"));
}
