//! TURBO-6082: co-located package identities must not share a task log or
//! restore/replay another identity's cached log. All tasks use Node overrides;
//! neither Go nor Cargo builds (nor package-manager installs) are needed.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::Duration,
};

use common::setup;
use serde_json::{Value, json};

// The first three identities share one physical directory. The others exercise
// the legacy log layout, including a Go library with a package dependency.
const PACKAGES: &[(&str, &str, &str)] = &[
    ("js-lib", "packages/lib", "javascript"),
    ("example.com/lib", "packages/lib", "go"),
    ("rust-lib", "packages/lib", "rust"),
    ("js-pkg", "packages/js-pkg", "standalone-js"),
    ("example.com/api", "apps/api", "standalone-go"),
];

const COMMAND: &str = r#"
const fs = require('node:fs');
const path = require('node:path');
const id = process.argv[1];
const state = process.env.COLOCATED_LOG_STATE;
const participants = process.env.COLOCATED_LOG_PARTICIPANTS.split(',');
fs.appendFileSync(path.join(state, id + '.count'), 'executed\n');
if (process.env.COLOCATED_LOG_FORBID_EXECUTION === '1') {
    throw new Error('cache restore executed ' + id);
}
fs.writeSync(1, `COLOCATED:${id}:begin\n`);
// Publishing readiness happens only after this task has opened and written its
// log. No task may finish until every other task has also opened its log.
fs.writeFileSync(path.join(state, id + '.ready'), 'ready');
const deadline = Date.now() + 20000;
function poll() {
    const missing = participants.filter(peer =>
        !fs.existsSync(path.join(state, peer + '.ready')));
    if (missing.length === 0) {
        fs.writeSync(1, `COLOCATED:${id}:hash:${process.env.TURBO_HASH}\n`);
        fs.writeSync(2, `COLOCATED:${id}:stderr\n`);
        fs.writeSync(1, `COLOCATED:${id}:end\n`);
        return;
    }
    if (Date.now() >= deadline) {
        throw new Error('barrier timed out waiting for ' + missing.join(','));
    }
    // Bounded condition polling, not a sleep used to assume task overlap.
    setTimeout(poll, 10);
}
poll();
"#;

fn setup_workspace(dir: &Path) {
    // Use the existing mixed Go/npm fixture, but no npm install: every build
    // command is replaced, including the fixture's standalone JS and Go tasks.
    setup::copy_fixture("go_monorepo", dir).unwrap();
    // Keep both Go modules as libraries: executable metadata derives dist/api
    // even with outputs:[], and unfiltered Go builds prefer executables over
    // libraries. This regression needs all five tasks and only implicit logs.
    fs::write(
        dir.join("apps/api/main.go"),
        "package api\n\nimport \"example.com/lib\"\n\nfunc Run() { lib.Greet() }\n",
    )
    .unwrap();
    fs::write(
        dir.join("packages/lib/package.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "js-lib",
            "version": "0.0.0",
            "private": true,
            "scripts": { "build": "node -e \"process.exit(99)\"" }
        }))
        .unwrap(),
    )
    .unwrap();
    let lock_path = dir.join("package-lock.json");
    let mut lock: Value = serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["packages"]["packages/lib"] = json!({ "version": "0.0.0" });
    lock["packages"]["node_modules/js-lib"] = json!({ "resolved": "packages/lib", "link": true });
    fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    fs::write(
        dir.join("Cargo.toml"),
        "[workspace]\nmembers = [\"packages/lib\"]\nresolver = \
         \"2\"\n\n[workspace.metadata]\nname = \"cargo-workspace\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("packages/lib/Cargo.toml"),
        "[package]\nname = \"rust-lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("packages/lib/src")).unwrap();
    fs::write(dir.join("packages/lib/src/lib.rs"), "pub fn library() {}\n").unwrap();
    // A dependency-free lockfile avoids invoking Cargo even during setup.
    fs::write(
        dir.join("Cargo.lock"),
        "version = 4\n\n[[package]]\nname = \"rust-lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let mut config = json!({
        "futureFlags": {
            "experimentalGoWorkspaces": true,
            "experimentalCargoWorkspaces": true,
            "experimentalTaskCommand": true
        },
        "globalPassThroughEnv": [
            "COLOCATED_LOG_STATE",
            "COLOCATED_LOG_PARTICIPANTS",
            "COLOCATED_LOG_FORBID_EXECUTION"
        ],
        "tasks": { "build": { "cache": true, "outputs": [], "dependsOn": [] } }
    });
    for (package, _, identity) in PACKAGES {
        // Explicit per-identity overrides also defeat native library defaults
        // (which may disable caching). There are no user-authored shared outputs.
        config["tasks"][format!("{package}#build")] = json!({
            "cache": true,
            "outputs": [],
            "dependsOn": [],
            "command": ["node", "-e", COMMAND, identity]
        });
    }
    fs::write(
        dir.join("turbo.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
    // Fixture commits must not depend on an interactive signing agent from the
    // developer's global Git config. Leave hooks and the real checkout alone.
    for args in [
        ["init", "--quiet", "--initial-branch=main"],
        ["config", "commit.gpgsign", "false"],
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            common::combined_output(&output)
        );
    }
    setup::setup_git(dir).unwrap();
}

