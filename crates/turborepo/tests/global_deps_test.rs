mod common;

use std::fs;

use common::{run_turbo, setup};

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
