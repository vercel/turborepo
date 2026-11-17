//! `futureFlags.skipUnusedRestores`: cache hits of tasks that were only
//! included as dependencies are not restored unless a task that executes
//! depends on them.
#![cfg(unix)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{combined_output, git, run_turbo, run_turbo_with_env};
use serde_json::{Value, json};

/// `util` <- `lib` <- `app`. Each build reads its dependency's output, so a
/// build that executes fails unless its dependency's outputs are on disk.
fn fixture(root: &Path, enabled: bool, tasks: Value) {
    fs::write(
        root.join("package.json"),
        json!({
            "name": "skip-unused-restores",
            "private": true,
            "packageManager": "npm@10.9.4",
            "workspaces": ["packages/*"]
        })
        .to_string(),
    )
    .unwrap();
    fs::write(root.join(".gitignore"), ".turbo\nnode_modules\ndist\nout\n").unwrap();
    fs::write(
        root.join("turbo.json"),
        json!({
            "agentGuidance": false,
            "futureFlags": { "skipUnusedRestores": enabled },
            "tasks": tasks
        })
        .to_string(),
    )
    .unwrap();
    for (name, dependency) in [("util", None), ("lib", Some("util")), ("app", Some("lib"))] {
        let package = root.join("packages").join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::create_dir_all(package.join("test")).unwrap();
        fs::write(package.join("src/index.txt"), name).unwrap();
        fs::write(package.join("test/index.txt"), name).unwrap();
        let read_dependency = dependency
            .map(|dependency| format!("cat ../{dependency}/dist/out.txt > dist/out.txt && "))
            .unwrap_or_default();
        let read_own_build =
            "cat dist/out.txt > /dev/null && mkdir -p out && echo ok > out/test.txt";
        let mut manifest = json!({
            "name": name,
            "scripts": {
                "build": format!("mkdir -p dist && {read_dependency}echo {name} >> dist/out.txt"),
                "test": read_own_build,
                "check": "cat ../*/dist/out.txt > /dev/null && mkdir -p out && echo ok > out/check.txt",
            }
        });
        if let Some(dependency) = dependency {
            manifest["dependencies"] = json!({ dependency: "*" });
        }
        fs::write(package.join("package.json"), manifest.to_string()).unwrap();
    }
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "user.email", "turbo-test@example.com"]);
    git(root, &["config", "user.name", "Turborepo Test"]);
    git(root, &["add", "."]);
    git(
        root,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "initial"],
    );
}

fn build_tasks() -> Value {
    json!({ "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] } })
}

fn turbo_ok(root: &Path, args: &[&str]) -> String {
    let output = run_turbo(root, args);
    let text = combined_output(&output);
    assert!(output.status.success(), "turbo {args:?} failed:\n{text}");
    text
}

fn clear_outputs(root: &Path) {
    for name in ["util", "lib", "app"] {
        for output in ["dist", "out"] {
            let path = root.join("packages").join(name).join(output);
            if path.exists() {
                fs::remove_dir_all(path).unwrap();
            }
        }
    }
}

fn has_outputs(root: &Path, package: &str) -> bool {
    root.join("packages").join(package).join("dist").exists()
}

fn latest_summary(root: &Path) -> Value {
    let runs = root.join(".turbo/runs");
    let latest = fs::read_dir(&runs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .max_by_key(|path| fs::metadata(path).unwrap().modified().unwrap())
        .unwrap();
    serde_json::from_slice(&fs::read(latest).unwrap()).unwrap()
}

fn summary_task<'a>(summary: &'a Value, task_id: &str) -> &'a Value {
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == task_id)
        .unwrap_or_else(|| panic!("{task_id} missing from summary"))
}

fn dry_run_hash(root: &Path, args: &[&str], task_id: &str) -> Value {
    let mut args = args.to_vec();
    args.push("--dry=json");
    let output = run_turbo(root, &args);
    assert!(output.status.success(), "{}", combined_output(&output));
    let dry: Value = serde_json::from_slice(&output.stdout).unwrap();
    summary_task(&dry, task_id)["hash"].clone()
}

#[test]
fn unused_dependency_hits_are_reported_without_restoring() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root, true, build_tasks());
    turbo_ok(root, &["run", "build"]);
    clear_outputs(root);

    let args = ["run", "build", "--filter=app"];
    let lib_hash = dry_run_hash(root, &args, "lib#build");
    let output = turbo_ok(
        root,
        &[&args[..], &["--summarize", "--output-logs=full"]].concat(),
    );

    // The selected task is restored; its dependencies were only hashed.
    assert!(has_outputs(root, "app"), "{output}");
    assert!(!has_outputs(root, "lib"), "{output}");
    assert!(!has_outputs(root, "util"), "{output}");
    assert!(
        output.contains("lib:build: cache hit, outputs not restored"),
        "{output}"
    );
    assert!(output.contains("3 cached, 3 total"), "{output}");

    let summary = latest_summary(root);
    let lib = summary_task(&summary, "lib#build");
    assert_eq!(lib["cache"]["status"], "HIT");
    assert_eq!(lib["cache"]["outputsRestored"], false);
    assert_eq!(lib["hash"], lib_hash);
    assert_eq!(lib["expandedOutputs"], json!([]));
    let app = summary_task(&summary, "app#build");
    assert_eq!(app["cache"]["status"], "HIT");
    assert!(app["cache"].get("outputsRestored").is_none());
}

