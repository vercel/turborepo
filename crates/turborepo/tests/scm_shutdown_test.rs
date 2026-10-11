#![cfg(unix)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use common::process::{unix::TaskTreeGuard, wait_for_process_exit};
use tempfile::TempDir;

/// Releases fixture gates and reaps Turbo even when an assertion panics.
struct RunGuard {
    child: Child,
    tree: Option<TaskTreeGuard>,
    git_tree: Option<TaskTreeGuard>,
    root: PathBuf,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        let _ = fs::write(self.root.join(".git/release-task"), "release");
        let _ = fs::write(self.root.join(".git/release-diff"), "release");
        // The managed Git child has its own process group. Releasing its
        // fixture gate lets normal scope cleanup run before a failure fallback.
        let deadline = Instant::now() + Duration::from_secs(1);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            // Drop the process-group guard before reaping its leader.
            drop(self.tree.take());
            drop(self.git_tree.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct Fixture {
    root: TempDir,
    shim: TempDir,
    real_git: PathBuf,
    path: std::ffi::OsString,
}

impl Fixture {
    fn new(cache: bool, fail: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let shim = tempfile::tempdir().unwrap();
        // An unborn repository exercises the cached-diff fallback without
        // creating commits or needing a signing key in the test environment.
        common::git(root.path(), &["init", "--quiet"]);
        fs::write(root.path().join("package.json"), r#"{"name":"scm-shutdown","private":true,"packageManager":"npm@10.8.2","scripts":{"build":"node task.cjs","hold":"node hold.cjs"}}"#).unwrap();
        fs::write(
            root.path().join("turbo.json"),
            serde_json::json!({
                "agentGuidance": false,
                "tasks": {"build": {"cache": cache}, "hold": {"cache": false}}
            })
            .to_string(),
        )
        .unwrap();
        fs::write(root.path().join(".gitattributes"), "*.png binary\n").unwrap();
        fs::write(root.path().join(".gitignore"), ".turbo\n").unwrap();
        fs::write(
            root.path().join("task.cjs"),
            format!(
                r#"
const fs = require('node:fs');
fs.writeFileSync('.git/task-started', 'started');
const deadline = Date.now() + 30000;
while (!fs.existsSync('.git/release-task')) {{
  if (Date.now() > deadline) process.exit(99);
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
}}
fs.writeFileSync('.git/task-finished', 'done');
process.exit({});
"#,
                i32::from(fail)
            ),
        )
        .unwrap();
        fs::write(
            root.path().join("hold.cjs"),
            r#"
const fs = require('node:fs');
fs.writeFileSync('.git/hold-started', 'started');
setTimeout(() => process.exit(99), 30000);
setInterval(() => {}, 1000);
"#,
        )
        .unwrap();
        common::git(root.path(), &["add", "."]);
        let real_git = which::which("git").unwrap();
        let git = shim.path().join("git");
        fs::write(
            &git,
            r#"#!/usr/bin/env node
const fs = require('node:fs');
const cp = require('node:child_process');
const args = process.argv.slice(2);
const diffArgs = args[0] === '-c' ? args.slice(2) : args;
const gated = diffArgs.join(' ') === 'diff HEAD --no-ext-diff --no-color';
if (gated) {
  // Simulate the lock held during Git's opportunistic refresh, including
  // an unresponsive child that requires the owned scope's force fallback.
  const index = process.env.GIT_INDEX_FILE || '.git/index';
  fs.writeFileSync(index + '.lock', 'locked');
  fs.writeFileSync('.git/private-index', index);
  if (fs.existsSync('.git/ignore-interrupt')) process.on('SIGINT', () => {});
  fs.writeFileSync('.git/diff-started', String(process.pid));
  const deadline = Date.now() + 30000;
  while (!fs.existsSync('.git/release-diff')) {
    if (Date.now() > deadline) process.exit(99);
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
  }
  fs.unlinkSync(index + '.lock');
}
// A tree object exercises the same diff refresh path without creating a commit.
if (gated && fs.existsSync('.git/diff-tree')) {
  args[args.indexOf('HEAD')] = fs.readFileSync('.git/diff-tree', 'utf8').trim();
}
const result = cp.spawnSync(process.env.REAL_GIT, args, {stdio: 'inherit'});
if (gated) fs.writeFileSync('.git/diff-finished', 'done');
process.exit(result.status ?? 1);
"#,
        )
        .unwrap();
        fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths(
            std::iter::once(shim.path().to_path_buf())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        Self {
            root,
            shim,
            real_git,
            path,
        }
    }

    fn marker(&self, name: &str) -> PathBuf {
        self.root.path().join(".git").join(name)
    }

    fn release(&self, name: &str) {
        fs::write(self.marker(name), "release").unwrap();
    }

    fn start(&self, hold: bool) -> RunGuard {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin("turbo"));
        for key in common::ambient_turbo_env_keys() {
            command.env_remove(key);
        }
        command
            .current_dir(self.root.path())
            .env("DO_NOT_TRACK", "1")
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .env("PATH", &self.path)
            .env("REAL_GIT", &self.real_git)
            .env("TURBO_CONFIG_DIR_PATH", self.shim.path().join("config"))
            .args(["run", "build", "--ui=stream"])
            .process_group(0)
            .stdout(Stdio::from(
                fs::File::create(self.shim.path().join("stdout")).unwrap(),
            ))
            .stderr(Stdio::from(
                fs::File::create(self.shim.path().join("stderr")).unwrap(),
            ));
        if hold {
            command.arg("hold");
        }
        let child = command.spawn().unwrap();
        let tree = TaskTreeGuard::new(child.id() as i32);
        RunGuard {
            child,
            tree: Some(tree),
            git_tree: None,
            root: self.root.path().to_path_buf(),
        }
    }

    fn wait_for(&self, run: &mut RunGuard, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.marker(name).exists() {
            assert!(
                run.child.try_wait().unwrap().is_none(),
                "Turbo exited before {name}: {}",
                self.output()
            );
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {name}: {}",
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
        if name == "diff-started" {
            let pid = fs::read_to_string(self.marker(name))
                .unwrap()
                .parse()
                .unwrap();
            run.git_tree = Some(TaskTreeGuard::new(pid));
        }
    }

    fn output(&self) -> String {
        ["stdout", "stderr"]
            .iter()
            .map(|name| fs::read_to_string(self.shim.path().join(name)).unwrap_or_default())
            .collect()
    }

    fn assert_git_gone(&self) {
        let pid: i32 = fs::read_to_string(self.marker("diff-started"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            !common::process::unix::process_exists(pid),
            "Git child survived Turbo"
        );
        let index = PathBuf::from(fs::read_to_string(self.marker("private-index")).unwrap());
        assert!(index.is_absolute());
        assert_ne!(index, self.marker("index"));
        assert!(!index.exists(), "temporary index was not removed");
        assert!(!PathBuf::from(format!("{}.lock", index.display())).exists());
        assert!(!self.marker("index.lock").exists());
        common::git(self.root.path(), &["update-index", "--refresh"]);
    }
}

#[test]
fn cache_disabled_does_not_start_scm_git() {
    check_no_capture(false, false);
}

#[test]
fn task_failure_does_not_start_scm_git() {
    check_no_capture(true, true);
}

fn check_no_capture(cache: bool, fail: bool) {
    let fixture = Fixture::new(cache, fail);
    let mut run = fixture.start(false);
    fixture.wait_for(&mut run, "task-started");
    // Keep Turbo alive while an eagerly spawned provenance task would run.
    thread::sleep(Duration::from_millis(200));
    fixture.release("release-task");
    let status = wait_for_process_exit(&mut run.child, Duration::from_secs(10));
    assert_eq!(status.success(), !fail, "{}", fixture.output());
    assert!(
        !fixture.marker("diff-started").exists(),
        "unused Git capture started"
    );
}

#[test]
fn cache_hit_does_not_start_scm_git() {
    let fixture = Fixture::new(true, false);
    fixture.release("release-task");
    fixture.release("release-diff");
    let mut warmup = fixture.start(false);
    assert!(
        wait_for_process_exit(&mut warmup.child, Duration::from_secs(15)).success(),
        "{}",
        fixture.output()
    );
    fixture.assert_git_gone();
    for name in [
        "diff-started",
        "diff-finished",
        "private-index",
        "task-started",
        "task-finished",
    ] {
        fs::remove_file(fixture.marker(name)).unwrap();
    }
    let mut run = fixture.start(false);
    assert!(
        wait_for_process_exit(&mut run.child, Duration::from_secs(10)).success(),
        "{}",
        fixture.output()
    );
    assert!(
        fixture.output().contains("cache hit"),
        "{}",
        fixture.output()
    );
    assert!(!fixture.marker("task-started").exists());
    assert!(
        !fixture.marker("diff-started").exists(),
        "cache hit started Git capture"
    );
}

#[test]
fn cache_write_waits_for_scm_git() {
    let fixture = Fixture::new(true, false);
    fixture.release("release-task");
    let mut run = fixture.start(false);
    fixture.wait_for(&mut run, "diff-started");
    assert!(
        !fixture.marker("index.lock").exists(),
        "provenance locked real index"
    );
    common::git(fixture.root.path(), &["update-index", "--refresh"]);
    thread::sleep(Duration::from_millis(200));
    assert!(
        run.child.try_wait().unwrap().is_none(),
        "Turbo abandoned Git"
    );
    fixture.release("release-diff");
    assert!(
        wait_for_process_exit(&mut run.child, Duration::from_secs(10)).success(),
        "{}",
        fixture.output()
    );
    assert!(fixture.marker("diff-finished").exists());
    let metadata: Vec<serde_json::Value> = fs::read_dir(fixture.root.path().join(".turbo/cache"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with("-meta.json"))
        .map(|entry| serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap())
        .collect();
    assert!(
        metadata.iter().any(|meta| meta["dirty_hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty())),
        "provenance was not recorded"
    );
    fixture.assert_git_gone();
}

fn index_snapshot(fixture: &Fixture) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(fixture.root.path().join(".git"))
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry.file_name() == "index"
                || entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("sharedindex.")
        })
        .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
        .collect();
    files.sort();
    files
}

#[test]
fn split_index_is_preserved_during_capture_and_cancellation() {
    for cancel in [false, true] {
        let fixture = Fixture::new(true, false);
        let tree = Command::new(&fixture.real_git)
            .current_dir(fixture.root.path())
            .arg("write-tree")
            .output()
            .unwrap();
        assert!(tree.status.success());
        fs::write(fixture.marker("diff-tree"), tree.stdout).unwrap();
        common::git(
            fixture.root.path(),
            &["config", "splitIndex.maxPercentChange", "0"],
        );
        common::git(fixture.root.path(), &["update-index", "--split-index"]);
        let tracked = fixture.root.path().join("task.cjs");
        let bytes = fs::read(&tracked).unwrap();
        fs::write(&tracked, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&tracked)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
            .unwrap();
        let before = index_snapshot(&fixture);
        assert!(
            before
                .iter()
                .any(|(name, _)| name.to_string_lossy().starts_with("sharedindex."))
        );
        fixture.release("release-task");
        fixture.release("ignore-interrupt");
        let mut run = fixture.start(cancel);
        fixture.wait_for(&mut run, "diff-started");
        assert_eq!(index_snapshot(&fixture), before);
        if cancel {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(run.child.id() as i32),
                nix::sys::signal::Signal::SIGINT,
            )
            .unwrap();
        } else {
            fixture.release("release-diff");
        }
        let status = wait_for_process_exit(&mut run.child, Duration::from_secs(10));
        if !cancel {
            assert!(status.success(), "{}", fixture.output());
        }
        assert_eq!(index_snapshot(&fixture), before, "cancel={cancel}");
        fixture.assert_git_gone();
    }
}

fn check_cancel(hold: bool, second_signal: bool, signal: bool) {
    let fixture = Fixture::new(true, false);
    fixture.release("release-task");
    fixture.release("ignore-interrupt");
    let mut run = fixture.start(hold);
    fixture.wait_for(&mut run, "diff-started");
    if hold {
        fixture.wait_for(&mut run, "hold-started");
    }
    if signal {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(run.child.id() as i32),
            nix::sys::signal::Signal::SIGINT,
        )
        .unwrap();
        if second_signal {
            thread::sleep(Duration::from_millis(50));
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(run.child.id() as i32),
                nix::sys::signal::Signal::SIGINT,
            );
        }
    }
    wait_for_process_exit(
        &mut run.child,
        Duration::from_secs(if signal { 5 } else { 10 }),
    );
    fixture.assert_git_gone();
}

#[test]
fn interrupted_cache_write_reaps_scm_git() {
    check_cancel(true, false, true);
}

#[test]
fn repeated_ctrl_c_reaps_scm_git() {
    check_cancel(true, true, true);
}

#[test]
fn ctrl_c_after_tasks_finish_reaps_scm_git() {
    check_cancel(false, false, true);
}

#[test]
fn normal_scm_drain_is_bounded() {
    check_cancel(false, false, false);
}
