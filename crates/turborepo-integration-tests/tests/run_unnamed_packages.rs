//! Port of turborepo-tests/integration/tests/run/unnamed-packages.t
//!
//! Tests that packages without a "name" field in package.json are ignored.
//! The nested_packages fixture has an unnamed package at
//! apps/my-app/.ignore/package.json which should be filtered out during package
//! discovery.

#![cfg(feature = "integration-tests")]

mod common;

use anyhow::Result;
use common::{TurboTestEnv, redact_output};
use insta::assert_snapshot;

/// Test that unnamed packages are filtered out and turbo runs successfully.
///
/// The nested_packages fixture contains:
/// - my-app: has name, has build script
/// - util: has name, no build script
/// - .ignore (nested under my-app): NO name field
///
/// Expected behavior:
/// - Packages in scope: my-app, util (unnamed package ignored)
/// - Only my-app:build runs (util has no build script)
#[tokio::test]
async fn test_unnamed_packages_are_ignored() -> Result<()> {
    let env = TurboTestEnv::new().await?;
    env.copy_fixture("nested_packages").await?;
    env.set_package_manager("npm@10.5.0").await?;
    env.setup_git().await?;

    let result = env.run_turbo(&["build"]).await?;
    result.assert_success();

    assert_snapshot!(
        "unnamed_packages_build",
        redact_output(&result.combined_output())
    );

    Ok(())
}
