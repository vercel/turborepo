#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use common::{run_turbo, run_turbo_with_env, setup, turbo_output_filters};

fn setup_persistent_deps(fixture_suffix: &str) -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        &format!("persistent_dependencies/{fixture_suffix}"),
        "npm@10.5.0",
        true,
    )
    .unwrap();
    tempdir
}

// Tests 1-5 and 7-9: error cases where persistent tasks have invalid dependents
macro_rules! persistent_dep_error_test {
    ($test_name:ident, $fixture:expr, $task:expr) => {
        #[test]
        fn $test_name() {
            let tempdir = setup_persistent_deps($fixture);
            let output = run_turbo(tempdir.path(), &["run", $task]);

            assert!(
                !output.status.success(),
                "expected turbo to fail but it exited with {:?}\nstdout: {}\nstderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );

            let stderr = String::from_utf8_lossy(&output.stderr);
            insta::assert_snapshot!(stringify!($test_name), stderr.to_string());
        }
    };
}

persistent_dep_error_test!(test_1_topological, "1-topological", "dev");
persistent_dep_error_test!(test_2_same_workspace, "2-same-workspace", "build");
persistent_dep_error_test!(test_3_workspace_specific, "3-workspace-specific", "build");
persistent_dep_error_test!(test_4_cross_workspace, "4-cross-workspace", "dev");
persistent_dep_error_test!(test_5_root_workspace, "5-root-workspace", "build");
persistent_dep_error_test!(test_7_topological_nested, "7-topological-nested", "dev");
persistent_dep_error_test!(
    test_8_topological_with_extra,
    "8-topological-with-extra",
    "build"
);
persistent_dep_error_test!(
    test_9_cross_workspace_nested,
    "9-cross-workspace-nested",
    "build"
);

// Test 6: topological dependency where the dep package doesn't implement the
// task — succeeds
#[test]
fn test_6_topological_unimplemented() {
    let tempdir = setup_persistent_deps("6-topological-unimplemented");
    let output = run_turbo(tempdir.path(), &["run", "dev"]);

    assert!(
        output.status.success(),
        "expected turbo to succeed but it exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!(
            "test_6_topological_unimplemented",
            stdout.to_string()
        );
    });
}

// Test 10: concurrency checks — persistent tasks require enough concurrency
// slots
#[test]
fn test_10_too_many_concurrency() {
    let tempdir = setup_persistent_deps("10-too-many");

    let output = run_turbo(tempdir.path(), &["run", "build", "--concurrency=1"]);
    assert_concurrency_error(&output);
    insta::assert_snapshot!(
        "test_10_concurrency_1_flag",
        String::from_utf8_lossy(&output.stderr).to_string()
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build"],
        &[("TURBO_CONCURRENCY", "1")],
    );
    assert_concurrency_error(&output);
    insta::assert_snapshot!(
        "test_10_concurrency_1_env",
        String::from_utf8_lossy(&output.stderr).to_string()
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--concurrency=2"]);
    assert_concurrency_error(&output);
    insta::assert_snapshot!(
        "test_10_concurrency_2_flag",
        String::from_utf8_lossy(&output.stderr).to_string()
    );

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build"],
        &[("TURBO_CONCURRENCY", "2")],
    );
    assert_concurrency_error(&output);
    insta::assert_snapshot!(
        "test_10_concurrency_2_env",
        String::from_utf8_lossy(&output.stderr).to_string()
    );

    let output = run_turbo(tempdir.path(), &["run", "build", "--concurrency=3"]);
    assert_concurrency_success(&output);

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build"],
        &[("TURBO_CONCURRENCY", "3")],
    );
    assert_concurrency_success(&output);
}

fn assert_concurrency_error(output: &std::process::Output) {
    assert!(!output.status.success());
}

fn assert_concurrency_success(output: &std::process::Output) {
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 successful, 2 total"),
        "expected output to contain '2 successful, 2 total', got:\n{stdout}"
    );
}
