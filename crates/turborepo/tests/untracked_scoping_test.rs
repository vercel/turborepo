#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{git, run_turbo, setup};

/// A turbo.json without `globalDependencies` (the `basic_monorepo` fixture
/// declares `foo.txt`, which keeps the whole-repo scan) and without task
/// inputs, so `build` hashes every file under each package.
const PLAIN_TURBO_JSON: &str = r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}"#;

fn setup_repo(dir: &Path, turbo_json: &str) {
    setup::setup_integration_test(dir, "basic_monorepo", "npm@10.5.0", false).unwrap();
    fs::write(dir.join("turbo.json"), turbo_json).unwrap();
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "commit",
            "-m",
            "configure turbo.json",
            "--quiet",
            "--allow-empty",
        ],
    );
}

/// Run `turbo run ... --dry=json` and return `(taskId, hash)` pairs.
fn task_hashes(dir: &Path, args: &[&str]) -> Vec<(String, String)> {
    let mut full_args = vec!["run"];
    full_args.extend_from_slice(args);
    full_args.push("--dry=json");
    let output = run_turbo(dir, &full_args);
    assert!(
        output.status.success(),
        "turbo {:?} failed: {}",
        full_args,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("invalid dry=json output: {e}\n{stdout}"));
    json["tasks"]
        .as_array()
        .expect("tasks array in dry=json output")
        .iter()
        .map(|task| {
            (
                task["taskId"].as_str().expect("taskId").to_string(),
                task["hash"]
                    .as_str()
                    .unwrap_or_else(|| panic!("hash missing for {}", task["taskId"]))
                    .to_string(),
            )
        })
        .collect()
}

fn hash_of(hashes: &[(String, String)], task_id: &str) -> String {
    hashes
        .iter()
        .find(|(id, _)| id == task_id)
        .unwrap_or_else(|| panic!("missing {task_id} in {hashes:?}"))
        .1
        .clone()
}

/// Run turbo with `--verbosity=2` and return the debug log it wrote, so
/// tests can assert on the chosen untracked-file scan scope.
fn run_with_debug_log(dir: &Path, args: &[&str]) -> String {
    let debug_dir = dir.join(".turbo").join("debug-logs");
    fs::remove_dir_all(&debug_dir).ok();
    let mut full_args = vec!["run"];
    full_args.extend_from_slice(args);
    full_args.push("--verbosity=2");
    let output = run_turbo(dir, &full_args);
    assert!(
        output.status.success(),
        "turbo {:?} failed: {}",
        full_args,
        String::from_utf8_lossy(&output.stderr)
    );
    let mut logs: Vec<_> = fs::read_dir(&debug_dir)
        .expect("debug-logs directory should exist after a verbose run")
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!logs.is_empty(), "expected a debug log to be written");
    logs.sort();
    fs::read_to_string(logs.pop().unwrap()).unwrap()
}

#[test]
fn scoped_run_hashes_untracked_files_identically_to_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);

    // Untracked files: one inside the selected package's hashed inputs,
    // plus others in an unselected package and at the repo root.
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked util source v1",
    )
    .unwrap();
    fs::write(
        tempdir
            .path()
            .join("apps")
            .join("my-app")
            .join("untracked-app.txt"),
        "untracked app file",
    )
    .unwrap();
    fs::write(tempdir.path().join("untracked-root.txt"), "root untracked").unwrap();

    // The filtered run scopes the untracked scan to the selected package;
    // the same task without a filter keeps today's whole-repo scan. Both
    // must agree.
    let scoped = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    let full = task_hashes(tempdir.path(), &["util#build"]);
    assert_eq!(
        hash_of(&scoped, "util#build"),
        hash_of(&full, "util#build"),
        "scoped scan must discover the same untracked files as the whole-repo scan"
    );

    // The untracked file is genuinely hashed: changing its content changes
    // the scoped hash.
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked util source v2",
    )
    .unwrap();
    let changed = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert_ne!(
        hash_of(&scoped, "util#build"),
        hash_of(&changed, "util#build"),
        "untracked file inside the package inputs must affect the hash"
    );

    // Untracked-discovered and tracked-clean agree: restoring the content
    // and committing the file reproduces the first hash, proving the
    // untracked file was hashed with the same value the tracked index uses.
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked util source v1",
    )
    .unwrap();
    git(tempdir.path(), &["add", "packages/util/untracked.ts"]);
    git(
        tempdir.path(),
        &["commit", "-m", "track untracked.ts", "--quiet"],
    );
    let tracked = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert_eq!(
        hash_of(&scoped, "util#build"),
        hash_of(&tracked, "util#build"),
        "untracked-discovered and tracked-clean hashes must match"
    );

    // Scope selection: the filtered run walks only package directories;
    // the unfiltered run keeps the whole-repo scan.
    let scoped_log = run_with_debug_log(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert!(
        scoped_log.contains("untracked-file scan scope: package directory prefixes"),
        "expected a scoped scan, log:\n{scoped_log}"
    );
    let full_log = run_with_debug_log(tempdir.path(), &["util#build"]);
    assert!(
        full_log.contains("untracked-file scan scope: whole repo"),
        "expected a whole-repo scan, log:\n{full_log}"
    );
}

