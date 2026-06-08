#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::fs;

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

#[test]
fn test_basic_monorepo_cache_behaviors() {
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

    let output = run_turbo_with_env(
        tempdir.path(),
        &["run", "build", "--output-logs=none"],
        &[("TURBO_CACHE", "remote:rw")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Remote caching disabled (remote cache requested"),
        "Expected 'remote cache requested' message, got:\n{stdout}"
    );
}

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

// --- excluded-inputs.t ---

#[test]
fn test_excluded_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    // Copy the special turbo.json with excluded inputs config
    let src = common::manifest_dir()
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

#[test]
fn test_jit_inputs_hash_after_dependencies_complete() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "codegen": "node -e \"require('fs').mkdirSync('src/generated', { recursive: true }); require('fs').writeFileSync('src/generated/schema.txt', 'schema-v1\\n')\"",
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', fs.readFileSync('src/generated/schema.txt'))\""
  },
  "dependencies": {
    "util": "*"
  }
}
"#,
    )
    .unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "codegen": {
      "inputs": ["$TURBO_DEFAULT$", "!src/generated/**", "!.output/**"],
      "outputs": ["src/generated/**"]
    },
    "build": {
      "dependsOn": ["codegen"],
      "inputs": ["$TURBO_DEFAULT$", "!.output/**", "$TURBO_JIT$/src/generated/**"],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--dry=json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let build_task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == "my-app#build")
        .unwrap();
    assert_eq!(build_task["hash"], "Deferred because $TURBO_JIT$ was used.");
    assert_eq!(
        build_task["hashReason"],
        "Deferred because $TURBO_JIT$ was used."
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 cached, 2 total"),
        "expected first run to execute both tasks, got:\n{stdout}"
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 cached, 2 total"),
        "expected second run to cache both tasks after one run, got:\n{stdout}"
    );
    assert!(stdout.contains("FULL TURBO"));
}

#[test]
fn test_gitignored_output_deletion_restores_from_cache() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"require('fs').mkdirSync('.output', { recursive: true }); require('fs').writeFileSync('.output/result.txt', 'built\\n')\"",
    "maybefails": "exit 4"
  },
  "dependencies": {
    "util": "*"
  }
}
"#,
    )
    .unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    fs::write(tempdir.path().join("apps/my-app/.gitignore"), ".output\n").unwrap();

    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &[
            "commit",
            "-m",
            "configure gitignored build output",
            "--quiet",
        ],
    );

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("cache miss, executing"));

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("FULL TURBO"));

    let output_dir = tempdir.path().join("apps/my-app/.output");
    fs::remove_dir_all(&output_dir).unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=hash-only"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FULL TURBO"),
        "expected deleted gitignored output to restore from cache, got: {stdout}"
    );
    assert!(
        output_dir.join("result.txt").exists(),
        "expected cache hit to restore deleted output"
    );
}
