mod common;

use std::fs;

use common::{combined_output, git, run_turbo, run_turbo_with_env, setup};

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

const TURBO_JSON_GLOBAL_INPUTS_TOPO: &str = r#"{
  "futureFlags": { "globalConfiguration": true },
  "global": {
    "inputs": ["config.txt"]
  },
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
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
        combined.contains("app-a:build") && combined.contains("cache miss"),
        "expected app-a cache miss, got: {combined}"
    );
    assert!(
        combined.contains("app-b:build") && combined.contains("cache miss"),
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
        combined.contains("app-a:build") && combined.contains("cache miss"),
        "expected app-a cache miss, got: {combined}"
    );
    // app-b excludes config.txt via !$TURBO_ROOT$/config.txt → cache hit
    assert!(
        combined.contains("app-b:build") && combined.contains("cache hit"),
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

/// `--affected` should detect that tasks are affected when a
/// `global.inputs` file changes, even though the file is no longer
/// in the global hash.
#[test]
fn test_global_inputs_affected_detects_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), TURBO_JSON_GLOBAL_INPUTS);

    // Record the base commit before any changes.
    let base_sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(tempdir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    // Modify the global input file and commit.
    fs::write(tempdir.path().join("config.txt"), "changed value").unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "change config", "--quiet"],
    );

    // --affected should detect both tasks.
    let output = run_turbo_with_env(
        tempdir.path(),
        &["build", "--affected", "--output-logs=hash-only"],
        &[("TURBO_SCM_BASE", &base_sha), ("TURBO_SCM_HEAD", "HEAD")],
    );
    assert!(output.status.success());
    let combined = combined_output(&output);

    assert!(
        combined.contains("app-a:build"),
        "expected app-a:build to be affected by global input change, got: {combined}"
    );
    assert!(
        combined.contains("app-b:build"),
        "expected app-b:build to be affected by global input change, got: {combined}"
    );
}

/// When `dependsOn: ["^build"]` creates phantom dependency tasks for
/// packages without a build script (e.g. shared-lib), those phantom tasks
/// must NOT include global inputs in their hash. Otherwise, changing a
/// global input file cascades through the phantom task's hash into
/// downstream tasks that explicitly excluded the file.
#[test]
fn test_global_inputs_exclusion_not_defeated_by_phantom_dependency_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path(), TURBO_JSON_GLOBAL_INPUTS_TOPO);

    // First build: all miss
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());

    // Second build: all hit
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let combined = combined_output(&output);
    assert!(
        combined.contains("FULL TURBO"),
        "expected full cache hit on second run, got: {combined}"
    );

    // Modify the global input file
    fs::write(tempdir.path().join("config.txt"), "changed value").unwrap();

    // app-b depends on shared-lib (no build script) via ^build.
    // shared-lib:build is a phantom task — it must NOT hash config.txt.
    // app-b excludes config.txt → should still cache hit.
    let output = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output.status.success());
    let combined = combined_output(&output);

    assert!(
        combined.contains("app-a:build") && combined.contains("cache miss"),
        "expected app-a cache miss, got: {combined}"
    );
    assert!(
        combined.contains("app-b:build") && combined.contains("cache hit"),
        "expected app-b cache hit (phantom dep should not defeat negation), got: {combined}"
    );
}
