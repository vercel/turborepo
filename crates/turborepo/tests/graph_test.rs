mod common;

use std::fs;

use common::{run_turbo, setup, turbo_output_filters};

fn setup_topological(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "task_dependencies/topological", "npm@10.5.0", true)
        .unwrap();
}

#[test]
fn test_graph_to_stdout() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_topological(tempdir.path());

    let output = run_turbo(tempdir.path(), &["build", "-F", "my-app", "--graph"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph {"), "expected DOT output");
    assert!(
        stdout.contains(r#""[root] my-app#build" -> "[root] util#build""#),
        "expected my-app -> util edge"
    );
    assert!(
        stdout.contains(r#""[root] util#build" -> "[root] ___ROOT___""#),
        "expected util -> ROOT edge"
    );
}

#[test]
fn test_graph_to_dot_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_topological(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--graph=graph.dot"],
    );
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Generated task graph"),
        "expected generation message"
    );

    let dot = fs::read_to_string(tempdir.path().join("graph.dot")).unwrap();
    assert!(
        dot.contains(r#""[root] my-app#build" -> "[root] util#build""#),
        "expected edge in dot file"
    );
}

#[test]
fn test_graph_to_html_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_topological(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--graph=graph.html"],
    );
    assert!(output.status.success());

    let html = fs::read_to_string(tempdir.path().join("graph.html")).unwrap();
    assert!(html.contains("DOCTYPE"), "expected HTML DOCTYPE");
}

#[test]
fn test_graph_to_mermaid_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_topological(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--graph=graph.mermaid"],
    );
    assert!(output.status.success());

    let mermaid = fs::read_to_string(tempdir.path().join("graph.mermaid")).unwrap();
    assert!(mermaid.starts_with("graph TD"), "expected mermaid header");
    assert!(
        mermaid.contains("my-app#build"),
        "expected my-app#build in mermaid"
    );
    assert!(
        mermaid.contains("util#build"),
        "expected util#build in mermaid"
    );
}

#[test]
fn test_graph_invalid_extension() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_topological(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &["build", "-F", "my-app", "--graph=graph.mdx"],
    );
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("graph_invalid_extension", stderr.to_string());
    });
}
