mod common;

use common::{replace_turbo_json, run_turbo, setup};

#[test]
fn test_absolute_path_in_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "abs-path-inputs.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("`inputs` cannot contain an absolute path"),
        "expected absolute path inputs error, got: {combined}"
    );
}

#[test]
fn test_absolute_path_in_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "abs-path-outputs.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("`outputs` cannot contain an absolute path"),
        "expected absolute path outputs error, got: {combined}"
    );
}

#[test]
fn test_absolute_path_in_global_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), "abs-path-global-deps.json");

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("`globalDependencies` cannot contain an absolute path"),
        "expected absolute path globalDeps error, got: {combined}"
    );
}
