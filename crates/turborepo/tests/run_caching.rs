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

fn dry_task_hash_reason(output: &std::process::Output, task_id: &str) -> String {
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == task_id)
        .and_then(|task| task["hashReason"].as_str())
        .unwrap()
        .to_string()
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
fn test_structured_jit_inputs_hash_after_dependencies_complete() {
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
      "inputs": [
        "$TURBO_DEFAULT$",
        "!.output/**",
        {
          "mode": "jit",
          "globs": ["src/generated/**"]
        }
      ],
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
    assert!(build_task["hash"].is_null());
    assert_eq!(
        build_task["hashReason"],
        "Deferred because JIT hashing mode was used."
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
fn test_structured_jit_dependency_defers_dependent_hashing() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("packages/util/package.json"),
        r#"{
  "name": "util"
}
"#,
    )
    .unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"require('fs').mkdirSync('.output', { recursive: true }); require('fs').writeFileSync('.output/result.txt', 'built')\""
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
      "dependsOn": ["^build"],
      "cache": false
    },
    "build": {
      "dependsOn": ["^build", "codegen"],
      "inputs": [
        "$TURBO_DEFAULT$",
        "!src/generated/**",
        {
          "mode": "jit",
          "globs": ["src/generated/**"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );

    assert!(
        output.status.success(),
        "expected JIT-dependent task graph to succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_structured_jit_descendant_hashes_after_jit_hash_is_available() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("apps/my-app/marker.txt"), "before\n").unwrap();
    fs::write(tempdir.path().join("apps/my-app/jit-input.txt"), "stable\n").unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "generate": "node -e \"const fs = require('fs'); fs.mkdirSync('.generated', { recursive: true }); fs.writeFileSync('.generated/done.txt', 'done\\n'); fs.writeFileSync('marker.txt', 'after\\n')\"",
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/marker.txt', fs.readFileSync('marker.txt'))\""
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
    "generate": {
      "inputs": [
        {
          "mode": "jit",
          "globs": ["jit-input.txt"]
        }
      ],
      "outputs": [".generated/**"]
    },
    "build": {
      "dependsOn": ["generate"],
      "inputs": ["marker.txt"],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0 cached, 2 total"),
        "expected first run to execute both tasks, got:\n{stdout}"
    );

    fs::write(tempdir.path().join("apps/my-app/marker.txt"), "before\n").unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 cached, 2 total"),
        "expected JIT descendants to be hashed before the dependency command runs, got:\n{stdout}"
    );
}

#[test]
fn test_dependency_outputs_dry_run_reports_deferred_hash() {
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
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!src/generated/**", "!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"]
        }
      ],
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

    assert!(build_task["hash"].is_null());
    assert_eq!(
        build_task["hashReason"],
        "Deferred because dependencyOutputs hashing mode was used."
    );
}

#[test]
fn test_structured_startup_with_defaults_matches_legacy_startup_semantics() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/included.txt"),
        "included-v1\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "excluded-v1\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', fs.readFileSync('included.txt'))\""
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
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!excluded.txt", "!.output/**"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");

    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "excluded-v2\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 1 total"), "got:\n{stdout}");

    fs::write(
        tempdir.path().join("apps/my-app/included.txt"),
        "included-v2\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");
}

#[test]
fn test_structured_jit_with_defaults_uses_deferred_default_inputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/included.txt"),
        "included-v1\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "excluded-v1\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', fs.readFileSync('included.txt'))\""
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
      "inputs": [
        {
          "mode": "jit",
          "withDefaults": true,
          "globs": ["!excluded.txt", "!.output/**"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");

    fs::write(
        tempdir.path().join("apps/my-app/excluded.txt"),
        "excluded-v2\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 1 total"), "got:\n{stdout}");

    fs::write(
        tempdir.path().join("apps/my-app/included.txt"),
        "included-v2\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");
}

#[test]
fn test_structured_startup_inputs_support_turbo_root_globs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("root-config.txt"), "root-v1\n").unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/root-config.txt', fs.readFileSync('../../root-config.txt'))\""
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
      "inputs": [
        {
          "mode": "startup",
          "globs": ["$TURBO_ROOT$/root-config.txt", "!.output/**"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 1 total"), "got:\n{stdout}");

    fs::write(tempdir.path().join("root-config.txt"), "root-v2\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 1 total"), "got:\n{stdout}");
}

#[test]
fn test_structured_jit_inputs_support_turbo_root_globs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("root-counter.txt"), "jit-root-v1\n").unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "generate-root": "node -e \"require('fs').writeFileSync('../../root-generated.txt', require('fs').readFileSync('../../root-counter.txt'))\"",
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/root-generated.txt', fs.readFileSync('../../root-generated.txt'))\""
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
    "generate-root": {
      "cache": false,
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!.output/**"]
        }
      ],
      "outputs": ["$TURBO_ROOT$/root-generated.txt"]
    },
    "build": {
      "dependsOn": ["generate-root"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!.output/**"]
        },
        {
          "mode": "jit",
          "globs": ["$TURBO_ROOT$/root-generated.txt"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 2 total"), "got:\n{stdout}");

    fs::write(tempdir.path().join("root-counter.txt"), "jit-root-v2\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");
}

#[test]
fn test_package_config_extends_inputs_before_structured_normalization() {
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
      "inputs": ["$TURBO_DEFAULT$", "!.output/**"],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "build": {
      "inputs": [
        "$TURBO_EXTENDS$",
        {
          "mode": "jit",
          "globs": ["src/generated/**"]
        }
      ]
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

    assert_eq!(
        build_task["hashReason"],
        "Deferred because JIT hashing mode was used."
    );
}

#[test]
fn test_package_config_extends_rejects_duplicate_startup_after_normalization() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": ["$TURBO_DEFAULT$"],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/turbo.json"),
        r#"{
  "extends": ["//"],
  "tasks": {
    "build": {
      "inputs": [
        "$TURBO_EXTENDS$",
        {
          "mode": "startup",
          "globs": ["src/**"]
        }
      ]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Legacy input strings normalize to mode \"startup\""),
        "expected extends duplicate startup normalization error, got:\n{stderr}"
    );
}

#[test]
fn test_dependency_outputs_without_from_selects_direct_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "codegen": "echo codegen",
    "build": "echo build"
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
      "outputs": ["src/generated/**"]
    },
    "build": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "dependencyOutputs"
        }
      ],
      "outputs": ["dist/**"]
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
    assert!(
        output.status.success(),
        "expected default dependencyOutputs selection to use direct \
         dependencies\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        dry_task_hash_reason(&output, "my-app#build"),
        "Deferred because dependencyOutputs hashing mode was used."
    );
}

#[test]
fn test_dependency_outputs_from_selects_topological_dependencies() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "outputs": ["dist/**"]
    },
    "my-app#build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["^build"]
        }
      ],
      "outputs": ["dist/**"]
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
    assert!(
        output.status.success(),
        "expected dependencyOutputs.from to select ^build dependency \
         nodes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        dry_task_hash_reason(&output, "my-app#build"),
        "Deferred because dependencyOutputs hashing mode was used."
    );
}

#[test]
fn test_dependency_outputs_from_allows_empty_selector_matches() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["^build"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(
        output.status.success(),
        "expected terminal dependencyOutputs.from selector matches to be \
         ignored\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_dependency_outputs_from_must_match_existing_dependency_task_when_dependencies_exist() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["^codegen"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match any eligible dependency task node"),
        "expected dependencyOutputs.from validation error, got:\n{stderr}"
    );
}

#[test]
fn test_dependency_outputs_from_selects_package_qualified_dependency_node() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["util#build"]
        }
      ],
      "outputs": ["dist/**"]
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
    assert!(
        output.status.success(),
        "expected package-qualified dependencyOutputs.from to select node from ^build \
         expansion\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        dry_task_hash_reason(&output, "my-app#build"),
        "Deferred because dependencyOutputs hashing mode was used."
    );
}

#[test]
fn test_dependency_outputs_from_hashes_package_qualified_dependency_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("packages/util/seed.txt"), "v1\n").unwrap();
    fs::write(
        tempdir.path().join("packages/util/package.json"),
        r#"{
  "name": "util",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.writeFileSync('dist/generated.txt', fs.readFileSync('seed.txt'))\""
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', 'build\\n')\""
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
      "inputs": ["$TURBO_DEFAULT$", "!dist/**"],
      "outputs": ["dist/**"]
    },
    "my-app#build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["util#build"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2 cached, 2 total"), "got:\n{stdout}");

    fs::write(tempdir.path().join("packages/util/seed.txt"), "v2\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");
}

#[test]
fn test_dependency_outputs_replaces_selected_dependency_task_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("packages/util/seed.txt"), "v1\n").unwrap();
    fs::write(
        tempdir.path().join("packages/util/package.json"),
        r#"{
  "name": "util",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.writeFileSync('dist/generated.txt', 'stable\\n')\""
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', 'build\\n')\""
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
      "inputs": ["$TURBO_DEFAULT$", "!dist/**"],
      "outputs": ["dist/**"]
    },
    "my-app#build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["^build"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");

    fs::write(tempdir.path().join("packages/util/seed.txt"), "v2\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 2 total"), "got:\n{stdout}");
}

#[test]
fn test_dependency_outputs_distinguishes_cross_package_output_paths() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("packages/another/seed.txt"),
        "another-v1\n",
    )
    .unwrap();
    fs::write(tempdir.path().join("packages/util/seed.txt"), "util-v1\n").unwrap();
    fs::write(
        tempdir.path().join("packages/another/package.json"),
        r#"{
  "name": "another",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.writeFileSync('dist/generated.txt', fs.readFileSync('seed.txt'))\""
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("packages/util/package.json"),
        r#"{
  "name": "util",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('dist', { recursive: true }); fs.writeFileSync('dist/generated.txt', fs.readFileSync('seed.txt'))\""
  }
}
"#,
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "build": "node -e \"const fs = require('fs'); fs.mkdirSync('.output', { recursive: true }); fs.writeFileSync('.output/result.txt', 'build\\n')\""
  },
  "dependencies": {
    "another": "*",
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
    "another#build": {
      "cache": false,
      "inputs": ["$TURBO_DEFAULT$", "!seed.txt", "!dist/**"],
      "outputs": ["dist/**"]
    },
    "util#build": {
      "cache": false,
      "inputs": ["$TURBO_DEFAULT$", "!seed.txt", "!dist/**"],
      "outputs": ["dist/**"]
    },
    "my-app#build": {
      "dependsOn": ["^build"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["^build"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 3 total"), "got:\n{stdout}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 3 total"), "got:\n{stdout}");

    fs::write(
        tempdir.path().join("packages/another/seed.txt"),
        "another-v2\n",
    )
    .unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 3 total"), "got:\n{stdout}");
}

#[test]
fn test_dependency_outputs_from_can_select_transitive_dependency_node() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "codegen": "echo codegen",
    "generate": "echo generate",
    "build": "echo build"
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
      "outputs": ["src/generated/**"]
    },
    "generate": {
      "dependsOn": ["codegen"],
      "outputs": ["src/generated-wrapper/**"]
    },
    "build": {
      "dependsOn": ["generate"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"]
        }
      ],
      "outputs": ["dist/**"]
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
    assert!(
        output.status.success(),
        "expected explicit dependencyOutputs.from to select transitive dependency subgraph \
         nodes\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        dry_task_hash_reason(&output, "my-app#build"),
        "Deferred because dependencyOutputs hashing mode was used."
    );
}

#[test]
fn test_dependency_outputs_hashes_materialized_dependency_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("apps/my-app/seed.txt"), "v1\n").unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "codegen": "node -e \"const fs = require('fs'); fs.mkdirSync('src/generated', { recursive: true }); fs.writeFileSync('src/generated/schema.txt', fs.readFileSync('seed.txt'))\"",
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
      "cache": false,
      "inputs": ["$TURBO_DEFAULT$", "!seed.txt", "!src/generated/**", "!.output/**"],
      "outputs": ["src/generated/**"]
    },
    "build": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!seed.txt", "!src/generated/**", "!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");

    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 cached, 2 total"), "got:\n{stdout}");

    fs::write(tempdir.path().join("apps/my-app/seed.txt"), "v2\n").unwrap();
    let output = run_turbo(
        tempdir.path(),
        &["run", "build", "--filter=my-app", "--output-logs=none"],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("0 cached, 2 total"), "got:\n{stdout}");
}

#[test]
fn test_dependency_outputs_globs_cannot_select_undeclared_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(tempdir.path().join("apps/my-app/seed.txt"), "v1\n").unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/private.txt"),
        "private-v1\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("apps/my-app/package.json"),
        r#"{
  "name": "my-app",
  "scripts": {
    "codegen": "node -e \"const fs = require('fs'); fs.mkdirSync('src/generated', { recursive: true }); fs.writeFileSync('src/generated/schema.txt', fs.readFileSync('seed.txt')); fs.writeFileSync('private.txt', fs.readFileSync('private.txt'))\"",
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
      "cache": false,
      "inputs": ["$TURBO_DEFAULT$", "!seed.txt", "!private.txt", "!src/generated/**", "!.output/**"],
      "outputs": ["src/generated/**"]
    },
    "build": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!seed.txt", "!private.txt", "!src/generated/**", "!.output/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"],
          "globs": ["src/generated/**", "private.txt"]
        }
      ],
      "outputs": [".output/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dependencyOutputs.globs") && stderr.contains("private.txt"),
        "expected dependencyOutputs.globs validation error, got:\n{stderr}"
    );
}

#[test]
fn test_dependency_outputs_selected_dependency_must_declare_outputs() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", true).unwrap();

    fs::write(
        tempdir.path().join("turbo.json"),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "codegen": {},
    "build": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"]
        }
      ],
      "outputs": ["dist/**"]
    }
  }
}
"#,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["run", "build", "--filter=my-app"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not declare outputs"),
        "expected selected dependency outputs validation error, got:\n{stderr}"
    );
    assert!(
        stderr.contains("Add outputs to")
            && stderr.contains("or remove it from dependencyOutputs.from"),
        "expected remediation guidance, got:\n{stderr}"
    );
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
