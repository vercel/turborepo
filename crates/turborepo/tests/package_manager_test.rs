mod common;

use std::fs;

use common::{run_turbo_with_env, setup};

fn get_package_manager(dir: &std::path::Path) -> String {
    let output = run_turbo_with_env(dir, &["config"], &[("TURBO_LOG_VERBOSITY", "off")]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["packageManager"].as_str().unwrap().to_string()
}

fn set_package_manager(dir: &std::path::Path, pm: &str) {
    let pkg_path = dir.join("package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["packageManager"] = serde_json::Value::String(pm.to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();
}

#[test]
fn test_detect_npm() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@8.19.4", false).unwrap();
    assert_eq!(get_package_manager(tempdir.path()), "npm");
}

#[test]
fn test_detect_yarn() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@8.19.4", false).unwrap();
    set_package_manager(tempdir.path(), "yarn@1.22.7");
    assert_eq!(get_package_manager(tempdir.path()), "yarn");
}

#[test]
fn test_detect_berry() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@8.19.4", false).unwrap();
    set_package_manager(tempdir.path(), "yarn@2.0.0");
    fs::write(
        tempdir.path().join(".yarnrc.yml"),
        "nodeLinker: node-modules\n",
    )
    .unwrap();
    assert_eq!(get_package_manager(tempdir.path()), "berry");
}

#[test]
fn test_detect_pnpm6() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@8.19.4", false).unwrap();
    set_package_manager(tempdir.path(), "pnpm@6.0.0");
    fs::write(
        tempdir.path().join("pnpm-workspace.yaml"),
        "packages:\n  - apps/*\n",
    )
    .unwrap();
    assert_eq!(get_package_manager(tempdir.path()), "pnpm6");
}

#[test]
fn test_detect_pnpm() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@8.19.4", false).unwrap();
    set_package_manager(tempdir.path(), "pnpm@7.0.0");
    fs::write(
        tempdir.path().join("pnpm-workspace.yaml"),
        "packages:\n  - apps/*\n",
    )
    .unwrap();
    assert_eq!(get_package_manager(tempdir.path()), "pnpm");
}
