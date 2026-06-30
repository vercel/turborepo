#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

use common::{combined_output, run_turbo, setup};

#[test]
fn test_recursive_turbo_invocation_detected() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let output = run_turbo(tempdir.path(), &["something"]);
    let combined = combined_output(&output);

    assert!(
        combined.contains("recursive_turbo_invocations"),
        "expected recursive turbo invocation error, got: {combined}"
    );
    assert!(
        combined.contains("creating a loop"),
        "expected loop warning, got: {combined}"
    );
}

#[test]
fn test_nub_workspace_root_script_does_not_trigger_recursive_turbo_invocation() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let package_json_path = tempdir.path().join("package.json");
    let contents = fs::read_to_string(&package_json_path).unwrap();
    let mut package_json: serde_json::Value = serde_json::from_str(&contents).unwrap();
    package_json
        .as_object_mut()
        .unwrap()
        .remove("packageManager");
    package_json["scripts"]["build"] = serde_json::Value::String("turbo run build".to_string());
    package_json["devEngines"]["packageManager"] = serde_json::json!({
        "name": "nub",
        "version": "0.2.10"
    });
    fs::write(
        &package_json_path,
        serde_json::to_string_pretty(&package_json).unwrap(),
    )
    .unwrap();
    fs::remove_file(tempdir.path().join("package-lock.json")).unwrap();
    fs::write(tempdir.path().join("lock.yaml"), "lockfileVersion: '9.0'\n").unwrap();

    let fake_bin_dir = tempdir.path().join("fake-bin");
    fs::create_dir(&fake_bin_dir).unwrap();
    let fake_nub_path = fake_bin_dir.join("nub");
    fs::write(&fake_nub_path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&fake_nub_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_nub_path, permissions).unwrap();
    }

    let mut command = common::turbo_command(tempdir.path());
    command
        .arg("build")
        .env("PATH", setup::prepend_to_path(&fake_bin_dir));
    let output = command.output().unwrap();
    let combined = combined_output(&output);

    assert!(
        output.status.success(),
        "expected nub workspace build to succeed, got: {combined}"
    );
    assert!(
        !combined.contains("recursive_turbo_invocations"),
        "did not expect recursive turbo invocation error, got: {combined}"
    );
}
