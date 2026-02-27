mod common;

use std::{fs, path::Path};

use common::{run_turbo, setup};

fn ls_dir(dir: &Path) -> Vec<String> {
    let mut entries: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    entries.sort();
    entries
}

// --- docker.t ---

#[test]
fn test_prune_docker() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web", "--docker"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added web"));

    // out/json contents
    let json_entries = ls_dir(&tempdir.path().join("out/json"));
    assert_eq!(
        json_entries,
        vec![
            ".npmrc",
            "apps",
            "package.json",
            "packages",
            "patches",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml"
        ]
    );

    // out/full contents
    let full_entries = ls_dir(&tempdir.path().join("out/full"));
    assert_eq!(
        full_entries,
        vec![
            ".npmrc",
            "apps",
            "package.json",
            "packages",
            "patches",
            "pnpm-workspace.yaml",
            "turbo.json"
        ]
    );

    // out contents
    let out_entries = ls_dir(&tempdir.path().join("out"));
    assert_eq!(
        out_entries,
        vec!["full", "json", "pnpm-lock.yaml", "pnpm-workspace.yaml"]
    );

    // pnpm patches in package.json
    let pkg_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(tempdir.path().join("out/json/package.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        pkg_json["pnpm"]["patchedDependencies"]["is-number@7.0.0"],
        "patches/is-number@7.0.0.patch"
    );
}

// --- out-dir.t ---

#[test]
fn test_prune_out_dir() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    let output = run_turbo(
        tempdir.path(),
        &[
            "prune",
            "web",
            &format!("--out-dir={}", out_dir.path().display()),
        ],
    );
    assert!(output.status.success());

    let pkg_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(out_dir.path().join("package.json")).unwrap())
            .unwrap();
    assert_eq!(pkg_json["name"], "monorepo");
    assert_eq!(pkg_json["packageManager"], "pnpm@7.25.1");
    assert_eq!(
        pkg_json["pnpm"]["patchedDependencies"]["is-number@7.0.0"],
        "patches/is-number@7.0.0.patch"
    );
}

// --- produces-valid-turbo-json.t ---

#[test]
fn test_prune_produces_valid_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    // Prune docs
    let output = run_turbo(tempdir.path(), &["prune", "docs"]);
    assert!(output.status.success());

    // Pruned turbo.json should not have tasks referencing pruned workspaces
    let pruned_turbo: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tempdir.path().join("out/turbo.json")).unwrap())
            .unwrap();
    assert!(pruned_turbo["tasks"]["build"].is_object());

    // Verify turbo can read the produced turbo.json
    let output = run_turbo(&tempdir.path().join("out"), &["build", "--dry=json"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut packages: Vec<String> = json["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    packages.sort();
    assert_eq!(packages, vec!["docs", "shared", "util"]);

    // Add remoteCache fields
    let _ = fs::remove_dir_all(tempdir.path().join("out"));
    let turbo_json_path = tempdir.path().join("turbo.json");
    // Strip comments (turbo.json has // comments which aren't valid JSON)
    let raw = fs::read_to_string(&turbo_json_path).unwrap();
    let stripped: String = raw
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut turbo: serde_json::Value = serde_json::from_str(&stripped).unwrap();
    turbo["remoteCache"] = serde_json::json!({
        "enabled": true,
        "timeout": 1000,
        "apiUrl": "my-domain.com/cache"
    });
    fs::write(
        &turbo_json_path,
        serde_json::to_string_pretty(&turbo).unwrap(),
    )
    .unwrap();

    run_turbo(tempdir.path(), &["prune", "docs"]);
    let pruned: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tempdir.path().join("out/turbo.json")).unwrap())
            .unwrap();
    assert_eq!(pruned["remoteCache"]["enabled"], true);
    assert_eq!(pruned["remoteCache"]["timeout"], 1000);
    assert_eq!(pruned["remoteCache"]["apiUrl"], "my-domain.com/cache");

    // Set enabled to false
    let _ = fs::remove_dir_all(tempdir.path().join("out"));
    turbo["remoteCache"]["enabled"] = serde_json::json!(false);
    fs::write(
        &turbo_json_path,
        serde_json::to_string_pretty(&turbo).unwrap(),
    )
    .unwrap();

    run_turbo(tempdir.path(), &["prune", "docs"]);
    let pruned: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(tempdir.path().join("out/turbo.json")).unwrap())
            .unwrap();
    assert_eq!(pruned["remoteCache"]["enabled"], false);
}

// --- composable-config.t ---

#[test]
fn test_prune_composable_config() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        true,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "docs"]);
    assert!(output.status.success());

    // Run turbo inside pruned output
    let output = run_turbo(&tempdir.path().join("out"), &["run", "new-task"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("docs:new-task:"));
    assert!(stdout.contains("building"));
    assert!(stdout.contains("1 successful, 1 total"));
}

// --- includes-root-deps.t ---

#[test]
fn test_prune_includes_root_deps() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Added web"));

    // Rename to turbo.jsonc, prune again
    fs::rename(
        tempdir.path().join("turbo.json"),
        tempdir.path().join("turbo.jsonc"),
    )
    .unwrap();
    let _ = fs::remove_dir_all(tempdir.path().join("out"));
    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(output.status.success());

    let out_entries = ls_dir(&tempdir.path().join("out"));
    assert!(
        out_entries.contains(&"turbo.jsonc".to_string()),
        "turbo.jsonc should be in output: {out_entries:?}"
    );
}

// --- resolutions.t ---

#[test]
fn test_prune_resolutions() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("berry_resolutions", tempdir.path()).unwrap();

    // Prune a: should have resolved is-odd
    let output = run_turbo(tempdir.path(), &["prune", "a"]);
    assert!(output.status.success());
    let yarn_lock = fs::read_to_string(tempdir.path().join("out/yarn.lock")).unwrap();
    assert!(yarn_lock.contains("\"is-odd@npm:3.0.0\":"));
    assert!(yarn_lock.contains("resolution: \"is-odd@npm:3.0.0\""));

    // Prune b: should NOT have the override
    let output = run_turbo(tempdir.path(), &["prune", "b"]);
    assert!(output.status.success());
    let yarn_lock = fs::read_to_string(tempdir.path().join("out/yarn.lock")).unwrap();
    assert!(yarn_lock.contains("\"is-odd@npm:^3.0.1\":"));
    assert!(yarn_lock.contains("resolution: \"is-odd@npm:3.0.1\""));
}

// --- yarn-pnp.t ---

#[test]
fn test_prune_yarn_pnp() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("berry_resolutions", tempdir.path()).unwrap();

    // Remove linker override
    let _ = fs::remove_file(tempdir.path().join(".yarnrc.yml"));

    let output = run_turbo(tempdir.path(), &["prune", "a"]);
    assert!(output.status.success());

    // .pnp.cjs should NOT be in output
    let out_entries = ls_dir(&tempdir.path().join("out"));
    assert!(
        !out_entries.contains(&".pnp.cjs".to_string()),
        ".pnp.cjs should not be in output: {out_entries:?}"
    );
    assert_eq!(out_entries, vec!["package.json", "packages", "yarn.lock"]);
}
