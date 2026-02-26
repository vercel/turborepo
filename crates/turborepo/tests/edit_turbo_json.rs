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

fn get_task_hashes(dir: &Path) -> Vec<(String, String)> {
    let output = run_turbo(dir, &["build", "--dry=json"]);
    assert!(
        output.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut tasks: Vec<(String, String)> = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["taskId"].as_str().unwrap().to_string(),
                t["hash"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    tasks.sort();
    tasks
}

fn hash_for<'a>(tasks: &'a [(String, String)], task_id: &str) -> &'a str {
    &tasks.iter().find(|(id, _)| id == task_id).unwrap().1
}

/// Extract globalCacheInputs from --dry=json as a stable string for comparison.
/// This is the input to the global hash, so if it changes, the global hash
/// changes.
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

// --- global.t ---
// The original prysk test extracted a "global hash" from debug output via
// find_global_hash.sh. That script was designed for Go turbo which printed
// "global hash: value=<hex>". Rust turbo doesn't print this, so the original
// test was a no-op. This version tests the same invariants using
// globalCacheInputs from --dry=json.

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

// --- task.t ---

#[test]
fn test_task_hash_changes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Baseline
    replace_turbo_json(tempdir.path(), "a-baseline.json");
    let step1 = get_task_hashes(tempdir.path());
    assert_eq!(hash_for(&step1, "another#build"), "e9a99dd97d223d88");
    assert_eq!(hash_for(&step1, "my-app#build"), "0555ce94ca234049");
    assert_eq!(hash_for(&step1, "util#build"), "bf1798d3e46e1b48");

    // Change only my-app
    replace_turbo_json(tempdir.path(), "b-change-only-my-app.json");
    let step2 = get_task_hashes(tempdir.path());
    assert_eq!(hash_for(&step2, "another#build"), "e9a99dd97d223d88");
    assert_eq!(hash_for(&step2, "my-app#build"), "6eea03fab6f9a8c8");
    assert_eq!(hash_for(&step2, "util#build"), "bf1798d3e46e1b48");

    // Change my-app dependsOn
    replace_turbo_json(tempdir.path(), "c-my-app-depends-on.json");
    let step3 = get_task_hashes(tempdir.path());
    assert_eq!(hash_for(&step3, "another#build"), "e9a99dd97d223d88");
    assert_eq!(hash_for(&step3, "my-app#build"), "8637a0f5db686164");
    assert_eq!(hash_for(&step3, "util#build"), "bf1798d3e46e1b48");

    // Non-material dep graph change — same as step 3
    replace_turbo_json(tempdir.path(), "d-depends-on-util.json");
    let step4 = get_task_hashes(tempdir.path());
    assert_eq!(hash_for(&step4, "another#build"), "e9a99dd97d223d88");
    assert_eq!(hash_for(&step4, "my-app#build"), "8637a0f5db686164");
    assert_eq!(hash_for(&step4, "util#build"), "bf1798d3e46e1b48");

    // Change util#build — impacts itself and my-app
    replace_turbo_json(tempdir.path(), "e-depends-on-util-but-modified.json");
    let step5 = get_task_hashes(tempdir.path());
    assert_eq!(hash_for(&step5, "another#build"), "e9a99dd97d223d88");
    assert_eq!(hash_for(&step5, "my-app#build"), "2721f01b53b758d0");
    assert_eq!(hash_for(&step5, "util#build"), "74c8eb9bab702b4b");
}
