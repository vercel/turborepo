#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path, time::Duration};

use common::{
    combined_output,
    process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher},
};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(20);

fn repo(root: &Path) {
    fs::write(
        root.join("package.json"),
        json!({
            "name": "launcher-fixture", "private": true, "packageManager": "npm@10.5.0",
            "scripts": {
                "observe": "node observer.js",
                "probe": "turbo run observe --env-mode=loose --ui=stream"
            }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("turbo.json"),
        r#"{"tasks":{"observe":{"cache":false}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("observer.js"),
        r#"
const fs = require('fs');
console.log('OBSERVATION:' + JSON.stringify({
  argv: process.argv.slice(2), cwd: process.cwd(), context: process.env.HARNESS_CONTEXT,
  userAgent: process.env.npm_config_user_agent, lifecycle: process.env.npm_lifecycle_event
}));
console.error('OBSERVER_STDERR');
if (process.argv.includes('--noisy')) {
  process.stdout.write('o'.repeat(2 * 1024 * 1024));
  process.stderr.write('e'.repeat(2 * 1024 * 1024));
}
if (process.argv.includes('--hold')) {
  const child = require('child_process').spawn(process.execPath,
    ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'inherit' });
  const stop = () => { child.kill(); child.once('exit', () => process.exit(0)); };
  process.on('SIGINT', stop);
  process.on('SIGTERM', stop);
  setInterval(() => {}, 1000);
  // Panic-path backstop if startup fails before the task guard can register.
  setTimeout(() => { child.kill(); process.exit(99); }, 30000);
  fs.writeFileSync('ready', JSON.stringify([process.pid, child.pid]));
}
"#,
    )
    .unwrap();
    common::git(root, &["init", "--quiet"]);
}

#[test]
fn installed_wrapper_resolves_native_binary_and_repository_local_handoff() {
    let launcher = ProcessLauncher::new();
    let cwd = tempfile::tempdir().unwrap();
    repo(cwd.path());
    for (entry, expected) in [
        (Entrypoint::Standalone, &launcher.standalone),
        (Entrypoint::NpmWrapper, &launcher.packaged_binary),
    ] {
        let mut command = launcher.command(entry, cwd.path());
        command.args(["bin", "-vv"]);
        // These must be removed, not allowed to bypass optional-package resolution.
        // The real test process need not mutate its environment to check that.
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "TURBO_BINARY_PATH" && value.is_none())
        );
        let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
        assert!(output.status.success(), "{}", combined_output(&output));
        let actual = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            Path::new(actual.trim()).canonicalize().unwrap(),
            expected.canonicalize().unwrap()
        );
    }

    let (_, local) = ProcessLauncher::install_package(cwd.path(), &launcher.standalone);
    fs::create_dir(cwd.path().join("nested")).unwrap();
    for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
        let mut command = launcher.command(entry, &cwd.path().join("nested"));
        command.args(["bin", "-vv"]);
        let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
        assert!(output.status.success(), "{}", combined_output(&output));
        assert!(combined_output(&output).contains("Local turbo version:"));
        assert_eq!(
            Path::new(String::from_utf8(output.stdout).unwrap().trim())
                .canonicalize()
                .unwrap(),
            local.canonicalize().unwrap()
        );
    }
}

