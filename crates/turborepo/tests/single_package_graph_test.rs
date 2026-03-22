mod common;

use std::fs;

use common::{run_turbo, setup};

#[test]
fn test_single_package_graph_to_stdout() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--graph"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph {"));
    assert!(stdout.contains(r#""[root] build" -> "[root] ___ROOT___""#));
}

#[test]
fn test_single_package_graph_to_dot_file() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["build", "--graph=graph.dot"]);
    assert!(output.status.success());

    let dot = fs::read_to_string(tempdir.path().join("graph.dot")).unwrap();
    assert!(dot.contains(r#""[root] build" -> "[root] ___ROOT___""#));
}

#[test]
fn test_single_package_with_deps_graph() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", true).unwrap();

    let output = run_turbo(tempdir.path(), &["run", "test", "--graph"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph {"));
    assert!(
        stdout.contains(r#""[root] test" -> "[root] build""#),
        "expected test -> build edge"
    );
    assert!(
        stdout.contains(r#""[root] build" -> "[root] ___ROOT___""#),
        "expected build -> ROOT edge"
    );
}
