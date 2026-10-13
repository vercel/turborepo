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
fn wrapper_leaves_the_console_to_exec_commands() {
    use std::{io::Write, process::Stdio};

    // On Windows, a wrapper that owns Ctrl+C also reads stdin in raw mode, so
    // the exec'd command would lose its console. Evaluate the shipped predicate.
    let source = fs::read_to_string(common::manifest_dir().join("../../packages/turbo/bin/turbo"))
        .unwrap()
        .replace("\r\n", "\n");
    let body = source
        .strip_suffix("runTurbo();\n")
        .expect("the wrapper starts turbo last");
    let body = body.strip_prefix("#!/usr/bin/env node\n").unwrap_or(body);
    let cases = [
        (json!(["exec", "--", "node"]), true),
        (json!(["--cwd", "apps/web", "exec", "--", "node"]), true),
        (json!(["-vv", "--verbosity", "2", "exec", "--", "x"]), true),
        (
            json!(["--experimental-otel-enabled", "true", "exec", "--", "x"]),
            true,
        ),
        (
            json!([
                "--experimental-otel-enabled",
                "--cwd",
                "a",
                "exec",
                "--",
                "x"
            ]),
            true,
        ),
        (
            json!(["--experimental-otel-enabled", "exec", "--", "x"]),
            false,
        ),
        (json!(["--token", "exec", "run", "build"]), false),
        (json!(["run", "exec"]), false),
        (json!(["build", "exec"]), false),
        (json!(["run", "build", "--", "exec"]), false),
        (json!(["--", "exec"]), false),
    ];
    let inputs: Vec<_> = cases.iter().map(|(args, _)| args).collect();
    let script = format!(
        "{body}\nprocess.stdout.write(JSON.stringify({}.map(selectsExec)));",
        json!(inputs)
    );
    let mut node = std::process::Command::new(which::which("node").unwrap())
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    node.stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let output = node.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", combined_output(&output));
    let selected: Vec<bool> = serde_json::from_slice(&output.stdout).unwrap();
    let expected: Vec<bool> = cases.iter().map(|(_, exec)| *exec).collect();
    assert_eq!(selected, expected);
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

/// The PIDs of a `--hold` task and its descendant, once both run.
#[cfg(unix)]
fn held(process: &mut CapturedProcess, cwd: &Path) -> [i32; 2] {
    let start = std::time::Instant::now();
    loop {
        if let Ok(pids) = fs::read_to_string(cwd.join("ready"))
            && let Ok(pids) = serde_json::from_str::<[i32; 2]>(&pids)
        {
            return pids;
        }
        assert!(start.elapsed() < TIMEOUT, "fixture did not become ready");
        assert!(
            process.child.try_wait().unwrap().is_none(),
            "launcher exited before task started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// TURBO-6424: global turbo hands off to the repository's local CLI by
/// becoming it, so no global turbo is left for SIGKILL to take down alone,
/// and the CLI's signal death is the entrypoint's own status.
#[cfg(unix)]
#[test]
fn global_turbo_becomes_the_local_cli_it_hands_off_to() {
    use std::{os::unix::process::ExitStatusExt, process::Command};

    use common::process::unix::TaskTreeGuard;
    use nix::{
        sys::signal::{self, Signal},
        unistd::Pid,
    };

    let launcher = ProcessLauncher::new();
    for entry in [Entrypoint::Standalone, Entrypoint::NpmWrapper] {
        let cwd = tempfile::tempdir().unwrap();
        repo(cwd.path());
        ProcessLauncher::install_package(cwd.path(), &launcher.standalone);
        let mut command = launcher.command(entry, cwd.path());
        command.args(["run", "observe", "--ui=stream", "-vv", "--", "--hold"]);
        let mut process = CapturedProcess::spawn(&mut command);
        let [task, _] = held(&mut process, cwd.path());
        // Tasks run in their own process group, which turbo's SIGKILL leaves
        // running whether or not it handed off.
        let _tasks = TaskTreeGuard::new(task);
        let entrypoint = process.child.id() as i32;
        let (mut pid, mut turbos) = (task, vec![]);
        loop {
            let ps = Command::new("ps")
                .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let ps = String::from_utf8(ps.stdout).unwrap();
            let (ppid, name) = ps.trim().split_once(' ').unwrap();
            if Path::new(name.trim())
                .file_name()
                .is_some_and(|name| name == "turbo")
            {
                turbos.push(pid);
            }
            if pid == entrypoint {
                break;
            }
            pid = ppid.parse().unwrap();
            assert!(pid > 1, "{entry:?}: the task is not below {entrypoint}");
        }
        assert_eq!(turbos.len(), 1, "{entry:?}: one turbo runs the task");
        signal::kill(Pid::from_raw(turbos[0]), Signal::SIGKILL).unwrap();
        let output = process.finish(TIMEOUT);
        assert_eq!(output.status.signal(), Some(9), "{entry:?}");
        let output = combined_output(&output);
        assert!(output.contains("Running local turbo binary in"), "{output}");
    }
}

#[cfg(unix)]
#[test]
fn launcher_captures_shutdown_and_guard_cleans_task_tree() {
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
            let [task, descendant] = held(&mut process, cwd.path());
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
