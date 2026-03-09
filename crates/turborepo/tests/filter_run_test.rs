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

#[test]
fn test_exclude_only_filter_includes_root_tasks() {
    // Regression test for https://github.com/vercel/turborepo/issues/8672
    // When using an exclude-only filter like --filter=!my-app, root tasks
    // defined with //#task syntax should still be included.
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Replace root package.json "something" script with a non-recursive command
    // (the default script calls "turbo run build" which triggers recursion detection)
    let pkg_json_path = tempdir.path().join("package.json");
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
    fs::write(&pkg_json_path, pkg_json).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "fix root script", "--quiet", "--allow-empty"],
    );

    // Use --dry=json to check which packages are in scope
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
    // The root task //#something should be in scope even with exclude-only filter
    assert!(
        stdout.contains("//#something"),
        "root task //#something should be in scope with exclude-only filter: {stdout}"
    );
}

#[test]
fn test_no_filter_includes_root_tasks() {
    // Verify that root tasks work without any filter (baseline behavior)
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Replace root package.json "something" script with a non-recursive command
    let pkg_json_path = tempdir.path().join("package.json");
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
    fs::write(&pkg_json_path, pkg_json).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "fix root script", "--quiet", "--allow-empty"],
    );

    // Use --dry=json to check scope without executing tasks
    let output = run_turbo(
        tempdir.path(),
        &["run", "something", "--dry=json"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry run failed: stdout={stdout}, stderr={stderr}"
    );
    // The root task //#something should appear in the dry run output
    assert!(
        stdout.contains("//#something"),
        "root task //#something should be in scope without filter: {stdout}"
    );
}
