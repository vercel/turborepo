#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, process::Stdio};

use common::{git, replace_turbo_json, run_turbo_with_env, setup};
use serde_json::Value;

const TURBO_JSON_GLOBAL_DEPS: &str = r#"{
  "globalDependencies": ["config.txt"],
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}
"#;

const TURBO_JSON_GLOBAL_INPUTS: &str = r#"{
  "futureFlags": { "globalConfiguration": true },
  "global": {
    "inputs": ["config.txt"]
  },
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}
"#;

fn dry_json(test_dir: &Path, args: &[&str]) -> Value {
    dry_json_with_env(test_dir, args, &[])
}

fn dry_json_with_env(test_dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Value {
    let output = run_turbo_with_env(test_dir, args, env);
    assert!(
        output.status.success(),
        "dry run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn task_hash_contract(json: &Value) -> String {
    let mut lines = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            format!(
                "{} hash={} external={} framework={} envMode={}",
                task["taskId"].as_str().unwrap(),
                task["hash"].as_str().unwrap(),
                task["hashOfExternalDependencies"].as_str().unwrap_or(""),
                task["framework"].as_str().unwrap_or(""),
                task["envMode"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines.join("\n")
}

fn global_cache_inputs_contract(json: &Value) -> String {
    let global = &json["globalCacheInputs"];
    let mut lines = vec![
        format!("rootKey={}", global["rootKey"].as_str().unwrap_or("")),
        format!(
            "hashOfExternalDependencies={}",
            global["hashOfExternalDependencies"].as_str().unwrap_or("")
        ),
        format!(
            "hashOfInternalDependencies={}",
            global["hashOfInternalDependencies"].as_str().unwrap_or("")
        ),
    ];

    if let Some(files) = global["files"].as_object() {
        let mut file_lines = files
            .iter()
            .map(|(path, hash)| format!("file:{path}={}", hash.as_str().unwrap()))
            .collect::<Vec<_>>();
        file_lines.sort();
        lines.extend(file_lines);
    }

    let env = &global["environmentVariables"];
    lines.push(format!(
        "env.specified={}",
        serde_json::to_string(&env["specified"]).unwrap()
    ));
    lines.push(format!(
        "env.configured={}",
        serde_json::to_string(&env["configured"]).unwrap()
    ));
    lines.push(format!(
        "env.inferred={}",
        serde_json::to_string(&env["inferred"]).unwrap()
    ));
    lines.push(format!(
        "env.passthrough={}",
        serde_json::to_string(&env["passthrough"]).unwrap()
    ));

    lines.join("\n")
}

fn labeled_contracts(contracts: &[(&str, String)]) -> String {
    contracts
        .iter()
        .map(|(label, contract)| format!("[{label}]\n{contract}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn setup_global_inputs_fixture(dir: &Path, turbo_json: &str) {
    setup::setup_integration_test(dir, "global_inputs", "npm@10.5.0", false).unwrap();
    fs::write(dir.join("turbo.json"), turbo_json).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "set turbo config", "--quiet"]);
}

fn setup_lockfile_fixture(dir: &Path, pm_name: &str) {
    let repo_root = common::manifest_dir().join("../../");
    let base_fixture =
        repo_root.join("turborepo-tests/integration/fixtures/lockfile_aware_caching");
    let pm_overlay = repo_root.join(format!(
        "turborepo-tests/integration/tests/lockfile-aware-caching/{pm_name}"
    ));

    setup::copy_dir_all(&base_fixture, dir).unwrap();
    setup::copy_dir_all(&pm_overlay, dir).unwrap();

    git(dir, &["init", "--quiet"]);
    git(dir, &["config", "user.email", "turbo-test@example.com"]);
    git(dir, &["config", "user.name", "Turbo Test"]);

    let gitignore = dir.join(".gitignore");
    let mut contents = fs::read_to_string(&gitignore).unwrap_or_default();
    contents.push_str("\n.turbo\nnode_modules\n");
    fs::write(&gitignore, contents).unwrap();

    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "Initial", "--quiet"]);
}

fn apply_patch(dir: &Path, target: &str, patch_file: &str) {
    let status = std::process::Command::new("patch")
        .args([target, patch_file])
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "patch {target} {patch_file} failed");
}

#[test]
fn baseline_monorepo_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();

    let json = dry_json(tempdir.path(), &["run", "build", "--dry=json"]);

    insta::assert_snapshot!("baseline_monorepo_task_hashes", task_hash_contract(&json));
    insta::assert_snapshot!(
        "baseline_monorepo_global_cache_inputs",
        global_cache_inputs_contract(&json)
    );
}

