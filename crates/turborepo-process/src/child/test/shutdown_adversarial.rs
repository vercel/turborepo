//! Adversarial graceful-shutdown contracts for piped and PTY children.
//!
//! These tests drive `ChildHandle::send_graceful_interrupt` and its
//! `send_interrupt_to_remaining_descendants` follow-up through the
//! deterministic `test/scripts/shutdown_fixture.js` fixture.
//!
//! What we actually promise, and therefore assert:
//!
//! * Given a cooperative worker, our shutdown delivers at least one interrupt
//!   it can act on, and `Child::stop` does not report completion until the
//!   worker has recorded that its cleanup finished.
//! * The owned process group is gone before the test returns.
//! * No fixture watchdog fired and no unexpected IO error occurred.
//! * The final cleanup line is captured for pipe children and for PTY children
//!   whose session leader stays alive.
//!
//! What we deliberately do not promise: an exact signal count when arbitrary
//! wrappers forward. Turborepo signals the process group on the pipe path, and
//! a wrapper may forward that signal to its child on top of the group delivery.
//! Duplicate SIGINTs are therefore legitimate and are kept as observations.
//!
//! Capture scope is separate from the counts. A PTY session leader that exits
//! early hangs up the terminal for the rest of the group; the worker tolerates
//! SIGHUP and still proves cleanup through its file marker, but output written
//! after the hangup is not a guarantee. Those cases are process-tree completion
//! tests. Pipe cases, and PTY cases whose session leader stays alive, still
//! require the final cleanup output.
//!
//! One focused regression pins down a duplicate that is ours, not a wrapper's:
//! nested nonforwarding wrappers under PTY get the group SIGINT and then our
//! remaining-descendant fallback signals the worker a second time. With every
//! wrapper known nonforwarding, an exact-one worker assertion is legitimate.

use std::{
    assert_matches,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use test_case::test_case;
use tracing_test::traced_test;

use super::{ObservedOutput, TEST_PTY, find_script_dir};
use crate::{
    Child, Command, PtySize,
    child::{
        ChildExit, ShutdownStyle,
        handle::{GracefulDescendant, descendants_needing_fallback, mark_group_targets},
    },
};

const READY_LIMIT: Duration = Duration::from_secs(15);
const FINISH_LIMIT: Duration = Duration::from_secs(15);
const GRACE_TIMEOUT: Duration = Duration::from_secs(8);
const SLOW_CLEANUP_MS: u64 = 1500;
const FAST_CLEANUP_MS: u64 = 200;
const FORWARD_DELAY_MS: u64 = 300;
const FIXTURE_WATCHDOG_MS: u64 = 30_000;

static NEXT_STATE_DIR: AtomicU64 = AtomicU64::new(0);

fn unique_state_dir(label: &str) -> PathBuf {
    loop {
        let id = NEXT_STATE_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "turbo-shutdown-{}-{id}-{label}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => panic!(
                "failed to create fixture state dir {}: {err}",
                dir.display()
            ),
        }
    }
}

/// Kills the fixture's process group and removes the state dir even if the test
/// panics. The root is its own group leader on Unix (`process_group(0)` for
/// piped children, a fresh session for PTY children) and descendants inherit
/// that group, so one `kill(-pgid, SIGKILL)` signals whatever is left.
struct CleanupGuard {
    pgid: Option<libc::pid_t>,
    state_dir: PathBuf,
}

impl CleanupGuard {
    fn new(state_dir: PathBuf) -> Self {
        Self {
            pgid: None,
            state_dir,
        }
    }

    /// Drop the panic-path kill; `Drop` still removes the state dir.
    fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
        let _ = fs::remove_dir_all(&self.state_dir);
    }
}

#[derive(Clone, Copy, Debug)]
enum Forward {
    /// Never signal the child.
    None,
    /// Forward after a delay, on top of whatever the group delivery did.
    Delayed,
}

