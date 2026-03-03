mod common;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

use common::setup;

/// Count the number of marker files in a package's `.markers` directory.
/// Each run of the build script creates a new marker file.
fn marker_count(test_dir: &Path, pkg: &str) -> usize {
    let marker_dir = test_dir.join("packages").join(pkg).join(".markers");
    if !marker_dir.exists() {
        return 0;
    }
    fs::read_dir(&marker_dir)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// Count marker files matching a given prefix (e.g. "build-" or "dev-").
fn prefixed_marker_count(test_dir: &Path, pkg: &str, prefix: &str) -> usize {
    let marker_dir = test_dir.join("packages").join(pkg).join(".markers");
    if !marker_dir.exists() {
        return 0;
    }
    fs::read_dir(&marker_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                .count()
        })
        .unwrap_or(0)
}

/// Wait until the prefixed marker count for a package reaches at least
/// `expected`, or timeout.
fn wait_for_prefixed_markers(
    test_dir: &Path,
    pkg: &str,
    prefix: &str,
    expected: usize,
    timeout: Duration,
) -> usize {
    let start = Instant::now();
    loop {
        let count = prefixed_marker_count(test_dir, pkg, prefix);
        if count >= expected {
            return count;
        }
        if start.elapsed() > timeout {
            return count;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Read the timestamp from a marker file. Returns None if the file doesn't
/// exist or can't be parsed.
fn read_marker_timestamp(test_dir: &Path, pkg: &str, marker_name: &str) -> Option<u64> {
    let path = test_dir
        .join("packages")
        .join(pkg)
        .join(".markers")
        .join(marker_name);
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Wait until the marker count for a package reaches at least `expected`,
/// or timeout. Returns the final marker count.
fn wait_for_markers(test_dir: &Path, pkg: &str, expected: usize, timeout: Duration) -> usize {
    let start = Instant::now();
    loop {
        let count = marker_count(test_dir, pkg);
        if count >= expected {
            return count;
        }
        if start.elapsed() > timeout {
            return count;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Spawn `turbo watch` with the given tasks as a child process.
fn spawn_turbo_watch_with_tasks(test_dir: &Path, tasks: &[&str]) -> Child {
    let turbo_bin = assert_cmd::cargo::cargo_bin("turbo");
    let mut cmd = std::process::Command::new(turbo_bin);
    cmd.arg("watch");
    for task in tasks {
        cmd.arg(task);
    }
    cmd.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn turbo watch")
}

/// Spawn `turbo watch build` as a child process.
fn spawn_turbo_watch(test_dir: &Path) -> Child {
    spawn_turbo_watch_with_tasks(test_dir, &["build"])
}

/// Gracefully stop a turbo watch process.
fn stop_watch(mut child: Child) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{self, Signal},
            unistd::Pid,
        };
        let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = child.kill();
    }

    let _ = child.wait();
}

/// RAII guard that ensures `stop_watch` runs even if a test panics.
/// Without this, a panic between `spawn_turbo_watch` and `stop_watch` would
/// leak the turbo process and its daemon, causing socket contention for
/// subsequent serialized tests.
struct WatchGuard(Option<Child>);

impl WatchGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn take(mut self) -> Child {
        self.0.take().expect("WatchGuard already consumed")
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            stop_watch(child);
        }
    }
}

/// Modify `packages/a/src.js`, commit, and wait for the marker count to
/// increase. If the watcher doesn't pick up the change within 15 seconds,
/// retry with different content up to `max_attempts` times. Returns the
/// final marker count.
fn retry_file_change(test_dir: &Path, pkg: &str, before: usize, max_attempts: usize) -> usize {
    let src_file = test_dir.join(format!("packages/{pkg}/src.js"));
    for attempt in 0..max_attempts {
        let value = 42 + attempt;
        fs::write(&src_file, format!("module.exports = {{ a: {value} }};\n")).unwrap();

        let status = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(test_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git add failed to execute");
        assert!(status.success(), "git add failed with {status}");

        let status = std::process::Command::new("git")
            .args([
                "commit",
                "-m",
                &format!("modify {pkg} (attempt {attempt})"),
                "--quiet",
            ])
            .current_dir(test_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git commit failed to execute");
        assert!(status.success(), "git commit failed with {status}");

        let count = wait_for_markers(test_dir, pkg, before + 1, Duration::from_secs(15));
        if count > before {
            return count;
        }
    }
    marker_count(test_dir, pkg)
}

fn setup_watch_test() -> (tempfile::TempDir, PathBuf) {
    let tempdir = tempfile::tempdir().expect("failed to create tempdir");
    let test_dir = tempdir.path().to_path_buf();

    setup::copy_fixture("watch_test", &test_dir).unwrap();
    setup::setup_git(&test_dir).unwrap();

    // Add .markers to .gitignore so marker files don't appear in git-based
    // hashes. Must be committed before turbo watch starts because the hash
    // watcher operates on committed state.
    let gitignore = test_dir.join(".gitignore");
    let mut gi = fs::read_to_string(&gitignore).unwrap_or_default();
    gi.push_str(".markers/\n");
    fs::write(&gitignore, gi).unwrap();

    common::git(&test_dir, &["add", "."]);
    common::git(
        &test_dir,
        &["commit", "-m", "add markers ignore", "--quiet"],
    );

    (tempdir, test_dir)
}

#[test]
fn watch_initial_run_executes_tasks() {
    let (_tempdir, test_dir) = setup_watch_test();
    let guard = WatchGuard::new(spawn_turbo_watch(&test_dir));

    // Wait for the initial build to complete
    let a_count = wait_for_markers(&test_dir, "a", 1, Duration::from_secs(30));
    let b_count = wait_for_markers(&test_dir, "b", 1, Duration::from_secs(30));

    drop(guard);

    assert!(
        a_count >= 1,
        "package a should have run at least once, ran {a_count} times"
    );
    assert!(
        b_count >= 1,
        "package b should have run at least once, ran {b_count} times"
    );
}

#[test]
fn watch_file_change_reruns_affected_package() {
    let (_tempdir, test_dir) = setup_watch_test();
    let guard = WatchGuard::new(spawn_turbo_watch(&test_dir));

    // Wait for initial run
    wait_for_markers(&test_dir, "a", 1, Duration::from_secs(30));
    wait_for_markers(&test_dir, "b", 1, Duration::from_secs(30));

    // Let the watcher fully settle after the initial build. The daemon's
    // file watcher, hash watcher, and package changes watcher all process
    // events asynchronously. Without this, the file modification below can
    // race with the tail end of initial-build event processing, causing the
    // filesystem event to be coalesced or the hash check to use stale state.
    std::thread::sleep(Duration::from_secs(2));

    let a_before = marker_count(&test_dir, "a");

    // Modify a file in package a and commit. The hash watcher uses
    // git-based hashing, so the change must be committed for the watcher
    // to detect a different hash.
    //
    // Retry up to 3 times: on macOS, FSEvents can occasionally coalesce
    // or delay events for files in temp directories, causing the watcher
    // to miss a change. Each retry writes new content and commits,
    // generating fresh filesystem events.
    let a_after = retry_file_change(&test_dir, "a", a_before, 3);

    drop(guard);

    assert!(
        a_after > a_before,
        "package a should have re-run after file change. before: {a_before}, after: {a_after}"
    );

    // Package b depends on a, so it may or may not re-run depending on whether
    // turbo detects the transitive dependency. We don't assert on b here since
    // the dep-change propagation is tested at the engine level.
}

#[cfg(unix)]
#[test]
fn watch_clean_shutdown_on_sigint() {
    use nix::{
        sys::signal::{self, Signal},
        unistd::Pid,
    };

    let (_tempdir, test_dir) = setup_watch_test();
    let guard = WatchGuard::new(spawn_turbo_watch(&test_dir));

    // Give it time to start
    wait_for_markers(&test_dir, "a", 1, Duration::from_secs(30));

    let mut child = guard.take();
    let pid = Pid::from_raw(child.id() as i32);
    signal::kill(pid, Signal::SIGINT).expect("failed to send SIGINT");

    // Wait for process to exit
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process exited — turbo watch exits with non-zero on
                // signal interrupt, which is expected.
                return;
            }
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(10) {
                    child.kill().unwrap();
                    panic!("turbo watch did not exit within 10s after SIGINT");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("error waiting for turbo watch: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Watch dependency ordering tests (watch_deps_test fixture)
// ---------------------------------------------------------------------------

fn setup_watch_deps_test() -> (tempfile::TempDir, PathBuf) {
    let tempdir = tempfile::tempdir().expect("failed to create tempdir");
    let test_dir = tempdir.path().to_path_buf();

    setup::copy_fixture("watch_deps_test", &test_dir).unwrap();
    setup::setup_git(&test_dir).unwrap();

    let gitignore = test_dir.join(".gitignore");
    let mut gi = fs::read_to_string(&gitignore).unwrap_or_default();
    gi.push_str(".markers/\n");
    fs::write(&gitignore, gi).unwrap();

    common::git(&test_dir, &["add", "."]);
    common::git(
        &test_dir,
        &["commit", "-m", "add markers ignore", "--quiet"],
    );

    (tempdir, test_dir)
}

/// Verify that persistent tasks (dev) only start after non-persistent tasks
/// (build) complete successfully. The `dev` task has `dependsOn: ["build"]`
/// and `persistent: true` (non-interruptible by default), so the watch
/// coordinator should gate it behind the build.
#[test]
fn watch_persistent_tasks_wait_for_build() {
    let (_tempdir, test_dir) = setup_watch_deps_test();
    let guard = WatchGuard::new(spawn_turbo_watch_with_tasks(&test_dir, &["dev"]));

    // Wait for dev markers to appear — this implies build already ran since
    // dev depends on build and is gated behind its success.
    let dev_a = wait_for_prefixed_markers(&test_dir, "a", "dev-", 1, Duration::from_secs(60));
    let build_a = prefixed_marker_count(&test_dir, "a", "build-");

    drop(guard);

    assert!(
        build_a >= 1,
        "build should have run before dev, but build ran {build_a} times"
    );
    assert!(
        dev_a >= 1,
        "dev should have started after build, but dev ran {dev_a} times"
    );

    // Verify ordering via timestamps: build must have started before dev
    let build_ts = read_marker_timestamp(&test_dir, "a", "build-0");
    let dev_ts = read_marker_timestamp(&test_dir, "a", "dev-0");

    if let (Some(build_t), Some(dev_t)) = (build_ts, dev_ts) {
        assert!(
            build_t <= dev_t,
            "build timestamp ({build_t}) should be <= dev timestamp ({dev_t})"
        );
    }
}

/// Verify that when build fails, persistent dev tasks never start.
#[test]
fn watch_persistent_tasks_skipped_on_build_failure() {
    let (_tempdir, test_dir) = setup_watch_deps_test();

    // Overwrite build.js in package a to exit with code 1
    let failing_build = r#"
const fs = require('fs');
const path = require('path');
const markerDir = path.join(__dirname, '.markers');
fs.mkdirSync(markerDir, { recursive: true });
const count = fs.readdirSync(markerDir).filter(f => f.startsWith('build-')).length;
fs.writeFileSync(path.join(markerDir, `build-${count}`), `${Date.now()}\n`);
console.log(`pkg-a build #${count} (FAILING)`);
process.exit(1);
"#;
    fs::write(test_dir.join("packages/a/build.js"), failing_build).unwrap();

    common::git(&test_dir, &["add", "."]);
    common::git(&test_dir, &["commit", "-m", "make build fail", "--quiet"]);

    let guard = WatchGuard::new(spawn_turbo_watch_with_tasks(&test_dir, &["dev"]));

    // Wait long enough for build to run and fail
    wait_for_prefixed_markers(&test_dir, "a", "build-", 1, Duration::from_secs(30));

    // Give extra time to confirm dev does NOT start
    std::thread::sleep(Duration::from_secs(5));

    let dev_a = prefixed_marker_count(&test_dir, "a", "dev-");

    drop(guard);

    assert_eq!(
        dev_a, 0,
        "dev should not have started because build failed, but dev ran {dev_a} times"
    );
}