#[test]
fn scoped_run_covers_untracked_files_in_nested_packages() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);

    // A workspace package nested two levels below `packages/`, an untracked
    // source file inside it, and an untracked file in the parent directory
    // that no task hashes.
    let inner = tempdir.path().join("packages").join("group").join("inner");
    fs::create_dir_all(inner.join("src")).unwrap();
    fs::write(
        inner.join("package.json"),
        r#"{ "name": "inner", "version": "1.0.0", "scripts": { "build": "echo inner" } }"#,
    )
    .unwrap();
    fs::write(inner.join("src").join("new.ts"), "nested untracked v1").unwrap();
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("group")
            .join("sibling.txt"),
        "outside any package",
    )
    .unwrap();

    let scoped = task_hashes(tempdir.path(), &["build", "--filter=inner", "--only"]);
    let full = task_hashes(tempdir.path(), &["inner#build"]);
    assert_eq!(
        hash_of(&scoped, "inner#build"),
        hash_of(&full, "inner#build"),
        "scoped scan must cover nested package directories"
    );

    fs::write(inner.join("src").join("new.ts"), "nested untracked v2").unwrap();
    let changed = task_hashes(tempdir.path(), &["build", "--filter=inner", "--only"]);
    assert_ne!(
        hash_of(&scoped, "inner#build"),
        hash_of(&changed, "inner#build"),
        "untracked file inside the nested package must affect the hash"
    );

    let log = run_with_debug_log(tempdir.path(), &["build", "--filter=inner", "--only"]);
    assert!(
        log.contains("untracked-file scan scope: package directory prefixes"),
        "expected a scoped scan, log:\n{log}"
    );
}

#[test]
fn scoped_scan_respects_ancestor_and_nested_gitignore_rules() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);

    // A tracked ancestor .gitignore (two levels above the package) and a
    // tracked .gitignore inside the package itself.
    fs::write(
        tempdir.path().join("packages").join(".gitignore"),
        "*.secret\n",
    )
    .unwrap();
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join(".gitignore"),
        "local-only.txt\n",
    )
    .unwrap();
    git(tempdir.path(), &["add", "."]);
    git(
        tempdir.path(),
        &["commit", "-m", "add gitignores", "--quiet"],
    );

    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("normal.txt"),
        "hashed",
    )
    .unwrap();
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("leaked.secret"),
        "ignored by the ancestor .gitignore",
    )
    .unwrap();
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("local-only.txt"),
        "ignored by the package .gitignore",
    )
    .unwrap();

    let with_ignored = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    let full = task_hashes(tempdir.path(), &["util#build"]);
    assert_eq!(
        hash_of(&with_ignored, "util#build"),
        hash_of(&full, "util#build"),
        "scoped and whole-repo scans must apply the same gitignore rules"
    );

    // Ignored files contribute nothing: removing them does not change the
    // hash.
    fs::remove_file(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("leaked.secret"),
    )
    .unwrap();
    fs::remove_file(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("local-only.txt"),
    )
    .unwrap();
    let without_ignored = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert_eq!(
        hash_of(&with_ignored, "util#build"),
        hash_of(&without_ignored, "util#build"),
        "gitignored untracked files must not be hashed in scoped mode"
    );

    // The non-ignored untracked file does contribute.
    fs::remove_file(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("normal.txt"),
    )
    .unwrap();
    let without_normal = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert_ne!(
        hash_of(&with_ignored, "util#build"),
        hash_of(&without_normal, "util#build"),
        "non-ignored untracked files must be hashed in scoped mode"
    );

    let log = run_with_debug_log(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert!(
        log.contains("untracked-file scan scope: package directory prefixes"),
        "expected a scoped scan, log:\n{log}"
    );
}

