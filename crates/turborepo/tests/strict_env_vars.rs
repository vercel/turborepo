mod common;

use std::{fs, path::Path};

use common::{run_turbo, run_turbo_with_env, setup};

fn turbo_configs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../turborepo-tests/integration/fixtures/turbo-configs")
}

fn replace_turbo_json(dir: &Path, config_name: &str) {
    let src = turbo_configs_dir().join(config_name);
    fs::copy(&src, dir.join("turbo.json"))
        .unwrap_or_else(|e| panic!("copy {} failed: {e}", src.display()));
    let normalized = fs::read_to_string(dir.join("turbo.json"))
        .unwrap()
        .replace("\r\n", "\n");
    fs::write(dir.join("turbo.json"), normalized).unwrap();
    std::process::Command::new("git")
        .args([
            "commit",
            "-am",
            "replace turbo.json",
            "--quiet",
            "--allow-empty",
        ])
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
}

/// Get all task hashes as a sorted string. When the global hash (including
/// env mode) changes, all task hashes change.
fn all_task_hashes(dir: &Path, extra_args: &[&str]) -> String {
    let mut args = vec!["run", "build", "--dry=json"];
    args.extend_from_slice(extra_args);
    let output = run_turbo(dir, &args);
    assert!(
        output.status.success(),
        "dry run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut hashes: Vec<String> = json["tasks"]
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
    hashes.sort();
    hashes.join(",")
}

fn setup_strict_env() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "strict_env_vars", "npm@10.5.0", false).unwrap();
    tempdir
}

// --- global-hash-strict.t ---

#[test]
fn test_global_hash_strict() {
    let tempdir = setup_strict_env();

    let baseline = all_task_hashes(tempdir.path(), &[]);
    let with_flag = all_task_hashes(tempdir.path(), &["--env-mode=strict"]);
    // Default is already strict
    assert_eq!(baseline, with_flag);

    // Empty passthrough config → same (empty array is no-op in strict mode)
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt-empty.json");
    let empty_global = all_task_hashes(tempdir.path(), &["--env-mode=strict"]);
    assert_eq!(with_flag, empty_global);

    // Add passthrough value → changes
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt.json");
    let with_global = all_task_hashes(tempdir.path(), &["--env-mode=strict"]);
    assert_ne!(empty_global, with_global);
}

// --- global-hash-loose.t ---

#[test]
fn test_global_hash_loose() {
    let tempdir = setup_strict_env();

    let baseline = all_task_hashes(tempdir.path(), &[]);
    let with_flag = all_task_hashes(tempdir.path(), &["--env-mode=loose"]);
    // Loose differs from default strict
    assert_ne!(baseline, with_flag);

    // Empty passthrough config → same as loose alone (loose doesn't care about
    // config)
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt-empty.json");
    let empty_global = all_task_hashes(tempdir.path(), &["--env-mode=loose"]);
    assert_eq!(with_flag, empty_global);

    // Add passthrough value → still same (loose doesn't care)
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt.json");
    let with_global = all_task_hashes(tempdir.path(), &["--env-mode=loose"]);
    assert_eq!(with_flag, with_global);
}

// --- global-hash-infer.t ---

#[test]
fn test_global_hash_default_mode() {
    // Replaces global-hash-infer.t. The original test used --env-mode=infer
    // which doesn't exist in Rust turbo (only loose/strict). The default mode
    // is strict, so this test verifies config changes affect hashes under
    // the default mode.
    let tempdir = setup_strict_env();

    let baseline = all_task_hashes(tempdir.path(), &[]);

    // Empty passthrough → same (empty array is no-op)
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt-empty.json");
    let empty_global = all_task_hashes(tempdir.path(), &[]);
    assert_eq!(baseline, empty_global);

    // Add passthrough value → changes
    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt.json");
    let with_global = all_task_hashes(tempdir.path(), &[]);
    assert_ne!(empty_global, with_global);
}

// --- usage-strict.t ---

