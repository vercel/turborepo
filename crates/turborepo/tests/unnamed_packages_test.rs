mod common;

use common::{run_turbo, setup, turbo_output_filters};

#[test]
fn test_unnamed_packages_are_filtered() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "nested_packages", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // The unnamed nested package should be silently filtered out
    assert!(
        stdout.contains("Packages in scope: my-app, util"),
        "expected only named packages in scope, got: {stdout}"
    );

    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("unnamed_packages_build", stdout.to_string());
    });
}
