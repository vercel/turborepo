mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_profile_generates_valid_trace() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--profile=build.trace"]);
    assert!(output.status.success());

    // Verify the trace file is valid JSON
    let trace_path = tempdir.path().join("build.trace");
    assert!(trace_path.exists(), "build.trace should exist");
    let trace_contents = fs::read_to_string(&trace_path).unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&trace_contents).expect("build.trace should be valid JSON");

    // Verify the markdown profile summary was generated
    let md_path = tempdir.path().join("build.trace.md");
    assert!(md_path.exists(), "build.trace.md should exist");
    let md_contents = fs::read_to_string(&md_path).unwrap();
    assert!(
        md_contents.starts_with("# CPU Profile"),
        "expected markdown to start with '# CPU Profile'"
    );
    assert!(
        md_contents.contains("Hot Functions"),
        "expected 'Hot Functions' section in profile"
    );
    assert!(
        md_contents.contains("Call Tree"),
        "expected 'Call Tree' section in profile"
    );
}