#[test]
fn test_usage_strict() {
    let tempdir = setup_strict_env();

    let env_vars = &[
        ("GLOBAL_VAR_PT", "higlobalpt"),
        ("GLOBAL_VAR_DEP", "higlobaldep"),
        ("LOCAL_VAR_PT", "hilocalpt"),
        ("LOCAL_VAR_DEP", "hilocaldep"),
        ("OTHER_VAR", "hiother"),
        ("SYSTEMROOT", "hisysroot"),
    ];

    let out_path = if cfg!(windows) {
        "apps\\my-app\\out.txt"
    } else {
        "apps/my-app/out.txt"
    };

    // Default config: no vars available in strict mode
    run_turbo_with_env(
        tempdir.path(),
        &["build", "-vv", "--env-mode=strict"],
        env_vars,
    );
    let out = fs::read_to_string(tempdir.path().join(out_path)).unwrap();
    assert!(
        out.contains("globalpt: ''") && out.contains("localpt: ''") && out.contains("other: ''"),
        "strict default: no vars should be available, got: {out}"
    );

    // With all.json: declared vars available, others not
    replace_turbo_json(tempdir.path(), "strict_env_vars/all.json");
    run_turbo_with_env(
        tempdir.path(),
        &["build", "-vv", "--env-mode=strict"],
        env_vars,
    );
    let out = fs::read_to_string(tempdir.path().join(out_path)).unwrap();
    assert!(
        out.contains("globalpt: 'higlobalpt'") && out.contains("localpt: 'hilocalpt'"),
        "strict all.json: declared vars should be available, got: {out}"
    );
    assert!(
        out.contains("other: ''"),
        "strict all.json: OTHER_VAR should NOT be available, got: {out}"
    );
}

// --- usage-loose.t ---

#[test]
fn test_usage_loose() {
    let tempdir = setup_strict_env();

    let env_vars = &[
        ("GLOBAL_VAR_PT", "higlobalpt"),
        ("GLOBAL_VAR_DEP", "higlobaldep"),
        ("LOCAL_VAR_PT", "hilocalpt"),
        ("LOCAL_VAR_DEP", "hilocaldep"),
        ("OTHER_VAR", "hiother"),
        ("SYSTEMROOT", "hisysroot"),
    ];

    let out_path = if cfg!(windows) {
        "apps\\my-app\\out.txt"
    } else {
        "apps/my-app/out.txt"
    };

    // Loose mode: all vars available
    run_turbo_with_env(
        tempdir.path(),
        &["build", "-vv", "--env-mode=loose"],
        env_vars,
    );
    let out = fs::read_to_string(tempdir.path().join(out_path)).unwrap();
    assert!(
        out.contains("globalpt: 'higlobalpt'") && out.contains("other: 'hiother'"),
        "loose: all vars should be available, got: {out}"
    );

    // With all.json: still all vars available
    replace_turbo_json(tempdir.path(), "strict_env_vars/all.json");
    run_turbo_with_env(
        tempdir.path(),
        &["build", "-vv", "--env-mode=loose"],
        env_vars,
    );
    let out = fs::read_to_string(tempdir.path().join(out_path)).unwrap();
    assert!(
        out.contains("globalpt: 'higlobalpt'") && out.contains("other: 'hiother'"),
        "loose all.json: all vars should still be available, got: {out}"
    );
}

// --- dry-json.t ---

#[test]
fn test_strict_env_dry_json() {
    let tempdir = setup_strict_env();

    // Default: passthrough is null
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let env_vars = &json["tasks"][0]["environmentVariables"];
    assert!(env_vars["passthrough"].is_null());
    assert!(env_vars["globalPassthrough"].is_null());

    // With all.json: passthrough is []
    replace_turbo_json(tempdir.path(), "strict_env_vars/all.json");
    let output = run_turbo(tempdir.path(), &["run", "build", "--dry=json"]);
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["taskId"].as_str() == Some("my-app#build"))
        .unwrap();
    let env_vars = &task["environmentVariables"];
    assert!(
        env_vars["passthrough"].is_array(),
        "passthrough should be array"
    );
    assert!(env_vars["globalPassthrough"].is_null());
}
