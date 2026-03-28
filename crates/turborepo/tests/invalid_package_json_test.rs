mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_empty_name_field() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Clear name field in my-app/package.json
    let pkg_path = tempdir.path().join("apps/my-app/package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["name"] = serde_json::Value::String(String::new());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("package.json must have a name field"),
        "expected empty name error: {stderr}"
    );
}

#[test]
fn test_invalid_package_manager_field() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let pkg_path = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    pkg["packageManager"] = serde_json::Value::String("bower@8.19.4".to_string());
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid_package_manager_field"),
        "expected invalid packageManager error: {stderr}"
    );
}

#[test]
fn test_invalid_semver_in_package_manager() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let pkg_path = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&pkg_path).unwrap();
    let mut pkg: serde_json::Value = serde_json::from_str(&contents).unwrap();
    // Version number too large for semver
    let huge_version = format!("npm@0.3.{}", "1".repeat(250));
    pkg["packageManager"] = serde_json::Value::String(huge_version);
    fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid_semantic_version"),
        "expected invalid semver error: {stderr}"
    );
}

#[test]
fn test_malformed_package_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    // Write invalid JSON with trailing comma
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{ "name": "foobar", }"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("package_json_parse_error"),
        "expected parse error: {stderr}"
    );
}
