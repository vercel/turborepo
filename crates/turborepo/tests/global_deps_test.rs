mod common;

use std::fs;

use common::{git, run_turbo, setup};

#[test]
fn test_global_deps_change_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "global_deps", "npm@10.5.0", true).unwrap();

    // First build: cache miss
    let output1 = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--output-logs=hash-only"],
    );
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(stdout1.contains("cache miss"));

    // Change a global deps file
    fs::write(tempdir.path().join("global_deps/foo.txt"), "new text").unwrap();

    // Second build: cache miss because global dep changed
    let output2 = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--output-logs=hash-only"],
    );
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("cache miss"),
        "expected cache miss after global dep change, got: {stdout2}"
    );

    // Change a non-global-dep file (CONTRIBUTING.md is not in globalDeps)
    let contributing = tempdir.path().join("global_deps/CONTRIBUTING.md");
    let mut contents = fs::read_to_string(&contributing).unwrap_or_default();
    contents.push_str("\nSubmit a PR!");
    fs::write(&contributing, contents).unwrap();

    // Third build: cache hit because CONTRIBUTING.md is not a global dep
    let output3 = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--output-logs=hash-only"],
    );
    assert!(output3.status.success());
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout3.contains("FULL TURBO"),
        "expected cache hit after non-global-dep change, got: {stdout3}"
    );
}

/// Regression: with the globalInputsAsTaskInputs flag enabled, changing a
/// global dep file still causes a cache miss for tasks that do NOT negate it.
#[test]
fn test_global_deps_with_flag_still_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "global_deps", "npm@10.5.0", true).unwrap();

    // Enable the future flag by rewriting turbo.json
    let turbo_json = r#"{
  "globalDependencies": ["global_deps/**", "!global_deps/**/*.md"],
  "globalEnv": ["SOME_ENV_VAR"],
  "futureFlags": {
    "globalInputsAsTaskInputs": true
  },
  "tasks": {
    "build": {
      "env": ["NODE_ENV"],
      "outputs": []
    },
    "my-app#build": {
      "outputs": ["banana.txt", "apple.json"],
      "inputs": ["$TURBO_DEFAULT$", ".env.local"]
    },
    "something": {},
    "//#something": {},
    "maybefails": {}
  }
}"#;
    fs::write(tempdir.path().join("turbo.json"), turbo_json).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "enable flag", "--quiet", "--allow-empty"],
    );

    // First build: cache miss
    let output1 = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--output-logs=hash-only"],
    );
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(
        stdout1.contains("cache miss"),
        "expected initial cache miss, got: {stdout1}"
    );

    // Change a global deps file
    fs::write(tempdir.path().join("global_deps/foo.txt"), "new text").unwrap();

    // Second build: cache miss because global dep changed (even with new flag)
    let output2 = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--output-logs=hash-only"],
    );
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("cache miss"),
        "expected cache miss after global dep change with flag on, got: {stdout2}"
    );
}

/// The key feature: with globalInputsAsTaskInputs enabled, a task that
/// negates a global input is NOT invalidated when that file changes.
/// A task that does NOT negate it IS still invalidated.
#[test]
fn test_global_deps_negation_with_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "global_deps", "npm@10.5.0", true).unwrap();

    // Rewrite turbo.json: util#build negates foo.txt, my-app#build does not
    let turbo_json = r#"{
  "globalDependencies": ["global_deps/foo.txt"],
  "futureFlags": {
    "globalInputsAsTaskInputs": true
  },
  "tasks": {
    "build": {
      "outputs": []
    },
    "util#build": {
      "outputs": [],
      "inputs": ["$TURBO_DEFAULT$", "!$TURBO_ROOT$/global_deps/foo.txt"]
    }
  }
}"#;
    fs::write(tempdir.path().join("turbo.json"), turbo_json).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "enable flag", "--quiet", "--allow-empty"],
    );

    // First build for both packages: cache miss
    let output1 = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(
        stdout1.contains("cache miss"),
        "expected initial cache miss, got: {stdout1}"
    );

    // Second build: cache hit (sanity check)
    let output2 = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("FULL TURBO"),
        "expected cache hit on second run, got: {stdout2}"
    );

    // Change the global dep file
    fs::write(
        tempdir.path().join("global_deps/foo.txt"),
        "changed content",
    )
    .unwrap();

    // Third build: util#build should be a cache HIT (it negated foo.txt),
    // my-app#build should be a cache MISS (it did not negate foo.txt).
    let output3 = run_turbo(tempdir.path(), &["build", "--output-logs=hash-only"]);
    assert!(output3.status.success());
    let stdout3 = String::from_utf8_lossy(&output3.stdout);

    // my-app:build should miss
    assert!(
        stdout3.contains("my-app:build: cache miss"),
        "expected my-app:build cache miss after global dep change, got: {stdout3}"
    );
    // util:build should hit because it negated the global dep
    assert!(
        stdout3.contains("util:build: cache hit"),
        "expected util:build cache hit after negated global dep change, got: {stdout3}"
    );
}
