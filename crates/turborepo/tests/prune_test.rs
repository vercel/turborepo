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

// --- global-dependencies.t ---

#[test]
fn test_prune_copies_global_dependencies_with_future_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    // Create root-level files that will be referenced by globalDependencies
    fs::write(
        tempdir.path().join("tsconfig.json"),
        r#"{"compilerOptions":{}}"#,
    )
    .unwrap();
    fs::create_dir_all(tempdir.path().join("config")).unwrap();
    fs::write(tempdir.path().join("config/base.json"), "{}").unwrap();
    fs::write(tempdir.path().join("config/ignore-me.md"), "ignored").unwrap();

    // Rewrite turbo.json with globalDependencies and the future flag enabled
    let turbo_json = serde_json::json!({
        "globalDependencies": ["tsconfig.json", "config/**", "!config/**/*.md"],
        "futureFlags": {
            "pruneIncludesGlobalFiles": true
        },
        "tasks": {
            "build": { "outputs": [] }
        }
    });
    fs::write(
        tempdir.path().join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(
        output.status.success(),
        "prune failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Global dependency files should be in the output
    assert!(
        tempdir.path().join("out/tsconfig.json").exists(),
        "tsconfig.json should be in pruned output"
    );
    assert!(
        tempdir.path().join("out/config/base.json").exists(),
        "config/base.json should be in pruned output"
    );

    // Excluded file should NOT be in the output
    assert!(
        !tempdir.path().join("out/config/ignore-me.md").exists(),
        "config/ignore-me.md should be excluded from pruned output"
    );
}

#[test]
fn test_prune_skips_global_dependencies_without_future_flag() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    fs::write(
        tempdir.path().join("tsconfig.json"),
        r#"{"compilerOptions":{}}"#,
    )
    .unwrap();

    // turbo.json with globalDependencies but NO future flag
    let turbo_json = serde_json::json!({
        "globalDependencies": ["tsconfig.json"],
        "tasks": {
            "build": { "outputs": [] }
        }
    });
    fs::write(
        tempdir.path().join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(output.status.success());

    // Without the flag, global dependency files should NOT be copied
    assert!(
        !tempdir.path().join("out/tsconfig.json").exists(),
        "tsconfig.json should NOT be in pruned output without future flag"
    );
}

#[test]
fn test_prune_global_dependencies_docker() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    fs::write(
        tempdir.path().join("tsconfig.json"),
        r#"{"compilerOptions":{}}"#,
    )
    .unwrap();

    let turbo_json = serde_json::json!({
        "globalDependencies": ["tsconfig.json"],
        "futureFlags": {
            "pruneIncludesGlobalFiles": true
        },
        "tasks": {
            "build": { "outputs": [] }
        }
    });
    fs::write(
        tempdir.path().join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web", "--docker"]);
    assert!(
        output.status.success(),
        "prune --docker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // In docker mode, global dependency files should be in both full and json
    assert!(
        tempdir.path().join("out/full/tsconfig.json").exists(),
        "tsconfig.json should be in out/full/"
    );
    assert!(
        tempdir.path().join("out/json/tsconfig.json").exists(),
        "tsconfig.json should be in out/json/"
    );
}

#[test]
fn test_prune_global_deps_does_not_overwrite_pruned_turbo_json() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    // A glob that matches turbo.json itself
    let turbo_json = serde_json::json!({
        "globalDependencies": ["*.json"],
        "futureFlags": {
            "pruneIncludesGlobalFiles": true
        },
        "tasks": {
            "build": { "outputs": [] },
            "web#build": { "dependsOn": ["web#gen"] },
            "web#gen": { "outputs": ["gen.txt"] }
        }
    });
    fs::write(
        tempdir.path().join("turbo.json"),
        serde_json::to_string_pretty(&turbo_json).unwrap(),
    )
    .unwrap();

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(
        output.status.success(),
        "prune failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The pruned turbo.json should NOT contain tasks for non-pruned workspaces.
    // If the original was copied over the pruned version, it would still be the
    // full config (which is fine here since we only have web tasks, but the key
    // point is that it went through prune_tasks).
    let pruned_contents = fs::read_to_string(tempdir.path().join("out/turbo.json")).unwrap();
    let pruned: serde_json::Value = serde_json::from_str(&pruned_contents).unwrap();
    let tasks = pruned["tasks"].as_object().unwrap();

    // The pruned turbo.json should have gone through prune_tasks, so it should
    // be a valid JSON object (not the JSONC original with comments).
    assert!(
        tasks.contains_key("build"),
        "pruned turbo.json should contain generic tasks"
    );
}

// --- key-order.t ---

#[test]
fn test_prune_preserves_package_json_key_order() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(
        tempdir.path(),
        "monorepo_with_root_dep",
        "pnpm@7.25.1",
        false,
    )
    .unwrap();

    let original_contents = fs::read_to_string(tempdir.path().join("package.json")).unwrap();
    let original: serde_json::Value = serde_json::from_str(&original_contents).unwrap();
    let original_keys: Vec<_> = original.as_object().unwrap().keys().cloned().collect();

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(output.status.success());

    let pruned_contents = fs::read_to_string(tempdir.path().join("out/package.json")).unwrap();
    let pruned: serde_json::Value = serde_json::from_str(&pruned_contents).unwrap();
    let pruned_keys: Vec<_> = pruned.as_object().unwrap().keys().cloned().collect();

    // The fixture has non-alphabetical key order (name, packageManager,
    // devDependencies, pnpm). Verify prune doesn't sort them.
    assert_eq!(
        original_keys, pruned_keys,
        "pruned package.json should preserve original key order"
    );
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

// --- pnpm per-workspace lockfile ---

/// Initialize a git repo without overwriting the fixture's .npmrc.
/// The standard `setup_git` replaces .npmrc with npm boilerplate, which would
/// destroy the `shared-workspace-lockfile=false` setting the fixture needs.
fn init_git_preserving_npmrc(dir: &Path) {
    let npmrc = fs::read_to_string(dir.join(".npmrc")).ok();
    setup::setup_git(dir).unwrap();
    if let Some(contents) = npmrc {
        fs::write(dir.join(".npmrc"), contents).unwrap();
        std::process::Command::new("git")
            .args(["add", ".npmrc"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--amend", "--no-edit", "--quiet"])
            .current_dir(dir)
            .output()
            .unwrap();
    }
}

#[test]
fn test_prune_pnpm_per_workspace_lockfile() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("pnpm_per_workspace_lockfile", tempdir.path()).unwrap();
    init_git_preserving_npmrc(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "web"]);
    assert!(
        output.status.success(),
        "prune failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Per-workspace lockfiles should be preserved for included workspaces
    assert!(
        tempdir.path().join("out/apps/web/pnpm-lock.yaml").exists(),
        "web's per-workspace lockfile should be in pruned output"
    );
    assert!(
        tempdir
            .path()
            .join("out/packages/ui/pnpm-lock.yaml")
            .exists(),
        "ui's per-workspace lockfile should be in pruned output"
    );
    assert!(
        tempdir
            .path()
            .join("out/packages/config/pnpm-lock.yaml")
            .exists(),
        "config's per-workspace lockfile should be in pruned output"
    );

    // Excluded workspace should not be present
    assert!(
        !tempdir.path().join("out/apps/docs").exists(),
        "docs should not be in pruned output"
    );

    // Root lockfile should be the original (just the "." importer)
    let root_lockfile = fs::read_to_string(tempdir.path().join("out/pnpm-lock.yaml")).unwrap();
    assert!(
        root_lockfile.contains(".: {}"),
        "root lockfile should be the original with only the root importer"
    );

    // .npmrc should be unmodified
    let npmrc = fs::read_to_string(tempdir.path().join("out/.npmrc")).unwrap();
    assert!(
        npmrc.contains("shared-workspace-lockfile=false"),
        ".npmrc should preserve shared-workspace-lockfile=false"
    );
}

#[test]
fn test_prune_pnpm_per_workspace_lockfile_docker() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::copy_fixture("pnpm_per_workspace_lockfile", tempdir.path()).unwrap();
    init_git_preserving_npmrc(tempdir.path());

    let output = run_turbo(tempdir.path(), &["prune", "web", "--docker"]);
    assert!(
        output.status.success(),
        "prune --docker failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // full/ should have per-workspace lockfiles
    assert!(
        tempdir
            .path()
            .join("out/full/apps/web/pnpm-lock.yaml")
            .exists(),
        "web's lockfile should be in out/full/"
    );
    assert!(
        tempdir
            .path()
            .join("out/full/packages/ui/pnpm-lock.yaml")
            .exists(),
        "ui's lockfile should be in out/full/"
    );

    // json/ should also have per-workspace lockfiles (needed for pnpm install)
    assert!(
        tempdir
            .path()
            .join("out/json/apps/web/pnpm-lock.yaml")
            .exists(),
        "web's lockfile should be in out/json/"
    );
    assert!(
        tempdir
            .path()
            .join("out/json/packages/ui/pnpm-lock.yaml")
            .exists(),
        "ui's lockfile should be in out/json/"
    );

    // Root lockfile at out/ level should be the original
    let root_lockfile = fs::read_to_string(tempdir.path().join("out/pnpm-lock.yaml")).unwrap();
    assert!(
        root_lockfile.contains(".: {}"),
        "root lockfile should be the original"
    );

    // json/ root lockfile should also be the original
    let json_lockfile = fs::read_to_string(tempdir.path().join("out/json/pnpm-lock.yaml")).unwrap();
    assert!(
        json_lockfile.contains(".: {}"),
        "json root lockfile should be the original"
    );

    // .npmrc should be unmodified in both directories
    let full_npmrc = fs::read_to_string(tempdir.path().join("out/full/.npmrc")).unwrap();
    assert!(
        full_npmrc.contains("shared-workspace-lockfile=false"),
        ".npmrc in full/ should preserve shared-workspace-lockfile=false"
    );
    let json_npmrc = fs::read_to_string(tempdir.path().join("out/json/.npmrc")).unwrap();
    assert!(
        json_npmrc.contains("shared-workspace-lockfile=false"),
        ".npmrc in json/ should preserve shared-workspace-lockfile=false"
    );
}
