mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_input_directory_glob_causes_cache_miss() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "dir_globs", "npm@10.5.0", true).unwrap();

    // First build: cache miss
    let output1 = run_turbo(
        tempdir.path(),
        &["build", "--filter=util", "--output-logs=hash-only"],
    );
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    assert!(stdout1.contains("cache miss"));

    // Add a file in the input directory
    fs::write(tempdir.path().join("packages/util/src/oops.txt"), "").unwrap();

    // Second build: cache miss due to new input file
    let output2 = run_turbo(
        tempdir.path(),
        &["build", "--filter=util", "--output-logs=hash-only"],
    );
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("cache miss"),
        "expected cache miss after adding file, got: {stdout2}"
    );

    // Verify output was produced
    let hello = fs::read_to_string(tempdir.path().join("packages/util/dist/hello.txt")).unwrap();
    assert!(hello.contains("world"));

    // Delete the output file
    fs::remove_file(tempdir.path().join("packages/util/dist/hello.txt")).unwrap();

    // Third build: cache hit restores the output
    let output3 = run_turbo(
        tempdir.path(),
        &["build", "--filter=util", "--output-logs=hash-only"],
    );
    assert!(output3.status.success());
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout3.contains("cache hit"),
        "expected cache hit, got: {stdout3}"
    );

    // Verify output was restored from cache
    let hello_restored =
        fs::read_to_string(tempdir.path().join("packages/util/dist/hello.txt")).unwrap();
    assert!(hello_restored.contains("world"));
}