#[test]
fn global_dependencies_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "globalDependencies": ["foo.txt"],
  "tasks": {
    "build": {
      "outputs": []
    }
  }
}"#,
    );

    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked",
    )
    .unwrap();

    let log = run_with_debug_log(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert!(
        log.contains("untracked-file scan scope: whole repo (not provably scoped)"),
        "globalDependencies must keep the whole-repo scan, log:\n{log}"
    );

    let scoped = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    let full = task_hashes(tempdir.path(), &["util#build"]);
    assert_eq!(hash_of(&scoped, "util#build"), hash_of(&full, "util#build"));
}

#[test]
fn turbo_root_inputs_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "inputs": ["$TURBO_DEFAULT$", "$TURBO_ROOT$/foo.txt"],
      "outputs": []
    }
  }
}"#,
    );

    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked",
    )
    .unwrap();

    let log = run_with_debug_log(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert!(
        log.contains("untracked-file scan scope: whole repo (not provably scoped)"),
        "$TURBO_ROOT$ inputs must keep the whole-repo scan, log:\n{log}"
    );

    let scoped = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    let full = task_hashes(tempdir.path(), &["util#build"]);
    assert_eq!(hash_of(&scoped, "util#build"), hash_of(&full, "util#build"));
}

#[test]
fn participating_root_tasks_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(
        tempdir.path(),
        r#"{
  "$schema": "https://turborepo.dev/schema.json",
  "tasks": {
    "build": {
      "outputs": []
    },
    "//#build": {
      "inputs": ["foo.txt"],
      "outputs": []
    }
  }
}"#,
    );

    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked",
    )
    .unwrap();

    // The root task is defined but does not participate in a package-only
    // selection, so the run still scopes.
    let package_only = run_with_debug_log(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert!(
        package_only.contains("untracked-file scan scope: package directory prefixes"),
        "a non-participating root task must not prevent scoping, log:\n{package_only}"
    );

    // Requesting the root task pulls the repo root into the hashed inputs,
    // so the scan must cover the whole repo.
    let with_root = run_with_debug_log(tempdir.path(), &["//#build", "--filter=util"]);
    assert!(
        with_root.contains("untracked-file scan scope: whole repo"),
        "a participating root task must keep the whole-repo scan, log:\n{with_root}"
    );
}

#[test]
fn affected_runs_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);
    git(tempdir.path(), &["checkout", "-b", "my-branch"]);
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("change.txt"),
        "changed on my-branch",
    )
    .unwrap();
    git(tempdir.path(), &["add", "."]);
    git(tempdir.path(), &["commit", "-m", "change util", "--quiet"]);

    let log = run_with_debug_log(tempdir.path(), &["build", "--affected"]);
    assert!(
        log.contains("untracked-file scan scope: whole repo"),
        "--affected must keep the whole-repo scan, log:\n{log}"
    );

    let hashes = task_hashes(tempdir.path(), &["build", "--affected"]);
    assert!(
        hashes.iter().any(|(id, _)| id == "util#build"),
        "expected util#build to be affected: {hashes:?}"
    );
}

#[test]
fn unfiltered_runs_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked",
    )
    .unwrap();

    let log = run_with_debug_log(tempdir.path(), &["build"]);
    assert!(
        log.contains("untracked-file scan scope: whole repo (not a scoping candidate)"),
        "a run without filters must keep the whole-repo scan, log:\n{log}"
    );

    // The unfiltered run hashes the task exactly like the scoped filtered
    // run for the same repo state.
    let full = task_hashes(tempdir.path(), &["build"]);
    let scoped = task_hashes(tempdir.path(), &["build", "--filter=util", "--only"]);
    assert_eq!(hash_of(&full, "util#build"), hash_of(&scoped, "util#build"));
}

#[test]
fn exclude_only_filters_keep_whole_repo_scan() {
    let tempdir = tempfile::tempdir().unwrap();
    setup_repo(tempdir.path(), PLAIN_TURBO_JSON);
    fs::write(
        tempdir
            .path()
            .join("packages")
            .join("util")
            .join("untracked.ts"),
        "untracked",
    )
    .unwrap();

    // An exclude-only filter selects every remaining package, so it can
    // never scope and must not wait on a scope decision.
    let log = run_with_debug_log(tempdir.path(), &["build", "--filter=!util"]);
    assert!(
        log.contains("untracked-file scan scope: whole repo (not a scoping candidate)"),
        "an exclude-only filter must keep the whole-repo scan, log:\n{log}"
    );

    let hashes = task_hashes(tempdir.path(), &["build", "--filter=!util"]);
    assert!(
        hashes.iter().any(|(id, _)| id == "my-app#build"),
        "expected my-app#build to run: {hashes:?}"
    );
    assert!(
        !hashes.iter().any(|(id, _)| id == "util#build"),
        "util was excluded: {hashes:?}"
    );
}

