mod common;

use std::fs;

use common::{combined_output, run_turbo, setup, turbo_output_filters};
use serde_json::json;

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
fn test_graph_to_html_escapes_task_names() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_graph_escape_fixture(tempdir.path());

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "back`tick",
            "interpolate${globalThis.alert(1)}",
            "break</script>out",
            "--graph=graph.html",
        ],
    );
    assert!(output.status.success(), "{}", combined_output(&output));

    let html = fs::read_to_string(tempdir.path().join("graph.html")).unwrap();
    assert!(html.contains("back`tick"));
    assert!(html.contains("interpolate${globalThis.alert(1)}"));
    assert!(!html.contains("const s = `"));
    assert!(!html.contains("break</script>out"));
    assert!(html.contains(r#"break\u003C/script\u003Eout"#));
}

fn setup_graph_escape_fixture(dir: &std::path::Path) {
    fs::create_dir_all(dir.join("apps/app")).unwrap();
    fs::write(
        dir.join("package.json"),
        serde_json::to_string_pretty(&json!({
            "name": "monorepo",
            "packageManager": "npm@10.5.0",
            "workspaces": ["apps/*"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://turborepo.dev/schema.json",
            "tasks": {
                "back`tick": {},
                "interpolate${globalThis.alert(1)}": {},
                "break</script>out": {}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("apps/app/package.json"),
        serde_json::to_string_pretty(&json!({
            "name": "app",
            "scripts": {
                "back`tick": "echo backtick",
                "interpolate${globalThis.alert(1)}": "echo interpolate",
                "break</script>out": "echo break"
            }
        }))
        .unwrap(),
    )
    .unwrap();
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
