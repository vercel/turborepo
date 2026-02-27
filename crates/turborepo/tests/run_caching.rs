mod common;

use std::{fs, path::Path};

use common::{run_turbo, run_turbo_with_env, setup};

fn git_commit(dir: &Path, msg: &str) {
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-am", msg, "--quiet", "--allow-empty"])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

// --- global-deps.t ---

#[test]
fn test_global_deps_caching() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Run 1: with env var → cache miss
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--output-logs=none", "--filter=my-app"],
        &[("SOME_ENV_VAR", "hi")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"));

    // Run 2: same env var → cache hit
    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--output-logs=none", "--filter=my-app"],
        &[("SOME_ENV_VAR", "hi")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 1 total"));
    assert!(stdout.contains("FULL TURBO"));

    // Run 3: without env var → cache miss
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--output-logs=none", "--filter=my-app"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"));
}

// --- root-deps.t ---

#[test]
fn test_root_deps_caching() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "root_deps", "npm@10.5.0", true).unwrap();

    // No packages in scope at HEAD
    let output = run_turbo(tempdir.path(), &["build", "--filter=[HEAD]", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["packages"], serde_json::json!([]));

    // Warm cache: cache miss, hash 6a4c300cb14847b0
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing 6a4c300cb14847b0"));

    // Cache hit
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, suppressing logs 6a4c300cb14847b0"));
    assert!(stdout.contains("FULL TURBO"));

    // Touch a root internal dependency → cache miss with new hash
    fs::write(tempdir.path().join("packages/util/important.txt"), "").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing 34787620f332fb95"));

    // All packages in scope after root dep change
    let output = run_turbo(tempdir.path(), &["build", "--filter=[HEAD]", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["packages"],
        serde_json::json!(["//", "another", "my-app", "util", "yet-another"])
    );

    // Touch gitignored file → still cache hit
    fs::create_dir_all(tempdir.path().join("packages/util/dist")).unwrap();
    fs::write(tempdir.path().join("packages/util/dist/unused.txt"), "").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, suppressing logs 34787620f332fb95"));

    // Dependants of root dep
    let output = run_turbo(tempdir.path(), &["build", "--filter=...util", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["packages"],
        serde_json::json!(["//", "another", "my-app", "util", "yet-another"])
    );

    // Dependencies of another
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another...", "--dry=json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["packages"],
        serde_json::json!(["another", "util", "yet-another"])
    );
}

// --- remote-caching-enable.t ---

#[test]
fn test_remote_caching_enable() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Strip comments from turbo.json
    let turbo_json_path = tempdir.path().join("turbo.json");
    let contents = fs::read_to_string(&turbo_json_path).unwrap();
    let stripped: String = contents
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = stripped.replace("\r\n", "\n");
    fs::write(&turbo_json_path, &normalized).unwrap();
    git_commit(tempdir.path(), "remove comments");

    // No remoteCache config → enabled by default
    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--team=vercel",
            "--token=hi",
            "--output-logs=none",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remote caching enabled"));

    // Add empty remoteCache → still enabled
    let mut json: serde_json::Value = serde_json::from_str(&normalized).unwrap();
    json["remoteCache"] = serde_json::json!({});
    let new_contents = serde_json::to_string_pretty(&json).unwrap() + "\n";
    let new_normalized = new_contents.replace("\r\n", "\n");
    fs::write(&turbo_json_path, &new_normalized).unwrap();
    git_commit(tempdir.path(), "add empty remote caching config");

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--team=vercel",
            "--token=hi",
            "--output-logs=none",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remote caching enabled"));

    // Set remoteCache.enabled = false → disabled
    let mut json: serde_json::Value = serde_json::from_str(&new_normalized).unwrap();
    json["remoteCache"]["enabled"] = serde_json::json!(false);
    let disabled_contents = serde_json::to_string_pretty(&json).unwrap() + "\n";
    let disabled_normalized = disabled_contents.replace("\r\n", "\n");
    fs::write(&turbo_json_path, &disabled_normalized).unwrap();
    git_commit(tempdir.path(), "disable remote caching");

    let output = run_turbo(
        tempdir.path(),
        &[
            "run",
            "build",
            "--team=vercel",
            "--token=hi",
            "--output-logs=none",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Remote caching disabled"));
}

// --- cache-state.t ---

#[test]
fn test_cache_state() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Warm cache
    run_turbo(tempdir.path(), &["run", "build", "--output-logs=none"]);

    // Dry run to get cache state
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Get my-app#build hash and check cache meta
    let my_app_task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"] == "my-app#build")
        .unwrap();

    let hash = my_app_task["hash"].as_str().unwrap();

    // Read cache meta file
    let meta_path = tempdir
        .path()
        .join(format!(".turbo/cache/{hash}-meta.json"));
    let meta_contents = fs::read_to_string(&meta_path).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_contents).unwrap();
    let duration = meta["duration"].as_u64().unwrap();
    assert!(duration > 0, "cache duration should be > 0, got {duration}");

    // Validate cache state in dry run
    let cache = &my_app_task["cache"];
    assert_eq!(cache["local"], true);
    assert_eq!(cache["remote"], false);
    assert_eq!(cache["status"], "HIT");
    assert_eq!(cache["source"], "LOCAL");
}

// --- excluded-inputs.t ---

#[test]
fn test_excluded_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Copy the special turbo.json with excluded inputs config
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/tests/run-caching/excluded-inputs/turbo.json");
    fs::copy(&src, tempdir.path().join("turbo.json")).unwrap();
    git_commit(
        tempdir.path(),
        "Update turbo.json to include special inputs config",
    );

    // Run 1: cache miss
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing e228bd94fd46352c"));
    assert!(stdout.contains("0 cached, 1 total"));

    // Modify excluded file
    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "new excluded value\n",
    )
    .unwrap();

    // Run 2: still cache hit (excluded file doesn't affect hash)
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache hit, replaying logs e228bd94fd46352c"));
    assert!(stdout.contains("1 cached, 1 total"));
    assert!(stdout.contains("FULL TURBO"));
}
