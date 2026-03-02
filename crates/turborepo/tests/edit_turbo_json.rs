mod common;

use std::path::Path;

use common::{run_turbo, setup};

fn fixture_configs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/tests/edit-turbo-json/fixture-configs")
}

fn replace_turbo_json(dir: &Path, config_name: &str) {
    std::fs::copy(
        fixture_configs_dir().join(config_name),
        dir.join("turbo.json"),
    )
    .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-am", "no comment", "--quiet"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

/// Returns a sorted "taskId=hash" string for snapshotting.
fn task_hash_snapshot(dir: &Path) -> String {
    let output = run_turbo(dir, &["build", "--dry=json"]);
    assert!(
        output.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut lines: Vec<String> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            format!(
                "{}={}",
                t["taskId"].as_str().unwrap(),
                t["hash"].as_str().unwrap()
            )
        })
        .collect();
    lines.sort();
    lines.join("\n")
}

fn global_cache_inputs(dir: &Path) -> String {
    let output = run_turbo(dir, &["build", "--dry=json"]);
    assert!(
        output.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    serde_json::to_string_pretty(&json["globalCacheInputs"]).unwrap()
}

#[test]
fn test_global_hash_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Baseline
    replace_turbo_json(tempdir.path(), "1-baseline.json");
    let baseline = global_cache_inputs(tempdir.path());

    // Update pipeline: global hash stable (only task-level config changed)
    replace_turbo_json(tempdir.path(), "2-update-pipeline.json");
    let step2 = global_cache_inputs(tempdir.path());
    assert_eq!(
        baseline, step2,
        "pipeline change should not affect global hash"
    );

    // Update globalEnv: global hash changes
    replace_turbo_json(tempdir.path(), "3-update-global-env.json");
    let step3 = global_cache_inputs(tempdir.path());
    assert_ne!(
        baseline, step3,
        "globalEnv change should affect global hash"
    );

    // Update globalDeps non-materially: global hash stable
    replace_turbo_json(tempdir.path(), "4-update-global-deps.json");
    let step4 = global_cache_inputs(tempdir.path());
    assert_eq!(
        baseline, step4,
        "non-material globalDeps change should not affect global hash"
    );

    // Update globalDeps materially: global hash changes
    replace_turbo_json(tempdir.path(), "5-update-global-deps-materially.json");
    let step5 = global_cache_inputs(tempdir.path());
    assert_ne!(
        baseline, step5,
        "material globalDeps change should affect global hash"
    );

    // Update passThroughEnv: global hash changes
    replace_turbo_json(tempdir.path(), "6-update-passthrough-env.json");
    let step6 = global_cache_inputs(tempdir.path());
    assert_ne!(
        baseline, step6,
        "passThroughEnv change should affect global hash"
    );
}

#[test]
fn test_task_hash_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    replace_turbo_json(tempdir.path(), "a-baseline.json");
    insta::assert_snapshot!("task_hashes_baseline", task_hash_snapshot(tempdir.path()));

    replace_turbo_json(tempdir.path(), "b-change-only-my-app.json");
    insta::assert_snapshot!(
        "task_hashes_change_my_app",
        task_hash_snapshot(tempdir.path())
    );

    replace_turbo_json(tempdir.path(), "c-my-app-depends-on.json");
    insta::assert_snapshot!(
        "task_hashes_my_app_depends_on",
        task_hash_snapshot(tempdir.path())
    );

    replace_turbo_json(tempdir.path(), "d-depends-on-util.json");
    insta::assert_snapshot!(
        "task_hashes_depends_on_util",
        task_hash_snapshot(tempdir.path())
    );

    replace_turbo_json(tempdir.path(), "e-depends-on-util-but-modified.json");
    insta::assert_snapshot!(
        "task_hashes_util_modified",
        task_hash_snapshot(tempdir.path())
    );
}
