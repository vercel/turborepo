mod common;

use std::fs;

use common::{git, run_turbo, setup};

const TURBO_JSON_GLOBAL_DEPS: &str = r#"{
  "globalDependencies": ["config.txt"],
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}
"#;

const TURBO_JSON_GLOBAL_INPUTS: &str = r#"{
  "futureFlags": { "globalConfiguration": true },
  "global": {
    "inputs": ["config.txt"]
  },
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}
"#;

fn setup_fixture(dir: &std::path::Path, turbo_json: &str) {
    setup::setup_integration_test(dir, "global_inputs", "npm@10.5.0", true).unwrap();
    fs::write(dir.join("turbo.json"), turbo_json).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "set turbo config", "--quiet"]);
}

/// With `globalDependencies`, changing a global dep file invalidates ALL
/// tasks — even tasks that explicitly exclude the file with a negation glob.
/// The negation has no effect because the file is in the global hash, not
/// in per-task inputs.
#[test]
fn test_global_dependencies_cannot_be_excluded_by_task_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), TURBO_JSON_GLOBAL_DEPS);

    // First build: both tasks miss
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("FULL TURBO"),
        "expected cache miss on first run, got: {stdout}"
    );

    // Second build: both tasks hit
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected full cache hit on second run, got: {stdout}"
    );

    // Modify the global dep file
    fs::write(tempdir.path().join("config.txt"), "changed value").unwrap();

    // Third build: BOTH tasks miss — app-b's negation glob doesn't help
    // because config.txt is in the global hash
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("app-a#build") && combined.contains("cache miss"),
        "expected app-a cache miss, got: {combined}"
    );
    assert!(
        combined.contains("app-b#build") && combined.contains("cache miss"),
        "expected app-b cache miss (negation glob has no effect with globalDependencies), got: \
         {combined}"
    );
}

/// With `global.inputs` (via `futureFlags.globalConfiguration`), global
/// input files are prepended to every task's inputs instead of being
/// folded into the global hash. This means task-level negation globs
/// can successfully exclude them.
#[test]
fn test_global_inputs_can_be_excluded_by_task_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), TURBO_JSON_GLOBAL_INPUTS);

    // First build: both tasks miss
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("FULL TURBO"),
        "expected cache miss on first run, got: {stdout}"
    );

    // Second build: both tasks hit
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected full cache hit on second run, got: {stdout}"
    );

    // Modify the global input file
    fs::write(tempdir.path().join("config.txt"), "changed value").unwrap();

    // Third build: app-a misses (no negation) but app-b HITS (excluded config.txt)
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        !combined.contains("FULL TURBO"),
        "expected at least one cache miss, got: {combined}"
    );
    // app-a has no negation glob → config.txt is in its inputs → cache miss
    assert!(
        combined.contains("app-a#build") && combined.contains("cache miss"),
        "expected app-a cache miss, got: {combined}"
    );
    // app-b excludes config.txt via !$TURBO_ROOT$/config.txt → cache hit
    assert!(
        combined.contains("app-b#build") && combined.contains("cache hit"),
        "expected app-b cache hit (negation glob works with global.inputs), got: {combined}"
    );
}

/// When `global.inputs` is used and a task has no explicit `inputs` key,
/// the task should still hash all package files (via the default git
/// index behavior) in addition to the global input files.
#[test]
fn test_global_inputs_preserves_default_package_file_hashing() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), TURBO_JSON_GLOBAL_INPUTS);

    // First build: cache miss
    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "app-a", "--output-logs=hash-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss on first run, got: {stdout}"
    );

    // Second build: cache hit
    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "app-a", "--output-logs=hash-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected cache hit on second run, got: {stdout}"
    );

    // Modify a package file (not a global input)
    fs::write(
        tempdir.path().join("packages/app-a/index.js"),
        "console.log('modified');",
    )
    .unwrap();

    // Third build: cache miss — package files are still tracked
    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "app-a", "--output-logs=hash-only"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cache miss"),
        "expected cache miss after package file change (default hashing preserved), got: {stdout}"
    );
}
