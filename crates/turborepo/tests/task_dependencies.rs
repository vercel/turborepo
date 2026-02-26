mod common;

use common::{run_turbo, setup, turbo_output_filters};

fn setup_task_deps(fixture_suffix: &str) -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        &format!("task_dependencies/{fixture_suffix}"),
        "npm@10.5.0",
        true,
    )
    .unwrap();
    tempdir
}

// === complex.t ===

#[test]
fn test_complex_build1_graph() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(
        tempdir.path(),
        &["run", "build1", "--filter=app-b", "--graph"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("complex_build1_graph", stdout.to_string());
}

#[test]
fn test_complex_build2_missing_custom_task() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(tempdir.path(), &["run", "build2"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        combined
            .contains(r#"Could not find "app-a#custom" in root turbo.json or "custom" in package"#),
        "expected missing custom task error, got:\n{combined}"
    );
}

#[test]
fn test_complex_build3_missing_package() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(tempdir.path(), &["run", "build3"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        combined
            .contains(r#"Could not find package "unknown" referenced by task "unknown#custom" in"#),
        "expected missing package error, got:\n{combined}"
    );
}

#[test]
fn test_complex_test_graph() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(tempdir.path(), &["run", "test", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("complex_test_graph", stdout.to_string());
}

#[test]
fn test_complex_test_only_graph() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(tempdir.path(), &["run", "test", "--only", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("complex_test_only_graph", stdout.to_string());
}

#[test]
fn test_complex_build4_self_dependency() {
    let tempdir = setup_task_deps("complex");
    let output = run_turbo(tempdir.path(), &["run", "build4"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        combined.contains("#build4 depends on itself"),
        "expected self-dependency error, got:\n{combined}"
    );
}

// === root-workspace.t ===

#[test]
fn test_root_workspace() {
    let tempdir = setup_task_deps("root-to-workspace");
    let output = run_turbo(tempdir.path(), &["run", "mytask"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("root_workspace", stdout.to_string());
    });
}

// === overwriting.t ===

#[test]
fn test_overwriting() {
    let tempdir = setup_task_deps("overwriting");
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // workspace-a:generate SHOULD have run
    assert!(
        stdout.contains("workspace-a:generate"),
        "expected workspace-a:generate to run"
    );
    // workspace-a:build SHOULD have run
    assert!(
        stdout.contains("workspace-a:build"),
        "expected workspace-a:build to run"
    );
    // workspace-b:generate should NOT have run (workspace-b overrides build to not
    // depend on generate)
    assert!(
        !stdout.contains("workspace-b:generate"),
        "expected workspace-b:generate to NOT run, but it did"
    );
}

// === topological.t ===

#[test]
fn test_topological_run() {
    let tempdir = setup_task_deps("topological");
    let output = run_turbo(tempdir.path(), &["run", "build"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::with_settings!({ filters => turbo_output_filters() }, {
        insta::assert_snapshot!("topological_run", stdout.to_string());
    });
}

#[test]
fn test_topological_graph() {
    let tempdir = setup_task_deps("topological");
    let output = run_turbo(tempdir.path(), &["run", "build", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("topological_graph", stdout.to_string());
}

// === workspace-tasks.t ===

#[test]
fn test_workspace_tasks_build1_graph() {
    let tempdir = setup_task_deps("workspace-tasks");
    let output = run_turbo(tempdir.path(), &["run", "build1", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("workspace_tasks_build1_graph", stdout.to_string());
}

#[test]
fn test_workspace_tasks_build2_graph() {
    let tempdir = setup_task_deps("workspace-tasks");
    let output = run_turbo(tempdir.path(), &["run", "build2", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("workspace_tasks_build2_graph", stdout.to_string());
}

#[test]
fn test_workspace_tasks_build3_missing_root_task() {
    let tempdir = setup_task_deps("workspace-tasks");
    let output = run_turbo(tempdir.path(), &["run", "build3", "--graph"]);
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        combined
            .contains("//#not-exists requires an entry in turbo.json before it can be depended on"),
        "expected root task error, got:\n{combined}"
    );
    assert!(
        combined.contains("because it is a task declared in the root package.json"),
        "expected package.json reason, got:\n{combined}"
    );
}

#[test]
fn test_workspace_tasks_special_graph() {
    let tempdir = setup_task_deps("workspace-tasks");
    let output = run_turbo(tempdir.path(), &["run", "special", "--graph"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!("workspace_tasks_special_graph", stdout.to_string());
}
