mod common;

use common::{run_turbo, setup};

fn get_packages(output: &std::process::Output) -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("expected valid JSON from --dry=json");
    let mut pkgs: Vec<String> = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    pkgs.sort();
    pkgs
}

#[test]
fn test_all_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--dry=json"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["another", "my-app", "util"]);
}

#[test]
fn test_glob_filter_packages_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["build", "--dry=json", "-F", "./packages/*"],
    );
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["another", "util"]);
}

#[test]
fn test_name_glob_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--dry=json", "-F", "*-app"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["my-app"]);
}

#[test]
fn test_infer_from_packages_subdir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Run from packages/ with a relative filter
    let packages_dir = tempdir.path().join("packages");
    let output = run_turbo(&packages_dir, &["build", "--dry=json", "-F", "{./util}"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["util"]);
}

#[test]
fn test_filter_sibling_directory() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let packages_dir = tempdir.path().join("packages");
    let output = run_turbo(&packages_dir, &["build", "--dry=json", "-F", "../apps/*"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["my-app"]);
}

#[test]
fn test_infer_from_package_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Run from packages/util — should infer util as the package
    let util_dir = tempdir.path().join("packages/util");
    let output = run_turbo(&util_dir, &["build", "--dry=json"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["util"]);
}

#[test]
fn test_cwd_overrides_inference() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let util_dir = tempdir.path().join("packages/util");
    let output = run_turbo(&util_dir, &["build", "--cwd=../..", "--dry=json"]);
    assert!(output.status.success());
    // --cwd should override the inferred package
    assert_eq!(get_packages(&output), vec!["another", "my-app", "util"]);
}

#[test]
fn test_glob_filter_from_package_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let util_dir = tempdir.path().join("packages/util");
    let output = run_turbo(&util_dir, &["build", "--dry=json", "-F", "../*"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["util"]);
}

#[test]
fn test_name_glob_from_package_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let util_dir = tempdir.path().join("packages/util");
    let output = run_turbo(&util_dir, &["build", "--dry=json", "-F", "*nother"]);
    assert!(output.status.success());
    assert_eq!(get_packages(&output), vec!["another"]);
}
