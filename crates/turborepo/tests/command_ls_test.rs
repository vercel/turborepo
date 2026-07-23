#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{combined_output, run_turbo, setup};

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

#[test]
fn test_ls_rejects_nested_npm_workspace_root() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    std::fs::write(
        tempdir.path().join("packages/util/package.json"),
        r#"{"name":"util","workspaces":["apps/*"]}"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);

    assert!(!output.status.success());
    let output = combined_output(&output).replace('\\', "/");
    assert!(
        output.contains("multiple independent npm workspace roots are unsupported: accepted ,")
            && output.contains("conflicting packages/util"),
        "expected duplicate native workspace-root diagnostic, got:\n{output}"
    );
}

#[test]
fn test_ls_accepts_pnpm_per_workspace_lockfiles() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("pnpm_per_workspace_lockfile", tempdir.path()).unwrap();

    let output = run_turbo(tempdir.path(), &["ls"]);

    assert!(
        output.status.success(),
        "ls failed: {}",
        combined_output(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("4 packages"));
    for package in ["@repo/config", "@repo/ui", "docs", "web"] {
        assert!(stdout.contains(package), "missing {package} in:\n{stdout}");
    }
}
