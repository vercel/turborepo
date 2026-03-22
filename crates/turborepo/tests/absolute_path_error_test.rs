mod common;

use common::{replace_turbo_json, run_turbo, setup};

fn inputs_config() -> &'static str {
    if cfg!(windows) {
        "abs-path-inputs-win.json"
    } else {
        "abs-path-inputs.json"
    }
}

fn outputs_config() -> &'static str {
    if cfg!(windows) {
        "abs-path-outputs-win.json"
    } else {
        "abs-path-outputs.json"
    }
}

fn global_deps_config() -> &'static str {
    if cfg!(windows) {
        "abs-path-global-deps-win.json"
    } else {
        "abs-path-global-deps.json"
    }
}

#[test]
fn test_absolute_path_in_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    replace_turbo_json(tempdir.path(), inputs_config());

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
    replace_turbo_json(tempdir.path(), outputs_config());

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
    replace_turbo_json(tempdir.path(), global_deps_config());

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
