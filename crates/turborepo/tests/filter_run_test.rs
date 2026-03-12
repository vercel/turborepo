mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, setup};

/// Set up a basic_monorepo fixture with a non-recursive root `something`
/// script. The default fixture's `something` calls `turbo run build`, which
/// triggers recursion detection. This replaces it with a simple echo so we
/// can test root task scoping via `--dry=json` without side effects.
fn setup_root_task_fixture(dir: &Path) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", true).unwrap();

    let pkg_json = r#"{
  "name": "monorepo",
  "scripts": {
    "something": "echo root-task-executed"
  },
  "packageManager": "npm@10.5.0",
  "workspaces": [
    "apps/**",
    "packages/**"
  ]
}"#;
    fs::write(dir.join("package.json"), pkg_json).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "fix root script", "--quiet"]);
}

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

// -- Root task scoping tests --------------------------------------------------
//
// These tests verify that root tasks (//#something defined in turbo.json)
// are included or excluded from the run scope depending on the filter mode.
// Fixture: basic_monorepo with packages my-app, util, another.

#[test]
fn test_no_filter_includes_root_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(tempdir.path(), &["run", "something", "--dry=json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("//#something"),
        "root task should be in scope without filter: {stdout}"
    );
}

#[test]
fn test_exclude_only_filter_includes_root_tasks() {
    // Regression test for https://github.com/vercel/turborepo/issues/8672
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "something", "--filter=!my-app", "--dry=json"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("//#something"),
        "root task should be in scope with exclude-only filter: {stdout}"
    );
}

#[test]
fn test_multiple_exclude_filters_include_root_tasks() {
    // Multiple exclude filters are still "exclude-only" — root tasks should
    // be included as long as none of them target the root package.
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "something",
            "--filter=!my-app",
            "--filter=!util",
            "--dry=json",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        stdout.contains("//#something"),
        "root task should be in scope with multiple exclude-only filters: {stdout}"
    );
}

#[test]
fn test_exclude_root_filter_excludes_root_tasks() {
    // Explicitly excluding root (--filter=!//) should prevent root task injection.
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "something", "--filter=!//", "--dry=json"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        !stdout.contains("//#something"),
        "root task should NOT be in scope when root is explicitly excluded: {stdout}"
    );
}

#[test]
fn test_include_filter_excludes_root_tasks() {
    // An include filter (--filter=my-app) means the user opted into specific
    // packages. Root tasks should not be auto-injected.
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["run", "something", "--filter=my-app", "--dry=json"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        !stdout.contains("//#something"),
        "root task should NOT be in scope with include-only filter: {stdout}"
    );
}

#[test]
fn test_mixed_include_exclude_filter_excludes_root_tasks() {
    // Mixed include+exclude (--filter=my-app --filter=!util) is an explicit
    // selection, not "all packages minus some". Root tasks should not be injected.
    let tempdir = tempfile::tempdir().unwrap();
    setup_root_task_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "something",
            "--filter=my-app",
            "--filter=!util",
            "--dry=json",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    assert!(
        !stdout.contains("//#something"),
        "root task should NOT be in scope with mixed include+exclude filters: {stdout}"
    );
}