fn run(dir: &Path, state: &Path, dry: bool, filter: Option<&str>, warm: bool) -> (Value, String) {
    let runs = dir.join(".turbo/runs");
    if !dry && runs.exists() {
        fs::remove_dir_all(&runs).unwrap();
    }
    let participants = PACKAGES
        .iter()
        .map(|(_, _, identity)| *identity)
        .collect::<Vec<_>>()
        .join(",");
    let mut command = common::turbo_command(dir);
    command
        .args([
            "run",
            "build",
            "--cache=local:rw",
            "--concurrency=5",
            "--log-order=grouped",
            "--log-prefix=task",
            "--output-logs=full",
        ])
        .env("TURBO_CONFIG_DIR_PATH", state.join("config"))
        .env("COLOCATED_LOG_STATE", state)
        .env("COLOCATED_LOG_PARTICIPANTS", participants)
        .env(
            "COLOCATED_LOG_FORBID_EXECUTION",
            if warm { "1" } else { "0" },
        )
        .env("GOENV", "off")
        .env("GOTOOLCHAIN", "local")
        .env_remove("GOWORK")
        .env_remove("GOFLAGS")
        .timeout(Duration::from_secs(60));
    command.arg(if dry { "--dry=json" } else { "--summarize" });
    if let Some(package) = filter {
        command.arg(format!("--filter={package}"));
    }
    let output = command.output().expect("failed to execute turbo");
    let combined = common::combined_output(&output);
    assert!(
        output.status.success(),
        "dry={dry}, filter={filter:?}, warm={warm}: {combined}"
    );
    let summary = if dry {
        serde_json::from_slice(&output.stdout).expect("dry run emits JSON")
    } else {
        let summaries = fs::read_dir(runs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 1, "one summary per real run");
        serde_json::from_slice(&fs::read(&summaries[0]).unwrap()).unwrap()
    };
    (summary, combined)
}

fn task<'a>(summary: &'a Value, package: &str) -> &'a Value {
    let id = format!("{package}#build");
    summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["taskId"] == id)
        .unwrap_or_else(|| panic!("missing {id}: {summary}"))
}

fn log_file(task: &Value) -> String {
    task["logFile"].as_str().unwrap().replace('\\', "/")
}

fn assert_same_contract(expected: &Value, actual: &Value) {
    for field in ["taskId", "hash", "directory", "logFile"] {
        assert_eq!(expected[field], actual[field], "{field} changed");
    }
}

