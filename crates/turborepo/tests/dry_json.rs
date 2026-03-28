mod common;

use std::path::Path;

use common::{run_turbo, setup};
use serde_json::Value;

fn turbo_dry_json(test_dir: &Path, args: &[&str]) -> Result<Value, anyhow::Error> {
    let output = run_turbo(test_dir, args);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)?;
    Ok(json)
}

fn turbo_dry_json_expect_failure(test_dir: &Path, args: &[&str]) -> String {
    let output = run_turbo(test_dir, args);
    assert!(!output.status.success());
    String::from_utf8_lossy(&output.stderr).to_string()
}

// === monorepo tests ===

#[test]
fn test_monorepo_global_cache_inputs() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    insta::assert_json_snapshot!("monorepo_global_cache_inputs", json["globalCacheInputs"],);
    Ok(())
}

#[test]
fn test_monorepo_turbo_version() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    let version = json["turboVersion"]
        .as_str()
        .expect("turboVersion should be a string");
    assert!(
        version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
        "turboVersion should match [a-z0-9.-]+, got: {version}"
    );
    Ok(())
}

#[test]
fn test_monorepo_top_level_keys() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    let mut keys: Vec<String> = json.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    insta::assert_json_snapshot!("monorepo_top_level_keys", keys);
    Ok(())
}

#[test]
fn test_monorepo_my_app_build_task() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"] == "my-app#build")
        .expect("my-app#build task not found")
        .clone();

    insta::with_settings!({ filters => vec![(r"\\\\", "/")] }, {
        insta::assert_json_snapshot!(
            "monorepo_my_app_build_task",
            task,
        );
    });
    Ok(())
}

#[test]
fn test_monorepo_util_build_task() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"] == "util#build")
        .expect("util#build task not found")
        .clone();

    insta::with_settings!({ filters => vec![(r"\\\\", "/")] }, {
        insta::assert_json_snapshot!(
            "monorepo_util_build_task",
            task,
        );
    });
    Ok(())
}

#[test]
fn test_monorepo_env_var_in_summary() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;

    let config_dir = tempfile::tempdir()?;
    let mut cmd = assert_cmd::Command::cargo_bin("turbo")?;
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir.path())
        .env("DO_NOT_TRACK", "1")
        .env("NODE_ENV", "banana")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .args(["run", "build", "--dry=json", "--filter=util"])
        .current_dir(tempdir.path());

    let output = cmd.output()?;
    let json: Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))?;

    let env_vars = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"] == "util#build")
        .unwrap()["environmentVariables"]
        .clone();

    insta::assert_json_snapshot!("monorepo_util_build_env_vars_with_node_env", env_vars);
    Ok(())
}

#[test]
fn test_monorepo_missing_task_error() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let stderr =
        turbo_dry_json_expect_failure(tempdir.path(), &["run", "doesnotexist", "--dry=json"]);

    assert!(
        stderr.contains("Could not find task `doesnotexist` in project"),
        "Expected missing task error, got: {stderr}"
    );
    Ok(())
}

// === monorepo no changes ===

#[test]
fn test_monorepo_no_changes_packages() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false)?;
    let json = turbo_dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--filter=[main]"],
    )?;

    insta::assert_json_snapshot!("monorepo_no_changes_packages", json["packages"]);
    Ok(())
}

// === single-package ===

#[test]
fn test_single_package_dry_json() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    insta::with_settings!({ filters => vec![(r"\\\\", "/")] }, {
        insta::assert_json_snapshot!(
            "single_package_dry_json",
            json,
            {
                ".id" => "[id]",
                ".turboVersion" => "[turboVersion]",
                ".user" => "[user]",
                ".scm.sha" => "[sha]",
                ".scm.branch" => "[branch]",
            }
        );
    });
    Ok(())
}

// === single-package-no-change ===

#[test]
fn test_single_package_no_change() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", false)?;
    let json = turbo_dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--filter=[main]"],
    )?;

    insta::assert_json_snapshot!("single_package_no_change_packages", json["packages"]);
    Ok(())
}

// === single-package-no-config ===

#[test]
fn test_single_package_no_config() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true)?;

    // Remove turbo.json and commit
    std::fs::remove_file(tempdir.path().join("turbo.json"))?;
    std::process::Command::new("git")
        .args(["commit", "-am", "Delete turbo config", "--quiet"])
        .current_dir(tempdir.path())
        .status()?;

    let json = turbo_dry_json(tempdir.path(), &["run", "build", "--dry=json"])?;

    insta::with_settings!({ filters => vec![(r"\\\\", "/")] }, {
        insta::assert_json_snapshot!(
            "single_package_no_config",
            json,
            {
                ".id" => "[id]",
                ".turboVersion" => "[turboVersion]",
                ".user" => "[user]",
                ".scm.sha" => "[sha]",
                ".scm.branch" => "[branch]",
            }
        );
    });
    Ok(())
}

// === single-package-with-deps ===

#[test]
fn test_single_package_with_deps() -> Result<(), anyhow::Error> {
    let tempdir = tempfile::tempdir()?;
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true)?;
    let json = turbo_dry_json(tempdir.path(), &["run", "test", "--dry=json"])?;

    insta::with_settings!({ filters => vec![(r"\\\\", "/")] }, {
        insta::assert_json_snapshot!(
            "single_package_with_deps",
            json,
            {
                ".id" => "[id]",
                ".turboVersion" => "[turboVersion]",
                ".user" => "[user]",
                ".scm.sha" => "[sha]",
                ".scm.branch" => "[branch]",
            }
        );
    });
    Ok(())
}
