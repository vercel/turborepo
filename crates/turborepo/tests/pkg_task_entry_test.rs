mod common;

use common::{combined_output, run_turbo, setup};

fn setup_fixture(dir: &std::path::Path) {
    setup::setup_integration_test(dir, "pkg_task_entry", "npm@10.5.0", true).unwrap();
}

fn dry_run_tasks(dir: &std::path::Path, args: &[&str]) -> Vec<String> {
    let mut full_args = vec!["run"];
    full_args.extend_from_slice(args);
    full_args.push("--dry=json");
    let output = run_turbo(dir, &full_args);
    assert!(
        output.status.success(),
        "turbo failed: {}",
        combined_output(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid JSON from --dry=json");
    let mut tasks: Vec<String> = json["tasks"]
        .as_array()
        .expect("expected tasks array")
        .iter()
        .map(|t| t["taskId"].as_str().unwrap().to_string())
        .collect();
    tasks.sort();
    tasks
}

#[test]
fn pkg_task_syntax_as_sole_entry_point() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    // `turbo run web#build` should only run web#build (+ dependency lib#build)
    let tasks = dry_run_tasks(tempdir.path(), &["web#build"]);
    assert_eq!(tasks, vec!["lib#build", "web#build"]);
}

#[test]
fn pkg_task_syntax_union_with_filter() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    // `turbo run build --filter=docs web#build` should run:
    // - docs#build (from --filter + bare task)
    // - web#build (from pkg#task entry)
    // - lib#build (dependency of web)
    let tasks = dry_run_tasks(tempdir.path(), &["build", "--filter=docs", "web#build"]);
    assert_eq!(tasks, vec!["docs#build", "lib#build", "web#build"]);
}

#[test]
fn pkg_task_syntax_union_with_filter_cross_product() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    // `turbo run build --filter=docs web#lint` should run:
    // - docs#build (from --filter + bare task `build`)
    // - web#build (cross-product: web brought in by web#lint + bare task build)
    // - web#lint (from pkg#task entry)
    // - lib#build (dependency of web via ^build)
    let tasks = dry_run_tasks(tempdir.path(), &["build", "--filter=docs", "web#lint"]);
    assert_eq!(
        tasks,
        vec!["docs#build", "lib#build", "web#build", "web#lint"]
    );
}

#[test]
fn pkg_task_syntax_multiple_qualified_tasks() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    // `turbo run web#build docs#lint` should only run those specific entries
    // plus web's dependency
    let tasks = dry_run_tasks(tempdir.path(), &["web#build", "docs#lint"]);
    assert_eq!(tasks, vec!["docs#lint", "lib#build", "web#build"]);
}

#[test]
fn pkg_task_syntax_nonexistent_package_errors() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    let output = run_turbo(tempdir.path(), &["run", "nonexistent#build"]);
    assert!(
        !output.status.success(),
        "expected failure for nonexistent package"
    );
    let combined = combined_output(&output);
    assert!(
        combined.contains("nonexistent") && combined.contains("Could not find package"),
        "expected error about missing package, got: {combined}"
    );
}

#[test]
fn pkg_task_syntax_filter_exclusion_overridden() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_fixture(tempdir.path());

    // `turbo run web#build --filter=!web` should still run web#build
    // because pkg#task always adds its package regardless of filter exclusions
    let tasks = dry_run_tasks(tempdir.path(), &["web#build", "--filter=!web"]);
    assert_eq!(tasks, vec!["lib#build", "web#build"]);
}
