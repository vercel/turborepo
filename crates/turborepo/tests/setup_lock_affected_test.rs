#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{collections::BTreeSet, fs, path::Path, process::Output};

use common::{git, run_turbo, run_turbo_with_env};

fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "."]);
    git(
        dir,
        &["-c", "commit.gpgsign=false", "commit", "-qm", message],
    );
}

const TASK_INPUT_MODES: [(bool, bool); 4] =
    [(false, false), (false, true), (true, false), (true, true)];

fn fixture(dir: &Path, enabled: bool, task_inputs: (bool, bool), committed_lock: bool) {
    fs::write(
        dir.join("package.json"),
        r#"{"name":"root","private":true,"packageManager":"npm@10.5.0","workspaces":["packages/*"]}"#,
    )
    .unwrap();
    fs::write(
        dir.join("package-lock.json"),
        r#"{"name":"root","lockfileVersion":3,"packages":{"":{"name":"root","workspaces":["packages/*"]},"packages/a":{"name":"a","version":"1.0.0"},"packages/b":{"name":"b","version":"1.0.0"},"node_modules/a":{"resolved":"packages/a","link":true},"node_modules/b":{"resolved":"packages/b","link":true}}}"#,
    )
    .unwrap();
    fs::write(
        dir.join("turbo.json"),
        serde_json::json!({
            "futureFlags": {
                "experimentalSetup": enabled,
                "affectedUsingTaskInputs": task_inputs.0,
                "filterUsingTasks": task_inputs.1
            },
            // Neither configured exclusions nor narrow task inputs may override
            // the managed lock's default-global status.
            "globalDependencies": ["!turbo.lock"],
            "tasks": {"build": {"inputs": ["src/**"], "outputs": []}}
        })
        .to_string(),
    )
    .unwrap();
    for name in ["a", "b"] {
        let package = dir.join("packages").join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(
            package.join("package.json"),
            serde_json::json!({"name": name, "version": "1.0.0", "scripts": {"build": "echo build"}})
                .to_string(),
        )
        .unwrap();
        fs::write(package.join("src/index.js"), "export {};\n").unwrap();
    }
    if committed_lock {
        fs::write(dir.join("turbo.lock"), "initial resolution\n").unwrap();
    }
    git(dir, &["init", "-q", "--initial-branch=main"]);
    git(dir, &["config", "user.email", "turbo-test@example.com"]);
    git(dir, &["config", "user.name", "Turborepo Test"]);
    commit(dir, "Initial");
}

fn json(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_selection(dir: &Path, managed: bool, base: &str, head: &str) {
    let expected: BTreeSet<_> = if managed {
        ["a#build".to_string(), "b#build".to_string()].into()
    } else {
        BTreeSet::new()
    };
    let filter = if head.is_empty() {
        format!("--filter=[{base}]")
    } else {
        format!("--filter=[{base}...{head}]")
    };
    for args in [
        vec!["run", "build", "--affected", "--dry=json"],
        vec!["run", "build", &filter, "--dry=json"],
    ] {
        let result = json(run_turbo_with_env(
            dir,
            &args,
            &[("TURBO_SCM_BASE", base), ("TURBO_SCM_HEAD", head)],
        ));
        let selected: BTreeSet<_> = result["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["taskId"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(selected, expected, "{args:?}");
    }

    let refs = if head.is_empty() {
        format!("base: \"{base}\"")
    } else {
        format!("base: \"{base}\", head: \"{head}\"")
    };
    let query = format!(
        "{{ affectedPackages({refs}) {{ items {{ name reason {{ __typename }} }} }} \
         affectedTasks({refs}) {{ items {{ fullName reason {{ __typename }} }} }} }}"
    );
    let result = json(run_turbo(dir, &["query", &query]));
    let packages = result["data"]["affectedPackages"]["items"]
        .as_array()
        .unwrap();
    let non_root: BTreeSet<_> = packages
        .iter()
        .filter_map(|package| package["name"].as_str().filter(|name| *name != "//"))
        .collect();
    assert_eq!(
        non_root,
        if managed {
            ["a", "b"].into()
        } else {
            BTreeSet::new()
        }
    );
    if managed {
        assert!(
            packages
                .iter()
                .all(|package| { package["reason"]["__typename"] == "DefaultGlobalFileChanged" })
        );
    }
    let tasks: BTreeSet<_> = result["data"]["affectedTasks"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["fullName"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(tasks, expected);
}

#[test]
fn committed_setup_lock_changes_select_all_packages_in_both_modes() {
    for task_inputs in TASK_INPUT_MODES {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fixture(dir, true, task_inputs, true);
        fs::write(dir.join("turbo.lock"), "updated resolution\n").unwrap();
        commit(dir, "Update setup lock");
        assert_selection(dir, true, "HEAD~1", "HEAD");
        // A managed repository with no changes still selects nothing.
        assert_selection(dir, false, "HEAD", "HEAD");
    }
}

#[test]
fn setup_lock_changes_preserve_unconfigured_repositories() {
    for (enabled, committed_lock) in [(false, true), (false, false), (true, false)] {
        for task_inputs in TASK_INPUT_MODES {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path();
            fixture(dir, enabled, task_inputs, committed_lock);
            fs::write(dir.join("turbo.lock"), "local resolution\n").unwrap();
            assert_selection(dir, false, "HEAD", "");
            if !committed_lock {
                git(dir, &["add", "turbo.lock"]);
                assert_selection(dir, false, "HEAD", "");
            }
        }
    }
}

#[test]
fn committing_setup_lock_opts_change_detection_into_managed_mode() {
    for task_inputs in TASK_INPUT_MODES {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fixture(dir, true, task_inputs, false);
        fs::write(dir.join("turbo.lock"), "initial resolution\n").unwrap();
        commit(dir, "Commit setup lock");
        assert_selection(dir, true, "HEAD~1", "HEAD");
    }
}

#[test]
fn working_tree_setup_lock_deletion_is_global_in_both_modes() {
    for task_inputs in TASK_INPUT_MODES {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fixture(dir, true, task_inputs, true);
        fs::remove_file(dir.join("turbo.lock")).unwrap();
        assert_selection(dir, true, "HEAD", "");
    }
}

#[test]
fn package_local_setup_lock_is_not_global() {
    for task_inputs in TASK_INPUT_MODES {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fixture(dir, true, task_inputs, true);
        fs::write(dir.join("packages/a/turbo.lock"), "local resolution\n").unwrap();
        // Narrow task inputs exclude this package-local file. Package-level
        // affectedness may select a, but must not select the unrelated b.
        let result = json(run_turbo_with_env(
            dir,
            &["run", "build", "--affected", "--dry=json"],
            &[("TURBO_SCM_BASE", "HEAD"), ("TURBO_SCM_HEAD", "")],
        ));
        assert!(
            result["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|task| { task["taskId"].as_str().unwrap() != "b#build" })
        );
    }
}