impl Forward {
    fn as_arg(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Delayed => "delayed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ExitMode {
    /// Exit as soon as the interrupt is handled.
    Now,
    /// Stay alive until the child exits.
    AfterChild,
    /// Nonforwarding only: wait for the worker's file acknowledgment, then
    /// exit. This is the readiness barrier that makes our fallback re-signal a
    /// distinct delivery instead of a coalesced one.
    AfterAck,
}

impl ExitMode {
    fn as_arg(self) -> &'static str {
        match self {
            Self::Now => "now",
            Self::AfterChild => "after-child",
            Self::AfterAck => "after-ack",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FixtureSpec {
    /// When false the fixture launches a plain worker (no wrapper).
    wrapper: bool,
    /// Number of wrapper levels above the worker.
    depth: u32,
    forward: Forward,
    forward_delay_ms: u64,
    exit_mode: ExitMode,
    worker_cleanup_ms: u64,
}

/// How the test asks the child to shut down.
#[derive(Clone, Copy, Debug)]
enum Driver {
    /// One `Child::stop` using the gracefully-configured style.
    Stop,
    /// Several `Child::shutdown(Graceful)` requests issued while the worker
    /// cleans up; only internal command idempotency is under test here.
    RepeatedGraceful(u32),
}

struct Outcome {
    use_pty: bool,
    exit: Option<ChildExit>,
    output: String,
    worker_sigint_count: u32,
    worker_cleanup_complete: bool,
    /// True when shutdown reported completion before the worker recorded its
    /// cleanup marker.
    stop_finished_before_cleanup: bool,
    state_dir: PathBuf,
    _guard: CleanupGuard,
}

impl Outcome {
    fn marker_present(&self, name: &str) -> bool {
        self.state_dir.join(name).exists()
    }

    /// True when the root wrapper exited before worker cleanup finished; the
    /// scenario assertion for declared early-exit cases.
    fn early_exit_observed(&self) -> bool {
        self.marker_present("root-exited")
    }

    /// Any fixture process wrote a watchdog marker instead of shutting down.
    fn watchdog_fired(&self) -> bool {
        self.marker_contains("watchdog")
    }

    /// A fixture process hit a stream error we do not consider expected.
    fn io_error_fired(&self) -> bool {
        self.marker_contains("io-error")
    }

    fn marker_contains(&self, needle: &str) -> bool {
        fs::read_dir(&self.state_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.file_name().to_string_lossy().contains(needle))
            })
            .unwrap_or(false)
    }

    fn diagnostic(&self, scenario: &str) -> String {
        use std::fmt::Write as _;

        let mode = if self.use_pty { "pty" } else { "pipe" };
        let mut report = String::new();
        let _ = writeln!(report, "=== shutdown adversarial: {scenario} ({mode}) ===");
        let _ = writeln!(report, "root_exit={:?}", self.exit);
        let _ = writeln!(report, "worker_sigint_count={}", self.worker_sigint_count);
        let _ = writeln!(
            report,
            "worker_cleanup_complete={}",
            self.worker_cleanup_complete
        );
        let _ = writeln!(
            report,
            "stop_finished_before_cleanup={}",
            self.stop_finished_before_cleanup
        );
        let _ = writeln!(report, "early_exit_observed={}", self.early_exit_observed());
        let _ = writeln!(report, "--- captured output ---\n{}", self.output);
        let _ = writeln!(report, "--- state files ---");

        match fs::read_dir(&self.state_dir) {
            Ok(entries) => {
                let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                files.sort();
                for path in files {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let contents = fs::read_to_string(&path)
                        .map(|c| c.trim().to_string())
                        .unwrap_or_else(|err| format!("<unreadable: {err}>"));
                    report.push_str(&format!("  {name}={contents}\n"));
                }
            }
            Err(err) => report.push_str(&format!("  <failed to list state dir: {err}>\n")),
        }

        report
    }