fn assert_cached_log(task: &Value, status: &str) {
    assert_eq!(task["cache"]["status"], status, "{task}");
    if status == "HIT" {
        assert_eq!(task["cache"]["local"], true);
    }
    assert_eq!(task["resolvedTaskDefinition"]["cache"], true);
    assert_eq!(task["resolvedTaskDefinition"]["outputs"], json!([]));
    // With outputs:[], the path saved/restored under the task hash must be
    // exactly the summary's logFile, never a sibling identity's log.
    let outputs = task["expandedOutputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().replace('\\', "/"))
        .collect::<Vec<_>>();
    assert_eq!(outputs, vec![log_file(task)]);
}

fn assert_executed_once(state: &Path) {
    for (_, _, identity) in PACKAGES {
        assert_eq!(
            fs::read_to_string(state.join(format!("{identity}.count"))).unwrap(),
            "executed\n",
            "{identity} must execute only on the cold run"
        );
    }
}

fn remove_logs(dir: &Path, summary: &Value) {
    for task in summary["tasks"].as_array().unwrap() {
        let path = dir.join(log_file(task));
        fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }
}

#[test]
fn colocated_task_logs_are_isolated_and_restored_with_stable_hashes() {
    if which::which("go").is_err() {
        eprintln!("skipping: go is not on PATH");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    // Barrier files, execution counters, and config live outside the repository
    // and therefore cannot race with hashing or invalidate a warm-cache run.
    let state = tempfile::tempdir().unwrap();
    let dir = repo.path();
    setup_workspace(dir);

    let (before, _) = run(dir, state.path(), true, None, false);
    assert_eq!(
        before["tasks"].as_array().unwrap().len(),
        PACKAGES.len(),
        "discovered tasks: {:?}",
        before["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| &task["taskId"])
            .collect::<Vec<_>>()
    );
    for (_, _, identity) in PACKAGES {
        assert!(!state.path().join(format!("{identity}.count")).exists());
    }

    let (cold, cold_output) = run(dir, state.path(), false, None, false);
    assert_eq!(cold["tasks"].as_array().unwrap().len(), PACKAGES.len());
    assert_executed_once(state.path());
    let mut paths = BTreeSet::new();
    let mut hashes = BTreeSet::new();
    let mut logs = BTreeMap::new();
    for (package, directory, identity) in PACKAGES {
        let current = task(&cold, package);
        assert_same_contract(task(&before, package), current);
        assert_cached_log(current, "MISS");
        assert_eq!(
            current["directory"].as_str().unwrap().replace('\\', "/"),
            *directory
        );
        let path = log_file(current);
        assert!(paths.insert(path.clone()), "shared log path: {path}");
        let legacy = format!("{directory}/.turbo/turbo-build.log");
        if *directory == "packages/lib" {
            assert_ne!(path, legacy, "co-located identities need namespaced logs");
            assert!(path.starts_with("packages/lib/.turbo/"));
            assert!(
                !dir.join(legacy).exists(),
                "no legacy shared log should be written"
            );
        } else {
            assert_eq!(path, legacy, "non-shared directories keep legacy paths");
        }
        let hash = current["hash"].as_str().unwrap();
        assert!(!hash.is_empty());
        assert!(
            hashes.insert(hash.to_owned()),
            "overrides need distinct task hashes"
        );
        assert!(dir.join(format!(".turbo/cache/{hash}.tar.zst")).is_file());
        let bytes = fs::read(dir.join(&path)).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        for marker in [
            format!("COLOCATED:{identity}:begin"),
            format!("COLOCATED:{identity}:hash:{hash}"),
            format!("COLOCATED:{identity}:stderr"),
            format!("COLOCATED:{identity}:end"),
        ] {
            assert!(text.contains(&marker), "{path} is missing {marker}: {text}");
            assert!(
                cold_output.contains(&marker),
                "cold output is missing {marker}"
            );
        }
        for (_, _, other) in PACKAGES {
            if other != identity {
                assert!(
                    !text.contains(&format!("COLOCATED:{other}:")),
                    "{path} contains {other}'s output: {text}"
                );
            }
        }
        logs.insert(package.to_string(), bytes);
        fs::remove_file(state.path().join(format!("{identity}.ready"))).unwrap();
    }

    remove_logs(dir, &cold);
    let (warm_dry, _) = run(dir, state.path(), true, None, true);
    let (warm, replay) = run(dir, state.path(), false, None, true);
    assert_eq!(warm["tasks"].as_array().unwrap().len(), PACKAGES.len());
    assert_executed_once(state.path());
    for (package, _, identity) in PACKAGES {
        let expected = task(&cold, package);
        assert_same_contract(expected, task(&warm_dry, package));
        assert_same_contract(expected, task(&warm, package));
        assert_cached_log(task(&warm, package), "HIT");
        assert_eq!(
            fs::read(dir.join(log_file(expected))).unwrap(),
            logs[*package]
        );
        assert!(replay.contains(&format!("{package}:build: cache hit, replaying logs")));
        for phase in ["begin", "stderr", "end"] {
            assert!(replay.contains(&format!("COLOCATED:{identity}:{phase}")));
        }
    }

    // Discovery of shared directories must not depend on the selected tasks.
    // Remove every log before restoring one identity at a time, so even a cache
    // archive accidentally containing all sibling logs cannot pass unnoticed.
    remove_logs(dir, &warm);
    for (package, _, identity) in &PACKAGES[..3] {
        let (filtered_dry, _) = run(dir, state.path(), true, Some(package), true);
        let (filtered, replay) = run(dir, state.path(), false, Some(package), true);
        assert_eq!(filtered_dry["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["tasks"].as_array().unwrap().len(), 1);
        let expected = task(&cold, package);
        assert_same_contract(expected, task(&filtered_dry, package));
        assert_same_contract(expected, task(&filtered, package));
        assert_cached_log(task(&filtered, package), "HIT");
        assert_eq!(
            fs::read(dir.join(log_file(expected))).unwrap(),
            logs[*package]
        );
        assert!(replay.contains(&format!("{package}:build: cache hit, replaying logs")));
        for phase in ["begin", "stderr", "end"] {
            assert!(replay.contains(&format!("COLOCATED:{identity}:{phase}")));
        }
        for (other, _, other_identity) in PACKAGES {
            if other != package {
                assert!(!dir.join(log_file(task(&cold, other))).exists());
                assert!(!replay.contains(&format!("COLOCATED:{other_identity}:")));
            }
        }
        assert_executed_once(state.path());
        fs::remove_file(dir.join(log_file(expected))).unwrap();
    }
}