#[test]
fn watch_runs_hash_untracked_files_throughout_the_session() {
    let tempdir = tempfile::tempdir().unwrap();
    let test_dir = tempdir.path();

    setup::copy_fixture("watch_test", test_dir).unwrap();
    setup::setup_git(test_dir).unwrap();

    // Marker files must not join the hashes.
    let gitignore = test_dir.join(".gitignore");
    let mut gi = fs::read_to_string(&gitignore).unwrap_or_default();
    gi.push_str(".markers/\n");
    fs::write(&gitignore, gi).unwrap();
    git(test_dir, &["add", "."]);
    git(test_dir, &["commit", "-m", "ignore markers", "--quiet"]);

    // An untracked file inside the watched package exists before the first
    // run; another one appears mid-session.
    fs::write(
        test_dir
            .join("packages")
            .join("a")
            .join("untracked-input.txt"),
        "untracked input v1",
    )
    .unwrap();

    let marker_count = |expected: usize, timeout: std::time::Duration| {
        let marker_dir = test_dir.join("packages").join("a").join(".markers");
        let start = std::time::Instant::now();
        loop {
            let count = fs::read_dir(&marker_dir)
                .map(|entries| entries.count())
                .unwrap_or(0);
            if count >= expected {
                return count;
            }
            if start.elapsed() > timeout {
                return count;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    };

    let turbo_bin = assert_cmd::cargo::cargo_bin("turbo");
    let config_dir = test_dir.join(".turbo").join("test-config");
    fs::create_dir_all(&config_dir).unwrap();
    let mut cmd = std::process::Command::new(turbo_bin);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    for key in common::ambient_turbo_env_keys() {
        cmd.env_remove(&key);
    }
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .env("TURBO_CONFIG_DIR_PATH", &config_dir)
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .args(["watch", "build", "--filter=pkg-a"]);
    let child = cmd.spawn().expect("failed to spawn turbo watch");

    fn stop_watch(child: &mut std::process::Child) {
        #[cfg(unix)]
        {
            let pgid = nix::unistd::Pid::from_raw(-(child.id() as i32));
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGTERM);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while matches!(child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = nix::sys::signal::kill(pgid, nix::sys::signal::Signal::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = child.kill();
        }
        let _ = child.wait();
    }

    /// Stops the watch process even if an assertion below panics, so a
    /// failing test cannot leak the child and its descendants.
    struct WatchGuard(Option<std::process::Child>);
    impl Drop for WatchGuard {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                stop_watch(&mut child);
            }
        }
    }

    let mut guard = WatchGuard(Some(child));

    // Initial run: the scoped-eligible filtered run must hash the untracked
    // file and execute.
    let after_initial = marker_count(1, std::time::Duration::from_secs(30));
    assert!(
        after_initial >= 1,
        "initial watch run did not execute within timeout"
    );

    // A new untracked file triggers a rebuild, which reruns with watch-mode
    // changed files (whole-repo scan).
    fs::write(
        test_dir.join("packages").join("a").join("untracked-2.txt"),
        "untracked input v2",
    )
    .unwrap();
    let after_untracked = marker_count(after_initial + 1, std::time::Duration::from_secs(30));
    assert!(
        after_untracked > after_initial,
        "untracked file change did not trigger a rebuild"
    );

    // A tracked change still rebuilds.
    fs::write(
        test_dir.join("packages").join("a").join("src.js"),
        "module.exports = { a: 43 };\n",
    )
    .unwrap();
    git(test_dir, &["add", "."]);
    git(test_dir, &["commit", "-m", "change src", "--quiet"]);
    let after_tracked = marker_count(after_untracked + 1, std::time::Duration::from_secs(30));
    assert!(
        after_tracked > after_untracked,
        "tracked file change did not trigger a rebuild"
    );

    // Stop eagerly; Drop is the safety net.
    if let Some(mut child) = guard.0.take() {
        stop_watch(&mut child);
    }
}