#[test]
fn captures_output_and_exit_from_both_entrypoints() {
    let launcher = ProcessLauncher::new();
    let cwd = tempfile::tempdir().unwrap();
    repo(cwd.path());
    for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
        let mut command = launcher.command(entry, cwd.path());
        command
            .args([
                "run",
                "observe",
                "--env-mode=loose",
                "--ui=stream",
                "--",
                "argument with spaces",
                "--literal",
            ])
            .env("HARNESS_CONTEXT", "launcher smoke")
            .env("npm_config_user_agent", "npm/10.5.0 node/v24.0.0")
            .env("npm_command", "run-script")
            .env("npm_lifecycle_event", "probe");
        let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
        assert!(output.status.success(), "{}", combined_output(&output));
        let combined = combined_output(&output);
        let observation: Value = serde_json::from_str(
            combined
                .lines()
                .find_map(|line| line.split_once("OBSERVATION:").map(|(_, json)| json))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            observation["argv"],
            json!(["argument with spaces", "--literal"])
        );
        assert_eq!(
            Path::new(observation["cwd"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            cwd.path().canonicalize().unwrap()
        );
        assert_eq!(observation["context"], "launcher smoke");
        assert!(combined.contains("OBSERVER_STDERR"));

        let mut invalid = launcher.command(entry, cwd.path());
        invalid.arg("--not-a-real-flag");
        let output = CapturedProcess::spawn(&mut invalid).finish(TIMEOUT);
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn drains_verbose_output_without_deadlocking_or_unbounded_capture() {
    let launcher = ProcessLauncher::new();
    let cwd = tempfile::tempdir().unwrap();
    repo(cwd.path());
    for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
        let mut command = launcher.command(entry, cwd.path());
        command.args(["run", "observe", "--ui=stream", "--", "--noisy"]);
        let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
        assert!(output.status.success(), "{}", combined_output(&output));
        assert_eq!(output.stdout.len(), 1024 * 1024);
        assert!(output.stderr.len() <= 1024 * 1024);
    }
}

#[cfg(unix)]
#[test]
fn actual_npm_script_uses_installed_bin_link_without_registry() {
    let launcher = ProcessLauncher::new();
    let cwd = tempfile::tempdir().unwrap();
    repo(cwd.path());
    ProcessLauncher::install_package(cwd.path(), &launcher.standalone);
    let mut command = launcher.command(Entrypoint::NpmScript("probe"), cwd.path());
    command.env("HARNESS_CONTEXT", "npm script");
    let output = CapturedProcess::spawn(&mut command).finish(TIMEOUT);
    assert!(output.status.success(), "{}", combined_output(&output));
    let combined = combined_output(&output);
    assert!(combined.contains("\"context\":\"npm script\""));
    assert!(combined.contains("\"userAgent\":\"npm/"));
    assert!(combined.contains("\"lifecycle\":\"observe\""));
    assert!(!combined.contains("attempt to install"));
}

#[cfg(unix)]
#[test]
fn launcher_captures_shutdown_and_guard_cleans_task_tree() {
    use std::{thread, time::Instant};

    use common::process::unix::wait_for_process_gone;
    use nix::{
        sys::signal::{self, Signal},
        unistd::Pid,
    };

    let launcher = ProcessLauncher::new();
    for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
        for teardown in ["signal", "drop", "timeout"] {
            let cwd = tempfile::tempdir().unwrap();
            repo(cwd.path());
            let mut command = launcher.command(entry, cwd.path());
            command.args(["run", "observe", "--ui=stream", "--", "--hold"]);
            let mut process = CapturedProcess::spawn(&mut command);
            let parent = process.child.id() as i32;
            let start = Instant::now();
            let [task, descendant] = loop {
                if let Ok(pids) = fs::read_to_string(cwd.path().join("ready"))
                    && let Ok(pids) = serde_json::from_str::<[i32; 2]>(&pids)
                {
                    break pids;
                }
                assert!(start.elapsed() < TIMEOUT, "fixture did not become ready");
                assert!(
                    process.child.try_wait().unwrap().is_none(),
                    "launcher exited before task started"
                );
                thread::sleep(Duration::from_millis(20));
            };
            process.track_task(task);
            process.track_task(descendant);
            if teardown == "signal" {
                signal::kill(Pid::from_raw(parent), Signal::SIGTERM).unwrap();
                let output = process.finish(TIMEOUT);
                assert!(
                    output.status.code().is_some() || {
                        use std::os::unix::process::ExitStatusExt;
                        output.status.signal().is_some()
                    }
                );
                assert!(combined_output(&output).contains("OBSERVATION:"));
            } else if teardown == "timeout" {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process.finish(Duration::from_millis(10))
                }));
                assert!(result.is_err(), "blocked fixture must hit the deadline");
            } else {
                drop(process);
            }
            wait_for_process_gone(parent, TIMEOUT);
            wait_for_process_gone(task, TIMEOUT);
            wait_for_process_gone(descendant, TIMEOUT);
        }
    }
}