    /// The owned group must actually be gone. This is an assertion, not a
    /// tolerated wait: a lingering descendant fails the test, and the panic
    /// path still kills the group via `CleanupGuard`.
    async fn assert_owned_group_gone(&mut self) {
        let Some(pgid) = self._guard.pgid else {
            return;
        };
        let deadline = Instant::now() + FINISH_LIMIT;
        while Instant::now() < deadline && !process_group_gone(pgid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let diag = self.diagnostic("owned group teardown");
        assert!(
            process_group_gone(pgid),
            "owned process group {pgid} survived stop(){diag}"
        );
        self._guard.disarm();
    }
}

async fn wait_for_file(path: &Path, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait until the worker file records at least one SIGINT.
async fn wait_for_worker_ack(state_dir: &Path, limit: Duration) -> bool {
    let path = state_dir.join("worker-sigint-count");
    let deadline = Instant::now() + limit;
    loop {
        if let Ok(contents) = fs::read_to_string(&path)
            && contents.trim().parse::<u32>().is_ok_and(|count| count >= 1)
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// True once no process remains in the group. `kill(-pgid, 0)` only probes the
/// group; it never delivers a signal.
fn process_group_gone(pgid: libc::pid_t) -> bool {
    let result = unsafe { libc::kill(-pgid, 0) };
    result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn fixture_args(spec: FixtureSpec, state_dir: &Path) -> Vec<OsString> {
    let script = find_script_dir().join_component("shutdown_fixture.js");
    let script_arg = script.as_std_path().as_os_str().to_os_string();
    let state_arg = state_dir.as_os_str().to_os_string();

    if spec.wrapper {
        vec![
            script_arg,
            OsString::from("wrapper"),
            state_arg,
            OsString::from("1"),
            OsString::from(spec.depth.to_string()),
            OsString::from(spec.forward.as_arg()),
            OsString::from(spec.forward_delay_ms.to_string()),
            OsString::from(spec.exit_mode.as_arg()),
            OsString::from(spec.worker_cleanup_ms.to_string()),
            OsString::from(FIXTURE_WATCHDOG_MS.to_string()),
        ]
    } else {
        vec![
            script_arg,
            OsString::from("worker"),
            state_arg,
            OsString::from(spec.worker_cleanup_ms.to_string()),
            OsString::from(FIXTURE_WATCHDOG_MS.to_string()),
        ]
    }
}

async fn run_fixture(use_pty: bool, label: &str, spec: FixtureSpec, driver: Driver) -> Outcome {
    let state_dir = unique_state_dir(label);
    let mut guard = CleanupGuard::new(state_dir.clone());

    let mut cmd = Command::new("node");
    cmd.args(fixture_args(spec, &state_dir));
    cmd.open_stdin();

    let child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(GRACE_TIMEOUT)),
        use_pty.then(PtySize::default),
    )
    .expect("failed to spawn shutdown fixture");
    guard.pgid = child.pid().map(|pid| pid as libc::pid_t);

    let ready_marker = if spec.wrapper { "tree-ready" } else { "ready" };
    assert!(
        wait_for_file(&state_dir.join(ready_marker), READY_LIMIT).await,
        "fixture tree never became ready (pid={:?})",
        child.pid()
    );

    let mut output_child = child.clone();
    let (mut observer, output, _ready_rx) = ObservedOutput::new();
    let output_task =
        tokio::spawn(async move { output_child.wait_with_piped_outputs(&mut observer).await });

    child.set_closing();
    let cleanup_path = state_dir.join("worker-cleanup-complete");
    let mut stop_tasks = Vec::new();
    match driver {
        Driver::Stop => {
            let mut stop_child = child.clone();
            stop_tasks.push(tokio::spawn(async move { stop_child.stop().await }));
        }
        Driver::RepeatedGraceful(requests) => {
            let mut first = child.clone();
            stop_tasks.push(tokio::spawn(async move {
                first
                    .shutdown(ShutdownStyle::Graceful(Some(GRACE_TIMEOUT)))
                    .await
            }));
            assert!(
                wait_for_worker_ack(&state_dir, READY_LIMIT).await,
                "worker never acknowledged the first graceful interrupt"
            );
            // Duplicate requests only exercise in-flight idempotency if the
            // first request is still pending and cleanup has not yet finished.
            assert!(
                !stop_tasks[0].is_finished(),
                "first graceful request completed before duplicates were issued"
            );
            assert!(
                !cleanup_path.exists(),
                "worker cleanup completed before duplicates were issued"
            );
            for _ in 1..requests {
                let mut extra = child.clone();
                stop_tasks.push(tokio::spawn(async move {
                    extra
                        .shutdown(ShutdownStyle::Graceful(Some(GRACE_TIMEOUT)))
                        .await
                }));
            }
        }
    }

    // Watch that shutdown does not report completion while the worker is still
    // alive and cleaning up. The worker writes its completion marker
    // immediately before exiting, so the marker must appear first.
    let deadline = Instant::now() + FINISH_LIMIT;
    let mut stop_finished_before_cleanup = false;
    while !cleanup_path.exists() {
        if Instant::now() >= deadline {
            break;
        }
        if stop_tasks.iter().all(|task| task.is_finished()) {
            // The marker can land between the `exists()` probe and the finished
            // check; recheck before concluding shutdown outraced cleanup.
            if !cleanup_path.exists() {
                stop_finished_before_cleanup = true;
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let output_exit = tokio::time::timeout(FINISH_LIMIT, output_task)
        .await
        .expect("output drain did not finish")
        .expect("output task panicked")
        .expect("output reader failed");

    for task in &mut stop_tasks {
        let stop_exit = tokio::time::timeout(FINISH_LIMIT, task)
            .await
            .expect("stop() did not finish")
            .expect("stop task panicked");
        assert_eq!(
            stop_exit, output_exit,
            "output reader and a shutdown request disagreed on the child exit ({label})"
        );
    }

    let worker_sigint_count = fs::read_to_string(state_dir.join("worker-sigint-count"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let worker_cleanup_complete = cleanup_path.exists();
    let output = String::from_utf8_lossy(&output.lock().unwrap().clone()).into_owned();

    let mut outcome = Outcome {
        use_pty,
        exit: output_exit,
        output,
        worker_sigint_count,
        worker_cleanup_complete,
        stop_finished_before_cleanup,
        state_dir,
        _guard: guard,
    };

    outcome.assert_owned_group_gone().await;

    println!(
        "shutdown observation: label={label} mode={} sigint_count={} cleanup_complete={} \
         final_output_present={} early_exit={} stop_before_cleanup={}",
        if use_pty { "pty" } else { "pipe" },
        outcome.worker_sigint_count,
        outcome.worker_cleanup_complete,
        outcome.output.contains("worker cleanup complete"),
        outcome.early_exit_observed(),
        outcome.stop_finished_before_cleanup,
    );

    outcome
}

/// Contracts that hold for every scenario, independent of capture scope.
fn assert_clean_teardown(outcome: &Outcome, scenario: &str) {
    let diag = outcome.diagnostic(scenario);
    assert!(
        outcome.worker_cleanup_complete,
        "worker never proved cleanup completed{diag}"
    );
    assert!(
        outcome.output.contains("worker started") || outcome.worker_sigint_count > 0,
        "worker produced no evidence that it ever ran{diag}"
    );
    assert!(
        !outcome.stop_finished_before_cleanup,
        "shutdown reported complete while a tracked descendant was still cleaning up{diag}"
    );
    assert!(
        !outcome.watchdog_fired(),
        "a fixture watchdog fired instead of a clean shutdown{diag}"
    );
    assert!(
        !outcome.io_error_fired(),
        "a fixture process hit an unexpected stream error{diag}"
    );
}

fn assert_interrupted(outcome: &Outcome, scenario: &str) {
    let diag = outcome.diagnostic(scenario);
    assert_matches!(outcome.exit, Some(ChildExit::Interrupted), "{diag}");
}

/// Capture is a guarantee only for pipe children and for PTY children whose
/// session leader stays alive until the worker exits.
fn assert_final_cleanup_captured(outcome: &Outcome, scenario: &str) {
    let diag = outcome.diagnostic(scenario);
    assert!(
        outcome.output.contains("worker cleanup complete"),
        "final cleanup output was not captured, but capture is in scope for this scenario{diag}"
    );
}

/// Legitimate when no arbitrary forwarding is involved: the plain worker and
/// the known-nonforwarding regression.
fn assert_exactly_one_interrupt(outcome: &Outcome, scenario: &str) {
    let diag = outcome.diagnostic(scenario);
    assert_eq!(
        outcome.worker_sigint_count, 1,
        "expected exactly one intended interruption{diag}"
    );
}

/// The real contract for forwarding wrappers. Turborepo signals the process
/// group on the pipe path and the worker's own wrapper may forward on top of
/// that; both are valid deliveries, so we only require that the worker saw at
/// least one interrupt it could act on.
fn assert_at_least_one_interrupt(outcome: &Outcome, scenario: &str) {
    let diag = outcome.diagnostic(scenario);
    assert!(
        outcome.worker_sigint_count >= 1,
        "worker observed no interrupt{diag}"
    );
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_shutdown_control_plain_worker_observes_single_interrupt(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "control-plain-worker",
        FixtureSpec {
            wrapper: false,
            depth: 0,
            forward: Forward::None,
            forward_delay_ms: 0,
            exit_mode: ExitMode::AfterChild,
            worker_cleanup_ms: FAST_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    assert_clean_teardown(&outcome, "control plain worker");
    assert_interrupted(&outcome, "control plain worker");
    assert_exactly_one_interrupt(&outcome, "control plain worker");
    assert_final_cleanup_captured(&outcome, "control plain worker");
}

/// A forwarding wrapper stays alive until a slow worker finishes cleanup. The
/// session leader never exits early, so final output capture is in scope for
/// both modes. The worker may see the group SIGINT plus the wrapper's forward.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_shutdown_forwarding_wrapper_waits_for_slow_cleanup(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "forwarding-wrapper-slow-cleanup",
        FixtureSpec {
            wrapper: true,
            depth: 0,
            forward: Forward::Delayed,
            forward_delay_ms: FORWARD_DELAY_MS,
            exit_mode: ExitMode::AfterChild,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    assert_clean_teardown(&outcome, "forwarding wrapper waits for slow cleanup");
    assert_interrupted(&outcome, "forwarding wrapper waits for slow cleanup");
    assert_at_least_one_interrupt(&outcome, "forwarding wrapper waits for slow cleanup");
    assert_final_cleanup_captured(&outcome, "forwarding wrapper waits for slow cleanup");
}

/// A forwarding wrapper may exit while the worker is still cleaning up.
///
/// Whether the wrapper exits early is an observation about the current
/// target-selection heuristics, not a shutdown invariant. The final cleanup
/// output is required whenever the session leader keeps the terminal usable:
/// on pipes, and on PTY when the wrapper did not exit early. When a PTY
/// session leader does exit early the terminal hangs up, so late output is not
/// guaranteed and only process-tree completion is asserted.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_shutdown_forwarding_wrapper_exits_before_worker_cleanup(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "forwarding-wrapper-exits-early",
        FixtureSpec {
            wrapper: true,
            depth: 0,
            forward: Forward::Delayed,
            forward_delay_ms: FORWARD_DELAY_MS,
            exit_mode: ExitMode::Now,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    let scenario = "forwarding wrapper exits before cleanup";
    assert_clean_teardown(&outcome, scenario);
    assert_interrupted(&outcome, scenario);
    assert_at_least_one_interrupt(&outcome, scenario);

    if !use_pty || !outcome.early_exit_observed() {
        assert_final_cleanup_captured(&outcome, scenario);
    }
}

/// Nested forwarding wrappers: with two descendants Turborepo targets the
/// process group on both paths, and each wrapper forwards on top of it, so the
/// leaf worker can observe the interruption several times. Counts are
/// observations.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_shutdown_nested_forwarding_wrappers(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "nested-forwarding-wrappers",
        FixtureSpec {
            wrapper: true,
            depth: 1,
            forward: Forward::Delayed,
            forward_delay_ms: FORWARD_DELAY_MS,
            exit_mode: ExitMode::AfterChild,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    assert_clean_teardown(&outcome, "nested forwarding wrappers");
    assert_interrupted(&outcome, "nested forwarding wrappers");
    assert_at_least_one_interrupt(&outcome, "nested forwarding wrappers");
    assert_final_cleanup_captured(&outcome, "nested forwarding wrappers");
}

/// Nested forwarding wrappers where the root exits before worker cleanup.
/// On PTY the extra nesting forces the group target, so the root is really
/// interrupted and its early exit is exercised. That exit hangs up the
/// terminal, so this is a process-tree completion test, not a capture test.
/// On pipe the terminal is not hung up and cleanup output remains in scope.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_shutdown_nested_forwarding_wrapper_exits_before_worker_cleanup(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "nested-forwarding-wrapper-exits-early",
        FixtureSpec {
            wrapper: true,
            depth: 1,
            forward: Forward::Delayed,
            forward_delay_ms: FORWARD_DELAY_MS,
            exit_mode: ExitMode::Now,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    let scenario = "nested forwarding wrapper exits before cleanup";
    assert_clean_teardown(&outcome, scenario);
    assert_interrupted(&outcome, scenario);
    assert_at_least_one_interrupt(&outcome, scenario);
    assert!(
        outcome.early_exit_observed(),
        "root wrapper was expected to exit before worker cleanup{}",
        outcome.diagnostic(scenario)
    );

    if !use_pty {
        assert_final_cleanup_captured(&outcome, scenario);
    }
}

/// Focused regression for OUR duplicate send. Nested nonforwarding wrappers
/// under PTY receive the group SIGINT, then the root exits and
/// `send_interrupt_to_remaining_descendants` signals the still-cleaning worker
/// again. Because no wrapper forwards, the only way the worker sees a second
/// SIGINT is our fallback. The root waits on the worker's file acknowledgment
/// before exiting, so the second delivery cannot be coalesced with the first.
/// Expected to stay red until the fallback stops re-signaling.
#[tokio::test]
#[traced_test]
async fn test_nonforwarding_wrappers_resignal_worker_under_pty() {
    let outcome = run_fixture(
        true,
        "nonforwarding-pty-resignal",
        FixtureSpec {
            wrapper: true,
            depth: 1,
            forward: Forward::None,
            forward_delay_ms: 0,
            exit_mode: ExitMode::AfterAck,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    let scenario = "nonforwarding wrappers re-signal under PTY";
    assert_clean_teardown(&outcome, scenario);
    assert_interrupted(&outcome, scenario);
    assert!(
        outcome.early_exit_observed(),
        "nonforwarding root was expected to exit after acknowledging the worker{}",
        outcome.diagnostic(scenario)
    );
    assert_exactly_one_interrupt(&outcome, scenario);
}

/// Passing pipe counterpart of the regression above. The process-group target
/// makes `send_interrupt_to_remaining_descendants` a no-op, so the worker sees
/// the group signal exactly once.
#[tokio::test]
#[traced_test]
async fn test_nonforwarding_wrappers_pipe_counterpart() {
    let outcome = run_fixture(
        false,
        "nonforwarding-pipe-counterpart",
        FixtureSpec {
            wrapper: true,
            depth: 1,
            forward: Forward::None,
            forward_delay_ms: 0,
            exit_mode: ExitMode::AfterAck,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::Stop,
    )
    .await;

    let scenario = "nonforwarding wrappers pipe counterpart";
    assert_clean_teardown(&outcome, scenario);
    assert_interrupted(&outcome, scenario);
    assert_exactly_one_interrupt(&outcome, scenario);
    assert_final_cleanup_captured(&outcome, scenario);
}

/// Repeated internal graceful shutdown requests while the worker cleans up must
/// be idempotent at the Child layer: they must not deliver a second OS
/// interrupt. (Repeated *OS* signals are a different escalation policy and are
/// not what this exercises.) The worker acknowledges the first interrupt before
/// the extra requests are issued, and cleanup is slow enough to overlap them.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_repeated_graceful_shutdown_requests_are_idempotent(use_pty: bool) {
    let outcome = run_fixture(
        use_pty,
        "repeated-graceful-shutdown",
        FixtureSpec {
            wrapper: false,
            depth: 0,
            forward: Forward::None,
            forward_delay_ms: 0,
            exit_mode: ExitMode::AfterChild,
            worker_cleanup_ms: SLOW_CLEANUP_MS,
        },
        Driver::RepeatedGraceful(4),
    )
    .await;

    let scenario = "repeated graceful shutdown requests";
    assert_clean_teardown(&outcome, scenario);
    assert_interrupted(&outcome, scenario);
    assert_exactly_one_interrupt(&outcome, scenario);
    assert_final_cleanup_captured(&outcome, scenario);
}

fn descendant(
    pid: libc::pid_t,
    process_group_id: Option<libc::pid_t>,
    initially_signaled: bool,
) -> GracefulDescendant {
    GracefulDescendant {
        pid,
        process_group_id,
        initially_signaled,
    }
}

/// Group delivery must only mark the members of the process group that
/// actually accepted the signal, so descendants outside the signaled group
/// (for example a PTY descendant that moved itself) stay eligible for the
/// direct fallback.
#[test]
fn group_delivery_marks_only_signaled_group_members() {
    let mut descendants = vec![
        descendant(101, Some(50), false),
        descendant(102, Some(50), false),
        descendant(103, Some(60), false),
        descendant(104, None, false),
    ];

    mark_group_targets(&mut descendants, 50);

    assert!(descendants[0].initially_signaled);
    assert!(descendants[1].initially_signaled);
    assert!(!descendants[2].initially_signaled);
    assert!(!descendants[3].initially_signaled);
}

/// A failed group delivery never reaches `mark_group_targets`, so every
/// surviving descendant remains available for the fallback to retry directly.
#[test]
fn failed_group_delivery_leaves_all_descendants_untargeted() {
    let descendants = vec![
        descendant(201, Some(70), false),
        descendant(202, None, false),
    ];

    let fallback = descendants_needing_fallback(&descendants, |_| true);

    assert_eq!(fallback, vec![201, 202]);
}

/// The fallback must skip descendants the initial interrupt already reached
/// while preserving an initially untargeted survivor.
#[test]
fn fallback_skips_targeted_descendants_and_preserves_untargeted() {
    let descendants = vec![
        descendant(301, Some(80), true),
        descendant(302, Some(81), false),
        descendant(303, Some(80), true),
    ];

    let fallback = descendants_needing_fallback(&descendants, |_| true);

    assert_eq!(fallback, vec![302]);
}

/// Direct-target delivery marks each descendant only when its own kill
/// syscall was accepted; a sibling captured in the same snapshot stays
/// eligible for the fallback.
#[test]
fn direct_target_delivery_marks_only_accepted_pids() {
    let mut descendants = vec![
        descendant(401, Some(90), false),
        descendant(402, Some(90), false),
    ];

    descendants[0].record_direct_delivery(true);
    descendants[1].record_direct_delivery(false);

    assert!(descendants[0].initially_signaled);
    assert!(!descendants[1].initially_signaled);

    let fallback = descendants_needing_fallback(&descendants, |_| true);
    assert_eq!(fallback, vec![402]);
}

/// Dead descendants are filtered before the fallback signals anything.
#[test]
fn fallback_filters_dead_descendants() {
    let descendants = vec![
        descendant(501, Some(95), false),
        descendant(502, Some(95), false),
    ];

    let fallback = descendants_needing_fallback(&descendants, |pid| pid != 501);

    assert_eq!(fallback, vec![502]);
}