#[test]
fn single_package_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "single_package", "npm@10.5.0", false).unwrap();

    let build_json = dry_json(tempdir.path(), &["run", "build", "--dry=json"]);
    let test_json = dry_json(tempdir.path(), &["run", "test", "--dry=json"]);

    let contract = labeled_contracts(&[
        ("build", task_hash_contract(&build_json)),
        ("test", task_hash_contract(&test_json)),
    ]);
    insta::assert_snapshot!("single_package_task_hashes", contract);
}

#[test]
fn workspace_config_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "composable_config", "npm@10.5.0", false)
        .unwrap();

    let override_json = dry_json(
        tempdir.path(),
        &[
            "run",
            "override-values-task",
            "--filter=override-values",
            "--dry=json",
        ],
    );
    let inherited_json = dry_json(
        tempdir.path(),
        &[
            "run",
            "missing-workspace-config-task",
            "--filter=missing-workspace-config",
            "--dry=json",
        ],
    );
    let override_with_deps_json = dry_json(
        tempdir.path(),
        &[
            "run",
            "override-values-task-with-deps",
            "--filter=override-values",
            "--dry=json",
        ],
    );
    let initial_config_change_json = dry_json(
        tempdir.path(),
        &[
            "run",
            "config-change-task",
            "--filter=config-change",
            "--dry=json",
        ],
    );

    fs::copy(
        tempdir.path().join("apps/config-change/turbo-changed.json"),
        tempdir.path().join("apps/config-change/turbo.json"),
    )
    .unwrap();

    let changed_config_json = dry_json(
        tempdir.path(),
        &[
            "run",
            "config-change-task",
            "--filter=config-change",
            "--dry=json",
        ],
    );

    let contract = labeled_contracts(&[
        ("override", task_hash_contract(&override_json)),
        ("inherited", task_hash_contract(&inherited_json)),
        (
            "override-with-deps",
            task_hash_contract(&override_with_deps_json),
        ),
        (
            "config-change-initial",
            task_hash_contract(&initial_config_change_json),
        ),
        (
            "config-change-updated",
            task_hash_contract(&changed_config_json),
        ),
    ]);
    insta::assert_snapshot!("workspace_config_task_hashes", contract);
}

#[test]
fn env_mode_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "strict_env_vars", "npm@10.5.0", false).unwrap();

    let default_json = dry_json(tempdir.path(), &["run", "build", "--dry=json"]);
    let strict_json = dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--env-mode=strict"],
    );
    let loose_json = dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--env-mode=loose"],
    );

    replace_turbo_json(tempdir.path(), "strict_env_vars/global_pt.json");

    let strict_passthrough_json = dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--env-mode=strict"],
    );
    let loose_passthrough_json = dry_json(
        tempdir.path(),
        &["run", "build", "--dry=json", "--env-mode=loose"],
    );

    let contract = labeled_contracts(&[
        ("default", task_hash_contract(&default_json)),
        ("strict", task_hash_contract(&strict_json)),
        ("loose", task_hash_contract(&loose_json)),
        (
            "strict-with-global-passthrough",
            task_hash_contract(&strict_passthrough_json),
        ),
        (
            "loose-with-global-passthrough",
            task_hash_contract(&loose_passthrough_json),
        ),
    ]);
    insta::assert_snapshot!("env_mode_task_hashes", contract);
}

