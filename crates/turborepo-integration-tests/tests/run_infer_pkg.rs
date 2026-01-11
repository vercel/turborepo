//! Port of turborepo-tests/integration/tests/run/infer-pkg.t
//!
//! Tests package inference behavior when running turbo from different
//! directories within the monorepo.

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::TurboTestEnv;

/// Run turbo with --dry=json and extract the packages array.
async fn get_dry_run_packages(env: &TurboTestEnv, args: &[&str]) -> Result<Vec<String>> {
    let result = env.run_turbo(args).await?;
    result.assert_success();

    // Parse the JSON output
    let json: serde_json::Value = serde_json::from_str(&result.stdout)?;
    let packages = json["packages"]
        .as_array()
        .expect("packages should be an array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    Ok(packages)
}

/// Run turbo from a subdirectory with --dry=json and extract the packages
/// array.
async fn get_dry_run_packages_from_dir(
    env: &TurboTestEnv,
    subdir: &str,
    args: &[&str],
) -> Result<Vec<String>> {
    let result = env.run_turbo_from_dir(subdir, args).await?;
    result.assert_success();

    // Parse the JSON output
    let json: serde_json::Value = serde_json::from_str(&result.stdout)?;
    let packages = json["packages"]
        .as_array()
        .expect("packages should be an array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    Ok(packages)
}

/// Test dry run from root returns all packages.
#[tokio::test]
async fn test_dry_run_from_root() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages(&env, &["build", "--dry=json"]).await?;

    assert_eq!(packages, vec!["another", "my-app", "util"]);
    Ok(())
}

/// Test dry run with glob filter "./packages/*".
#[tokio::test]
async fn test_dry_run_glob_filter_packages() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages =
        get_dry_run_packages(&env, &["build", "--dry=json", "-F", "./packages/*"]).await?;

    assert_eq!(packages, vec!["another", "util"]);
    Ok(())
}

/// Test dry run with name glob filter "*-app".
#[tokio::test]
async fn test_dry_run_name_glob_filter() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages(&env, &["build", "--dry=json", "-F", "*-app"]).await?;

    assert_eq!(packages, vec!["my-app"]);
    Ok(())
}

/// Test dry run from packages directory with relative filter.
#[tokio::test]
async fn test_dry_run_from_packages_dir_with_filter() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages =
        get_dry_run_packages_from_dir(&env, "packages", &["build", "--dry=json", "-F", "{./util}"])
            .await?;

    assert_eq!(packages, vec!["util"]);
    Ok(())
}

/// Test dry run from packages directory with sibling filter "../apps/*".
#[tokio::test]
async fn test_dry_run_from_packages_dir_sibling_filter() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages_from_dir(
        &env,
        "packages",
        &["build", "--dry=json", "-F", "../apps/*"],
    )
    .await?;

    assert_eq!(packages, vec!["my-app"]);
    Ok(())
}

/// Test dry run from packages directory with name glob filter.
#[tokio::test]
async fn test_dry_run_from_packages_dir_name_glob() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages =
        get_dry_run_packages_from_dir(&env, "packages", &["build", "--dry=json", "-F", "*-app"])
            .await?;

    assert_eq!(packages, vec!["my-app"]);
    Ok(())
}

/// Test package inference from a package directory.
#[tokio::test]
async fn test_infer_package_from_directory() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages =
        get_dry_run_packages_from_dir(&env, "packages/util", &["build", "--dry=json"]).await?;

    assert_eq!(packages, vec!["util"]);
    Ok(())
}

/// Test that --cwd disables package inference.
#[tokio::test]
async fn test_cwd_disables_inference() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages_from_dir(
        &env,
        "packages/util",
        &["build", "--cwd=../..", "--dry=json"],
    )
    .await?;

    assert_eq!(packages, vec!["another", "my-app", "util"]);
    Ok(())
}

/// Test dry run from package directory with relative glob filter.
#[tokio::test]
async fn test_dry_run_from_package_dir_glob_filter() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages_from_dir(
        &env,
        "packages/util",
        &["build", "--dry=json", "-F", "../*"],
    )
    .await?;

    assert_eq!(packages, vec!["util"]);
    Ok(())
}

/// Test dry run from package directory with name glob filter for another
/// package.
#[tokio::test]
async fn test_dry_run_from_package_dir_name_glob() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("basic_monorepo").await?;
    env.setup_git().await?;

    let packages = get_dry_run_packages_from_dir(
        &env,
        "packages/util",
        &["build", "--dry=json", "-F", "*nother"],
    )
    .await?;

    assert_eq!(packages, vec!["another"]);
    Ok(())
}
