mod common;

use common::{run_turbo, setup};

/// This test validates that running turbo from within a package subdirectory
/// correctly infers the package scope. We use `current_dir` to simulate
/// invoking turbo from packages/util.
#[test]
fn test_package_inference_from_subdirectory() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let util_dir = tempdir.path().join("packages/util");
    let output = run_turbo(&util_dir, &["build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Packages in scope: util"),
        "expected only util in scope when run from packages/util, got: {stdout}"
    );
    assert!(stdout.contains("1 successful, 1 total"));
}