#[test]
fn global_inputs_hashes() {
    let global_deps_dir = tempfile::tempdir().unwrap();
    setup_global_inputs_fixture(global_deps_dir.path(), TURBO_JSON_GLOBAL_DEPS);
    let global_deps_initial = dry_json(global_deps_dir.path(), &["run", "build", "--dry=json"]);
    fs::write(global_deps_dir.path().join("config.txt"), "changed value").unwrap();
    let global_deps_changed = dry_json(global_deps_dir.path(), &["run", "build", "--dry=json"]);

    let global_inputs_dir = tempfile::tempdir().unwrap();
    setup_global_inputs_fixture(global_inputs_dir.path(), TURBO_JSON_GLOBAL_INPUTS);
    let global_inputs_initial = dry_json(global_inputs_dir.path(), &["run", "build", "--dry=json"]);
    fs::write(global_inputs_dir.path().join("config.txt"), "changed value").unwrap();
    let global_inputs_changed = dry_json(global_inputs_dir.path(), &["run", "build", "--dry=json"]);

    let global_inputs_package_dir = tempfile::tempdir().unwrap();
    setup_global_inputs_fixture(global_inputs_package_dir.path(), TURBO_JSON_GLOBAL_INPUTS);
    fs::write(
        global_inputs_package_dir
            .path()
            .join("packages/app-a/index.js"),
        "console.log('modified');",
    )
    .unwrap();
    let global_inputs_package_changed = dry_json(
        global_inputs_package_dir.path(),
        &["run", "build", "--filter=app-a", "--dry=json"],
    );

    let contract = labeled_contracts(&[
        (
            "global-dependencies-initial",
            task_hash_contract(&global_deps_initial),
        ),
        (
            "global-dependencies-changed",
            task_hash_contract(&global_deps_changed),
        ),
        (
            "global-inputs-initial",
            task_hash_contract(&global_inputs_initial),
        ),
        (
            "global-inputs-changed",
            task_hash_contract(&global_inputs_changed),
        ),
        (
            "global-inputs-package-changed",
            task_hash_contract(&global_inputs_package_changed),
        ),
    ]);
    insta::assert_snapshot!("global_inputs_task_hashes", contract);
}

#[test]
fn gitignored_explicit_input_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "basic_monorepo", "npm@10.5.0", false).unwrap();
    replace_turbo_json(tempdir.path(), "gitignored-inputs.json");

    fs::write(
        tempdir.path().join("packages/util/internal.txt"),
        "hello world\n",
    )
    .unwrap();
    let mut gitignore = fs::read_to_string(tempdir.path().join(".gitignore")).unwrap_or_default();
    gitignore.push_str("\npackages/util/internal.txt\n");
    fs::write(tempdir.path().join(".gitignore"), gitignore).unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "add internal.txt", "--quiet"],
    );

    let initial_json = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=util", "--dry=json"],
    );
    fs::write(
        tempdir.path().join("packages/util/internal.txt"),
        "hello world\nchanged!\n",
    )
    .unwrap();
    let changed_json = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=util", "--dry=json"],
    );

    let contract = labeled_contracts(&[
        ("initial", task_hash_contract(&initial_json)),
        ("changed", task_hash_contract(&changed_json)),
    ]);
    insta::assert_snapshot!("gitignored_explicit_input_task_hashes", contract);
}

#[test]
fn framework_inference_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "framework_inference", "npm@10.5.0", false)
        .unwrap();

    let enabled_json = dry_json_with_env(
        tempdir.path(),
        &["run", "build", "--framework-inference=true", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );
    let disabled_json = dry_json_with_env(
        tempdir.path(),
        &["run", "build", "--framework-inference=false", "--dry=json"],
        &[("NEXT_PUBLIC_CHANGED", "true")],
    );

    let contract = labeled_contracts(&[
        ("enabled", task_hash_contract(&enabled_json)),
        ("disabled", task_hash_contract(&disabled_json)),
    ]);
    insta::assert_snapshot!("framework_inference_task_hashes", contract);
}

#[test]
fn lockfile_aware_hashes() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_lockfile_fixture(tempdir.path(), "npm");

    let a_initial = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=a", "--dry=json"],
    );
    let b_initial = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=b", "--dry=json"],
    );

    apply_patch(tempdir.path(), "package-lock.json", "package-lock.patch");

    let a_dep_bump = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=a", "--dry=json"],
    );
    let b_dep_bump = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=b", "--dry=json"],
    );

    apply_patch(tempdir.path(), "package-lock.json", "turbo-bump.patch");

    let a_root_bump = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=a", "--dry=json"],
    );
    let b_root_bump = dry_json(
        tempdir.path(),
        &["run", "build", "--filter=b", "--dry=json"],
    );

    let contract = labeled_contracts(&[
        ("a-initial", task_hash_contract(&a_initial)),
        ("b-initial", task_hash_contract(&b_initial)),
        ("a-after-b-dep-bump", task_hash_contract(&a_dep_bump)),
        ("b-after-b-dep-bump", task_hash_contract(&b_dep_bump)),
        ("a-after-root-bump", task_hash_contract(&a_root_bump)),
        ("b-after-root-bump", task_hash_contract(&b_root_bump)),
    ]);
    insta::assert_snapshot!("lockfile_aware_npm_task_hashes", contract);
}