#[test]
fn dependency_hits_are_restored_without_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root, false, build_tasks());
    turbo_ok(root, &["run", "build"]);
    clear_outputs(root);

    let output = turbo_ok(root, &["run", "build", "--filter=app", "--summarize"]);
    for package in ["util", "lib", "app"] {
        assert!(has_outputs(root, package), "{package}: {output}");
    }
    let summary = latest_summary(root);
    assert!(
        summary_task(&summary, "lib#build")["cache"]
            .get("outputsRestored")
            .is_none()
    );
}

#[test]
fn dependencies_of_an_executing_task_are_restored() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root, true, build_tasks());
    turbo_ok(root, &["run", "build"]);
    clear_outputs(root);
    // `app` misses, and its build reads `lib`, which reads `util`.
    fs::write(root.join("packages/app/src/index.txt"), "changed").unwrap();

    let output = turbo_ok(root, &["run", "build", "--filter=app"]);
    assert!(output.contains("app:build: cache miss"), "{output}");
    for package in ["util", "lib", "app"] {
        assert!(has_outputs(root, package), "{package}: {output}");
    }
    assert_eq!(
        fs::read_to_string(root.join("packages/app/dist/out.txt")).unwrap(),
        "util\nlib\napp\n"
    );
}

#[test]
fn skipped_dependencies_are_restored_when_an_expected_hit_misses() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root, true, build_tasks());
    turbo_ok(root, &["run", "build"]);
    clear_outputs(root);

    // The entry still exists, so `app` is planned as a hit and its
    // dependencies are skipped, but the archive cannot be restored.
    let app_hash = dry_run_hash(root, &["run", "build", "--filter=app"], "app#build");
    let archive = root.join(format!(
        ".turbo/cache/{}.tar.zst",
        app_hash.as_str().unwrap()
    ));
    assert!(archive.exists());
    fs::write(&archive, "not an archive").unwrap();

    let output = turbo_ok(root, &["run", "build", "--filter=app", "--summarize"]);
    assert!(output.contains("app:build: cache miss"), "{output}");
    for package in ["util", "lib", "app"] {
        assert!(has_outputs(root, package), "{package}: {output}");
    }
    let summary = latest_summary(root);
    let lib = summary_task(&summary, "lib#build");
    assert_eq!(lib["cache"]["status"], "HIT");
    assert!(lib["cache"].get("outputsRestored").is_none(), "{lib}");
}

#[test]
fn runs_without_filters_restore_every_hit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(
        root,
        true,
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "test": { "dependsOn": ["build"], "outputs": ["out/**"] }
        }),
    );
    turbo_ok(root, &["run", "test"]);
    clear_outputs(root);

    // Unfiltered: `build` tasks are only dependencies of `test`, yet are
    // restored as before.
    let output = turbo_ok(root, &["run", "test"]);
    for package in ["util", "lib", "app"] {
        assert!(has_outputs(root, package), "{package}: {output}");
    }

    // The same graph, filtered, leaves the unused builds alone.
    clear_outputs(root);
    let output = turbo_ok(root, &["run", "test", "--filter=app"]);
    assert!(root.join("packages/app/out/test.txt").exists(), "{output}");
    for package in ["util", "lib", "app"] {
        assert!(!has_outputs(root, package), "{package}: {output}");
    }
}

#[test]
fn dependency_output_producers_are_restored_for_hashing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(
        root,
        true,
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] },
            "check": {
                "dependsOn": ["^build"],
                "outputs": ["out/**"],
                "inputs": [
                    "$TURBO_DEFAULT$",
                    { "mode": "dependencyOutputs", "globs": ["dist/**"] }
                ]
            }
        }),
    );
    let args = ["run", "check", "--filter=app", "--summarize"];
    turbo_ok(root, &args);
    let first = latest_summary(root);
    clear_outputs(root);

    // `app#check` hashes `lib`'s outputs, so they must be on disk to find
    // the same hash and hit.
    let output = turbo_ok(root, &args);
    assert!(has_outputs(root, "lib"), "{output}");
    let second = latest_summary(root);
    let check = summary_task(&second, "app#check");
    assert_eq!(check["cache"]["status"], "HIT", "{output}");
    assert_eq!(check["hash"], summary_task(&first, "app#check")["hash"]);
}

#[test]
fn affected_dependents_that_hit_do_not_restore_their_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(
        root,
        true,
        json!({
            "build": { "dependsOn": ["^build"], "outputs": ["dist/**"], "inputs": ["src/**"] },
            "test": { "dependsOn": ["build"], "outputs": ["out/**"], "inputs": ["test/**"] }
        }),
    );
    turbo_ok(root, &["run", "test"]);
    clear_outputs(root);

    // Only `lib`'s tests change. `lib` and its dependent `app` are affected;
    // `lib#test` executes and needs `lib#build` and `util#build`, while
    // `app#test` hits, so `app#build` is never needed.
    fs::write(root.join("packages/lib/test/index.txt"), "changed").unwrap();
    let output = run_turbo_with_env(
        root,
        &["run", "test", "--affected", "--summarize"],
        &[("TURBO_SCM_BASE", "main")],
    );
    let text = combined_output(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("lib:test: cache miss"), "{text}");
    assert!(has_outputs(root, "util"), "{text}");
    assert!(has_outputs(root, "lib"), "{text}");
    assert!(!has_outputs(root, "app"), "{text}");
    assert!(root.join("packages/app/out/test.txt").exists(), "{text}");

    let summary = latest_summary(root);
    let app_build = summary_task(&summary, "app#build");
    assert_eq!(app_build["cache"]["status"], "HIT");
    assert_eq!(app_build["cache"]["outputsRestored"], false);
}
