mod common;

use common::{run_turbo, setup, turbo_output_filters};

#[test]
fn test_passthrough_args_with_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "passthrough", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["-F", "my-app", "echo", "--", "hello"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello"),
        "expected passthrough arg 'hello' in output, got: {stdout}"
    );
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("passthrough_filter_hello", stdout.to_string());
    });
}

#[test]
fn test_passthrough_args_with_task_id() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "passthrough", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["my-app#echo", "--", "goodbye"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("goodbye"),
        "expected passthrough arg 'goodbye' in output, got: {stdout}"
    );
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("passthrough_task_id_goodbye", stdout.to_string());
    });
}
