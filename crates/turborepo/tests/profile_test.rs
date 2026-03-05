mod common;

use std::fs;

use common::{run_turbo, setup};

fn setup_and_run(args: &[&str]) -> (tempfile::TempDir, std::process::Output) {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();
    let output = run_turbo(tempdir.path(), args);
    assert!(output.status.success());
    (tempdir, output)
}

fn assert_valid_trace_and_markdown(dir: &std::path::Path, trace_name: &str) {
    let trace_path = dir.join(trace_name);
    assert!(trace_path.exists(), "{trace_name} should exist");
    let trace_contents = fs::read_to_string(&trace_path).unwrap();
    let _: serde_json::Value =
        serde_json::from_str(&trace_contents).expect("trace file should be valid JSON");

    let md_path = dir.join(format!("{trace_name}.md"));
    assert!(md_path.exists(), "{trace_name}.md should exist");
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

fn assert_default_profile_files_exist(dir: &std::path::Path) {
    let entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("profile."))
        .collect();
    let trace_files: Vec<_> = entries
        .iter()
        .filter(|e| !e.file_name().to_string_lossy().ends_with(".md"))
        .collect();
    let md_files: Vec<_> = entries
        .iter()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
        .collect();
    assert_eq!(
        trace_files.len(),
        1,
        "expected exactly one trace file, got: {trace_files:?}"
    );
    assert_eq!(
        md_files.len(),
        1,
        "expected exactly one .md file, got: {md_files:?}"
    );
}

#[test]
fn test_profile_generates_valid_trace() {
    let (tempdir, _) = setup_and_run(&["build", "--profile=build.trace"]);
    assert_valid_trace_and_markdown(tempdir.path(), "build.trace");
}

#[test]
fn test_profile_default_filename() {
    let (tempdir, _) = setup_and_run(&["build", "--profile"]);
    assert_default_profile_files_exist(tempdir.path());
}

#[test]
fn test_anon_profile_generates_valid_trace() {
    let (tempdir, _) = setup_and_run(&["build", "--anon-profile=anon.trace"]);
    assert_valid_trace_and_markdown(tempdir.path(), "anon.trace");
}

#[test]
fn test_anon_profile_default_filename() {
    let (tempdir, _) = setup_and_run(&["build", "--anon-profile"]);
    assert_default_profile_files_exist(tempdir.path());
}
