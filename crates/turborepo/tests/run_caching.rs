mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, run_turbo_with_env, setup};

/// Extract a hash from a log line like "cache miss, executing abc123def"
fn extract_hash<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| {
            let idx = line.find(prefix)?;
            let rest = &line[idx + prefix.len()..];
            Some(rest.split_whitespace().next().unwrap_or(rest.trim()))
        })
        .expect("could not find hash in output")
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

    // Warm cache: cache miss — snapshot the hash
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    let initial_hash = extract_hash(&stdout, "cache miss, executing ");
    insta::assert_snapshot!("root_deps_initial_hash", initial_hash);

    // Cache hit — same hash
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("cache hit, suppressing logs {initial_hash}")));
    assert!(stdout.contains("FULL TURBO"));

    // Touch a root internal dependency → cache miss with DIFFERENT hash
    fs::write(tempdir.path().join("packages/util/important.txt"), "").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    let new_hash = extract_hash(&stdout, "cache miss, executing ");
    assert_ne!(
        initial_hash, new_hash,
        "hash should change after root dep change"
    );
    insta::assert_snapshot!("root_deps_changed_hash", new_hash);

    // All packages in scope after root dep change
    let output = run_turbo(tempdir.path(), &["build", "--filter=[HEAD]", "--dry=json"]);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["packages"],
        serde_json::json!(["//", "another", "my-app", "util", "yet-another"])
    );

    // Touch gitignored file → still cache hit with SAME hash
    fs::create_dir_all(tempdir.path().join("packages/util/dist")).unwrap();
    fs::write(tempdir.path().join("packages/util/dist/unused.txt"), "").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["build", "--filter=another", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("cache hit, suppressing logs {new_hash}")));

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
    git(
        tempdir.path(),
        &["commit", "-am", "remove comments", "--quiet"],
    );

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
    git(
        tempdir.path(),
        &[
            "commit",
            "-am",
            "add empty remote caching config",
            "--quiet",
        ],
    );

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
    git(
        tempdir.path(),
        &["commit", "-am", "disable remote caching", "--quiet"],
    );

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
    git(
        tempdir.path(),
        &[
            "commit",
            "-am",
            "Update turbo.json to include special inputs config",
            "--quiet",
        ],
    );

    // Run 1: cache miss — capture hash
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));
    let hash = extract_hash(&stdout, "cache miss, executing ");
    insta::assert_snapshot!("excluded_inputs_hash", hash);
    assert!(stdout.contains("0 cached, 1 total"));

    // Modify excluded file
    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "new excluded value\n",
    )
    .unwrap();

    // Run 2: still cache hit with SAME hash (excluded file doesn't affect hash)
    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("cache hit, replaying logs {hash}")));
    assert!(stdout.contains("1 cached, 1 total"));
    assert!(stdout.contains("FULL TURBO"));
}
