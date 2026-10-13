#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        io::{Read, Write},
        os::unix::process::CommandExt,
        path::{Path, PathBuf},
        process::{Child, Command, Output, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use nix::{
        sys::signal::{self, Signal},
        unistd::Pid,
    };
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::common::{
        self,
        process::{
            unix::{TaskTreeGuard, wait_for_process_gone},
            wait_for_process_exit,
        },
        setup,
    };

    const START_TIMEOUT: Duration = Duration::from_secs(15);
    const EXIT_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_PTY_OUTPUT_BYTES: usize = 256 * 1024;
    /// Upper bound on how long a fixture process may run before it kills its
    /// own process group. This is a hang backstop, not a pass condition: the
    /// graceful tests still finish only after the release file is written.
    const FIXTURE_WATCHDOG_SECS: u64 = 60;

    struct ChildGuard {
        child: Option<Child>,
    }

    impl ChildGuard {
        fn new(child: Child) -> Self {
            Self { child: Some(child) }
        }

        fn child_mut(&mut self) -> &mut Child {
            self.child.as_mut().expect("child guard consumed")
        }

        /// Wait for a task to write its readiness marker, reporting turbo's
        /// output if the run exits first or never gets there. Without this the
        /// only signal is a bare marker-file timeout, which hides startup
        /// failures such as an unusable package manager.
        fn wait_for_startup_marker(&mut self, path: &Path, timeout: Duration) {
            let start = Instant::now();
            loop {
                if path.exists() {
                    return;
                }
                let exited = self
                    .child_mut()
                    .try_wait()
                    .expect("failed waiting for child exit")
                    .is_some();
                if exited {
                    // The marker may have landed in the same tick turbo exited.
                    if path.exists() {
                        return;
                    }
                    panic!(
                        "turbo exited before {} was created\n{}",
                        path.display(),
                        self.collect_output()
                    );
                }
                if start.elapsed() > timeout {
                    panic!(
                        "timed out waiting for {}\n{}",
                        path.display(),
                        self.collect_output()
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
        }

        /// Stop the run if it is still alive and return everything it wrote.
        fn collect_output(&mut self) -> String {
            let Some(mut child) = self.child.take() else {
                return String::new();
            };
            let _ = child.kill();
            match child.wait_with_output() {
                Ok(output) => normalize_output(&format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )),
                Err(err) => format!("failed to read turbo output: {err}"),
            }
        }

        fn into_output(mut self, timeout: Duration) -> Output {
            let _ = wait_for_process_exit(self.child_mut(), timeout);
            self.child
                .take()
                .expect("child guard consumed")
                .wait_with_output()
                .expect("failed waiting for child output")
        }

        /// Assert turbo is still running. Used to prove the CLI waits for
        /// in-flight task cleanup instead of exiting as soon as it forwards a
        /// shutdown request.
        fn assert_still_running(&mut self) {
            let status = self
                .child_mut()
                .try_wait()
                .expect("failed waiting for child exit");
            assert!(
                status.is_none(),
                "turbo exited while task cleanup was still blocked: {status:?}"
            );
        }
    }

    /// Writes a blocked task's `release` file on drop so a failed assertion
    /// cannot leave the task tree waiting forever.
    struct ReleaseGuard {
        path: PathBuf,
    }

    impl ReleaseGuard {
        fn new(test_dir: &Path) -> Self {
            Self {
                path: test_dir.join("apps/app-a/release"),
            }
        }
    }

    impl Drop for ReleaseGuard {
        fn drop(&mut self) {
            let _ = fs::write(&self.path, b"");
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(child) = &mut self.child {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    struct PtyTurbo {
        child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
        writer: Option<Box<dyn Write + Send>>,
        output: Arc<Mutex<Vec<u8>>>,
        reader_thread: Option<thread::JoinHandle<()>>,
    }

    impl PtyTurbo {
        fn send_ctrl_c(&mut self) {
            let writer = self.writer.as_mut().expect("pty writer already taken");
            writer
                .write_all(&[3])
                .expect("failed to write Ctrl+C to pty");
            writer.flush().expect("failed to flush Ctrl+C to pty");
        }

        fn wait_for_output(&self, expected: &str, timeout: Duration) {
            let start = Instant::now();
            loop {
                let found =
                    String::from_utf8_lossy(&self.output.lock().unwrap()).contains(expected);
                if found {
                    return;
                }
                if start.elapsed() > timeout {
                    panic!("timed out waiting for output: {expected}");
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        /// Wait for a task to write its readiness marker, reporting the pty
        /// transcript if turbo exits first or never gets there.
        fn wait_for_startup_marker(&mut self, path: &Path, timeout: Duration) {
            let start = Instant::now();
            loop {
                if path.exists() {
                    return;
                }
                let exited = self
                    .child
                    .as_mut()
                    .expect("pty child guard consumed")
                    .try_wait()
                    .expect("failed waiting for pty child")
                    .is_some();
                if exited {
                    // The marker may have landed in the same tick turbo exited.
                    if path.exists() {
                        return;
                    }
                    panic!(
                        "turbo exited before {} was created\n{}",
                        path.display(),
                        self.transcript()
                    );
                }
                if start.elapsed() > timeout {
                    panic!(
                        "timed out waiting for {}\n{}",
                        path.display(),
                        self.transcript()
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
        }

        fn transcript(&self) -> String {
            normalize_output(String::from_utf8_lossy(&self.output.lock().unwrap()).as_ref())
        }

        fn finish(self, timeout: Duration) -> String {
            self.finish_with_status(timeout).1
        }

        fn finish_with_status(mut self, timeout: Duration) -> (portable_pty::ExitStatus, String) {
            let start = Instant::now();
            let status = loop {
                let status = self
                    .child
                    .as_mut()
                    .expect("pty child guard consumed")
                    .try_wait()
                    .expect("failed waiting for pty child");
                if let Some(status) = status {
                    break status;
                }
                if start.elapsed() > timeout {
                    panic!(
                        "timed out waiting for pty child exit\n{}",
                        self.transcript()
                    );
                }
                thread::sleep(Duration::from_millis(100));
            };

            let _ = self.child.take();
            drop(self.writer.take());
            if let Some(reader_thread) = self.reader_thread.take() {
                // An orphan holding the terminal open would block EOF forever.
                while !reader_thread.is_finished() {
                    assert!(
                        start.elapsed() < timeout,
                        "pty output did not close\n{}",
                        self.transcript()
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                reader_thread.join().expect("pty reader thread panicked");
            }

            (status, self.transcript())
        }
    }

    impl Drop for PtyTurbo {
        fn drop(&mut self) {
            drop(self.writer.take());
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Do not hang a failing test behind a descendant that kept the pty.
            if let Some(reader_thread) = self.reader_thread.take()
                && reader_thread.is_finished()
            {
                let _ = reader_thread.join();
            }
        }
    }

    fn append_capped_output(output: &Arc<Mutex<Vec<u8>>>, chunk: &[u8]) {
        let mut output = output.lock().unwrap();
        output.extend_from_slice(chunk);
        if output.len() > MAX_PTY_OUTPUT_BYTES {
            let excess = output.len() - MAX_PTY_OUTPUT_BYTES;
            output.drain(..excess);
        }
    }

    /// Stage the shutdown fixture with `script_name` wired up as `app-a`'s
    /// persistent `dev` task.
    ///
    /// This deliberately uses a dedicated integration fixture rather than one
    /// of the published examples: examples are version-bumped on their own
    /// cadence, and a `packageManager` pin that outruns the Node.js version
    /// this suite runs on takes every test here down with it. The fixture's pin
    /// is owned by the test suite, and `turborepo-tests/integration/**` is a
    /// declared input of the Rust test task, so changing it invalidates the
    /// cached task.
    fn setup_shutdown_fixture(script_name: &str, script_contents: &str) -> (TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().expect("failed to create tempdir");
        let test_dir = tempdir.path().to_path_buf();
        setup::copy_fixture("graceful_shutdown", &test_dir).expect("failed to copy fixture");
        setup::prepare_corepack_from_package_json(&test_dir);

        let app_dir = test_dir.join("apps/app-a");
        fs::write(app_dir.join(script_name), script_contents).expect("failed to write script");

        let package_json_path = app_dir.join("package.json");
        let mut package_json: Value = serde_json::from_str(
            &fs::read_to_string(&package_json_path).expect("failed to read app package.json"),
        )
        .expect("failed to parse app package.json");
        package_json["scripts"]["dev"] = Value::String(format!("bash ./{script_name}"));
        fs::write(
            &package_json_path,
            serde_json::to_string_pretty(&package_json).expect("failed to serialize app package"),
        )
        .expect("failed to update app package.json");

        let turbo_json_path = test_dir.join("turbo.json");
        let mut turbo_json: Value = serde_json::from_str(
            &fs::read_to_string(&turbo_json_path).expect("failed to read turbo.json"),
        )
        .expect("failed to parse turbo.json");
        turbo_json["tasks"]["dev"] = json!({
            "cache": false,
            "persistent": true,
        });
        fs::write(
            &turbo_json_path,
            serde_json::to_string_pretty(&turbo_json).expect("failed to serialize turbo.json"),
        )
        .expect("failed to update turbo.json");

        (tempdir, test_dir)
    }

    /// A task that only finishes its SIGINT cleanup once the test writes a
    /// `release` file. The task owns its background descendant: on shutdown it
    /// waits for the external `release`, signals the private
    /// `descendant.release`, and reaps the child before reporting cleanup done.
    /// The descendant ignores INT and TERM and only exits when released, so
    /// cleanup is coordinated by the parent rather than by inherited signals.
    /// Every loop carries a watchdog deadline and the parent advertises
    /// readiness only after the descendant has published `descendant.ready`.
    fn blocked_cleanup_script(label: &str) -> String {
        format!(
            r#"#!/usr/bin/env bash
set -u
trap 'printf "{label} cleanup start\n"; : > cleanup.started; deadline=$((SECONDS + {watchdog})); while [ ! -f release ]; do if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi; sleep 0.1; done; : > descendant.release; wait "$child"; printf "{label} cleanup done\n"; exit 0' INT
(
  trap '' INT TERM
  : > descendant.ready
  deadline=$((SECONDS + {watchdog}))
  while [ ! -f descendant.release ]; do
    if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
    sleep 0.2 || true
  done
) &
child=$!
printf '%s\n' "$child" > child.pid
deadline=$((SECONDS + {watchdog}))
while [ ! -f descendant.ready ]; do
  if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
  sleep 0.05 || true
done
printf "{label} ready child=%s\n" "$child"
: > ready
deadline=$((SECONDS + {watchdog}))
while true; do
  if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
  sleep 0.2 || true
done
"#,
            label = label,
            watchdog = FIXTURE_WATCHDOG_SECS,
        )
    }

    fn turbo_bin() -> PathBuf {
        assert_cmd::cargo::cargo_bin("turbo")
    }

    fn turbo_node_wrapper() -> PathBuf {
        common::manifest_dir().join("../../packages/turbo/bin/turbo")
    }

    fn spawn_noninteractive_turbo(test_dir: &Path) -> ChildGuard {
        let mut cmd = Command::new(turbo_bin());
        let corepack_dir = setup::corepack_dir_for_test_dir(test_dir);
        cmd.arg("run").arg("dev").arg("--filter=app-a");
        for key in common::ambient_turbo_env_keys() {
            cmd.env_remove(&key);
        }
        cmd.env("PATH", setup::prepend_to_path(&corepack_dir))
            .env("COREPACK_HOME", setup::corepack_home())
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .env_remove("CI")
            .env_remove("GITHUB_ACTIONS")
            .current_dir(test_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ChildGuard::new(cmd.spawn().expect("failed to spawn turbo"))
    }

    fn spawn_noninteractive_turbo_via_node_wrapper(test_dir: &Path) -> ChildGuard {
        let mut cmd = Command::new("node");
        let corepack_dir = setup::corepack_dir_for_test_dir(test_dir);
        cmd.arg(turbo_node_wrapper())
            .arg("run")
            .arg("dev")
            .arg("--filter=app-a");
        for key in common::ambient_turbo_env_keys() {
            cmd.env_remove(&key);
        }
        cmd.env("PATH", setup::prepend_to_path(&corepack_dir))
            .env("COREPACK_HOME", setup::corepack_home())
            .env("TURBO_BINARY_PATH", turbo_bin())
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .env_remove("CI")
            .env_remove("GITHUB_ACTIONS")
            .current_dir(test_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        ChildGuard::new(cmd.spawn().expect("failed to spawn turbo wrapper"))
    }

    fn spawn_interactive_turbo(test_dir: &Path) -> PtyTurbo {
        spawn_interactive_turbo_command(test_dir, false)
    }

    fn spawn_interactive_turbo_via_node_wrapper(test_dir: &Path) -> PtyTurbo {
        spawn_interactive_turbo_command(test_dir, true)
    }

    fn spawn_interactive_turbo_command(test_dir: &Path, via_node_wrapper: bool) -> PtyTurbo {
        let corepack_dir = setup::corepack_dir_for_test_dir(test_dir);
        let mut command = if via_node_wrapper {
            let mut command = CommandBuilder::new("node");
            command.arg(turbo_node_wrapper());
            command.env("TURBO_BINARY_PATH", turbo_bin());
            command
        } else {
            CommandBuilder::new(turbo_bin())
        };
        command.arg("run");
        command.arg("dev");
        command.arg("--filter=app-a");
        command.cwd(test_dir);
        for key in common::ambient_turbo_env_keys() {
            command.env_remove(&key);
        }
        command.env("PATH", setup::prepend_to_path(&corepack_dir));
        command.env("COREPACK_HOME", setup::corepack_home());
        command.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1");
        command.env("TURBO_GLOBAL_WARNING_DISABLED", "1");
        command.env("TURBO_PRINT_VERSION_DISABLED", "1");
        command.env("DO_NOT_TRACK", "1");
        command.env("NPM_CONFIG_UPDATE_NOTIFIER", "false");
        command.env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0");
        command.env_remove("CI");
        command.env_remove("GITHUB_ACTIONS");

        spawn_pty_command(command)
    }

    fn spawn_pty_command(command: CommandBuilder) -> PtyTurbo {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("failed to create pty pair");

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();
        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("failed to clone pty reader");
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0; 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => append_capped_output(&output_clone, &buffer[..n]),
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let writer = pair
            .master
            .take_writer()
            .expect("failed to take pty writer");

        let child = pair
            .slave
            .spawn_command(command)
            .expect("failed to spawn turbo in pty");

        PtyTurbo {
            child: Some(child),
            writer: Some(writer),
            output,
            reader_thread: Some(reader_thread),
        }
    }

    fn read_pid_file(path: &Path) -> Result<i32, String> {
        fs::read_to_string(path)
            .map_err(|err| err.to_string())?
            .trim()
            .parse::<i32>()
            .map_err(|err| err.to_string())
    }

    fn wait_for_pid_file(path: &Path, timeout: Duration) -> i32 {
        let start = Instant::now();
        loop {
            match read_pid_file(path) {
                Ok(pid) => return pid,
                Err(_) if start.elapsed() <= timeout => thread::sleep(Duration::from_millis(100)),
                Err(err) => panic!("timed out waiting for pid file {}: {err}", path.display()),
            }
        }
    }

    fn send_signal(pid: i32, signal_kind: Signal) {
        signal::kill(Pid::from_raw(pid), signal_kind).expect("failed to send signal");
    }

    fn send_signal_to_process_group(process_group_id: i32, signal_kind: Signal) {
        signal::kill(Pid::from_raw(-process_group_id), signal_kind)
            .expect("failed to send signal to process group");
    }

    fn normalize_output(text: &str) -> String {
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    #[test]
    fn run_finishes_successfully_without_shutdown_banner() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "fast-success.sh",
            r#"#!/usr/bin/env bash
set -eu
printf "fast success\n"
exit 0
"#,
        );

        let child = spawn_noninteractive_turbo(&test_dir);
        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            output.status.success(),
            "successful runs should exit 0\n{combined}"
        );
        assert!(
            !combined.contains("Shutting down Turborepo tasks..."),
            "normal completion should not emit shutdown UX\n{combined}"
        );
    }

    #[test]
    fn run_stops_with_task_when_parent_exits() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "finite-parent.sh",
            r#"#!/usr/bin/env bash
set -eu
deadline=$((SECONDS + 10))
while [ ! -f sidecar.ready ]; do
  if [ "$SECONDS" -ge "$deadline" ]; then exit 1; fi
  sleep 0.1
done
printf "parent done\n"
"#,
        );
        let app_dir = test_dir.join("apps/app-a");
        fs::write(
            app_dir.join("sidecar.sh"),
            r#"#!/usr/bin/env bash
set -u
trap 'printf "sidecar cleanup\n"; : > sidecar.cleanup; exit 0' INT
: > sidecar.ready
while true; do sleep 0.2 || true; done
"#,
        )
        .expect("failed to write sidecar script");

        let package_json_path = app_dir.join("package.json");
        let mut package_json: Value = serde_json::from_str(
            &fs::read_to_string(&package_json_path).expect("failed to read app package.json"),
        )
        .expect("failed to parse app package.json");
        package_json["scripts"]["sidecar"] = Value::String("bash ./sidecar.sh".to_string());
        fs::write(
            &package_json_path,
            serde_json::to_string_pretty(&package_json).expect("failed to serialize app package"),
        )
        .expect("failed to update app package.json");

        let turbo_json_path = test_dir.join("turbo.json");
        let mut turbo_json: Value = serde_json::from_str(
            &fs::read_to_string(&turbo_json_path).expect("failed to read turbo.json"),
        )
        .expect("failed to parse turbo.json");
        turbo_json["tasks"]["dev"] = json!({
            "cache": false,
            "with": ["sidecar"],
        });
        turbo_json["tasks"]["sidecar"] = json!({
            "cache": false,
            "persistent": true,
        });
        fs::write(
            &turbo_json_path,
            serde_json::to_string_pretty(&turbo_json).expect("failed to serialize turbo.json"),
        )
        .expect("failed to update turbo.json");

        let child = spawn_noninteractive_turbo(&test_dir);
        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(output.status.success(), "run should exit 0\n{combined}");
        assert!(
            app_dir.join("sidecar.cleanup").exists(),
            "with task should receive a graceful stop when its parent exits\n{combined}"
        );
        assert!(
            combined.contains("sidecar cleanup"),
            "expected sidecar cleanup output\n{combined}"
        );
    }

    #[test]
    fn run_gracefully_shuts_down_on_first_sigint_in_tty() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "graceful.sh",
            r#"#!/usr/bin/env bash
set -u
trap 'printf "graceful cleanup start\n"; sleep 0.5; : > cleanup.done; printf "graceful cleanup done\n"; exit 0' INT
printf "graceful ready\n"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let cleanup_file = test_dir.join("apps/app-a/cleanup.done");

        let mut child = spawn_interactive_turbo(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);

        child.send_ctrl_c();
        let transcript = child.finish(EXIT_TIMEOUT);

        assert!(
            cleanup_file.exists(),
            "graceful cleanup marker should exist"
        );
        let cleanup_idx = transcript
            .find("graceful cleanup start")
            .expect("expected child cleanup log after signal");
        let cleanup_done_idx = transcript
            .find("graceful cleanup done")
            .expect("expected delayed child cleanup completion log");
        assert!(
            cleanup_idx < cleanup_done_idx,
            "cleanup completion log should appear after cleanup starts\n{transcript}"
        );
        assert!(
            !transcript.contains("^C^C"),
            "single Ctrl+C should only display one terminal interrupt marker\n{transcript}"
        );
        assert!(
            transcript.contains(
                " - Shutting down Turborepo tasks...Press CTRL+C again to exit forcefully."
            ),
            "interactive shutdown banner should follow the terminal Ctrl+C marker\n{transcript}"
        );
    }

    #[test]
    fn run_gracefully_shuts_down_on_first_sigint_without_tty_and_exits_zero() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "graceful.sh",
            r#"#!/usr/bin/env bash
set -u
trap 'printf "graceful cleanup start\n"; sleep 0.5; : > cleanup.done; printf "graceful cleanup done\n"; exit 0' INT
printf "graceful ready\n"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let cleanup_file = test_dir.join("apps/app-a/cleanup.done");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);

        send_signal(child.child_mut().id() as i32, Signal::SIGINT);
        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            output.status.success(),
            "graceful shutdown on SIGINT should exit 0\n{combined}"
        );
        assert!(
            cleanup_file.exists(),
            "graceful cleanup marker should exist"
        );
        assert!(
            combined.contains("graceful cleanup done"),
            "expected cleanup completion log after signal\n{combined}"
        );
    }

    #[test]
    fn second_ctrl_c_force_kills_with_task_through_node_wrapper() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "dev-sigint.sh",
            r#"#!/usr/bin/env bash
set -u
trap 'exit 0' INT
: > dev.ready
while true; do sleep 0.2 || true; done
"#,
        );

        let app_dir = test_dir.join("apps/app-a");
        fs::write(
            app_dir.join("sidecar-sigint.sh"),
            r#"#!/usr/bin/env bash
set -u
trap '' INT TERM
sh -c 'trap "" INT TERM; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > sidecar-child.pid
: > sidecar.ready
while true; do sleep 0.2 || true; done
"#,
        )
        .expect("failed to write sidecar script");

        let package_json_path = app_dir.join("package.json");
        let mut package_json: Value = serde_json::from_str(
            &fs::read_to_string(&package_json_path).expect("failed to read app package.json"),
        )
        .expect("failed to parse app package.json");
        package_json["scripts"]["sidecar"] = Value::String("bash ./sidecar-sigint.sh".to_string());
        fs::write(
            &package_json_path,
            serde_json::to_string_pretty(&package_json).expect("failed to serialize app package"),
        )
        .expect("failed to update app package.json");

        let turbo_json_path = test_dir.join("turbo.json");
        let mut turbo_json: Value = serde_json::from_str(
            &fs::read_to_string(&turbo_json_path).expect("failed to read turbo.json"),
        )
        .expect("failed to parse turbo.json");
        turbo_json["tasks"]["dev"]["with"] = json!(["sidecar"]);
        turbo_json["tasks"]["sidecar"] = json!({
            "cache": false,
            "persistent": true,
        });
        fs::write(
            &turbo_json_path,
            serde_json::to_string_pretty(&turbo_json).expect("failed to serialize turbo.json"),
        )
        .expect("failed to update turbo.json");

        let dev_ready_file = app_dir.join("dev.ready");
        let sidecar_ready_file = app_dir.join("sidecar.ready");
        let sidecar_child_pid_file = app_dir.join("sidecar-child.pid");

        let mut child = spawn_interactive_turbo_via_node_wrapper(&test_dir);
        child.wait_for_startup_marker(&dev_ready_file, START_TIMEOUT);
        child.wait_for_startup_marker(&sidecar_ready_file, START_TIMEOUT);
        let sidecar_child_pid = wait_for_pid_file(&sidecar_child_pid_file, START_TIMEOUT);

        child.send_ctrl_c();
        child.wait_for_output("Shutting down Turborepo tasks", Duration::from_secs(5));
        child.send_ctrl_c();

        let transcript = child.finish(Duration::from_secs(5));
        wait_for_process_gone(sidecar_child_pid, Duration::from_secs(5));

        assert!(
            transcript.contains("Force killed Turborepo tasks:")
                && transcript.contains("app-a#sidecar"),
            "expected second Ctrl+C to force kill the `with` task\n{transcript}"
        );
    }

    #[test]
    fn node_wrapper_waits_for_graceful_shutdown_on_sigint() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "slow-graceful.sh",
            r#"#!/usr/bin/env bash
set -u
trap 'printf "wrapper cleanup start\n"; sleep 2; : > cleanup.done; printf "wrapper cleanup done\n"; exit 0' INT
printf "wrapper ready\n"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let cleanup_file = test_dir.join("apps/app-a/cleanup.done");

        let mut child = spawn_noninteractive_turbo_via_node_wrapper(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);

        let wrapper_pid = child.child_mut().id() as i32;
        send_signal_to_process_group(wrapper_pid, Signal::SIGINT);
        thread::sleep(Duration::from_secs(1));

        let exited_early = child
            .child_mut()
            .try_wait()
            .expect("failed waiting for node wrapper")
            .is_some();

        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            !exited_early,
            "node wrapper exited before turbo finished graceful shutdown\n{combined}"
        );
        assert!(
            output.status.success(),
            "graceful shutdown via node wrapper should exit 0\n{combined}"
        );
        assert!(
            cleanup_file.exists(),
            "graceful cleanup marker should exist"
        );
        assert!(
            combined.contains("wrapper cleanup done"),
            "expected cleanup completion log after signal\n{combined}"
        );
    }

    #[test]
    fn node_wrapper_forwards_sigterm_to_turbo() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "slow-sigterm.sh",
            r#"#!/usr/bin/env bash
set -u
trap 'printf "wrapper sigterm cleanup start\n"; sleep 2; : > cleanup.done; printf "wrapper sigterm cleanup done\n"; exit 0' INT
printf "wrapper term ready\n"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let cleanup_file = test_dir.join("apps/app-a/cleanup.done");

        let mut child = spawn_noninteractive_turbo_via_node_wrapper(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);

        let wrapper_pid = child.child_mut().id() as i32;
        send_signal(wrapper_pid, Signal::SIGTERM);
        thread::sleep(Duration::from_secs(1));

        let exited_early = child
            .child_mut()
            .try_wait()
            .expect("failed waiting for node wrapper")
            .is_some();

        if exited_early {
            let _ = signal::kill(Pid::from_raw(-wrapper_pid), Signal::SIGKILL);
        }

        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            !exited_early,
            "node wrapper exited before turbo finished graceful shutdown on SIGTERM\n{combined}"
        );
        assert!(
            output.status.success(),
            "graceful shutdown via node wrapper should exit 0 on SIGTERM\n{combined}"
        );
        assert!(
            cleanup_file.exists(),
            "graceful cleanup marker should exist"
        );
        assert!(
            combined.contains("wrapper sigterm cleanup done"),
            "expected cleanup completion log after signal\n{combined}"
        );
    }

    #[test]
    fn run_force_kills_on_second_sigint_in_tty() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "stubborn.sh",
            r#"#!/usr/bin/env bash
set -u
trap '' INT
sh -c 'trap "" INT TERM; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > child.pid
printf "stubborn ready child=%s\n" "$child"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let child_pid_file = test_dir.join("apps/app-a/child.pid");

        let mut child = spawn_interactive_turbo(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);

        child.send_ctrl_c();
        thread::sleep(Duration::from_millis(2500));
        child.send_ctrl_c();

        let transcript = child.finish(Duration::from_secs(5));
        wait_for_process_gone(task_child_pid, Duration::from_secs(5));

        assert!(
            transcript.contains("stubborn ready child="),
            "expected task output to remain visible in the TUI transcript\n{transcript}"
        );
        assert!(
            transcript.contains(
                " - Shutting down Turborepo tasks...Press CTRL+C again to exit forcefully."
            ),
            "expected interactive shutdown banner to include force-exit prompt\n{transcript}"
        );
        assert!(
            transcript
                .matches("Press CTRL+C again to exit forcefully.")
                .count()
                == 1,
            "force-exit prompt should only appear on the initial shutdown banner\n{transcript}"
        );
        assert!(
            transcript.contains("1 task shutting down..."),
            "expected repeated shutdown status after the initial banner\n{transcript}"
        );
        assert!(
            transcript.contains(" - Force killed Turborepo tasks: app-a#dev"),
            "expected force-kill status after second Ctrl+C\n{transcript}"
        );
        assert!(
            !transcript.contains(" WARNING "),
            "force-kill status should not use warning styling\n{transcript}"
        );
    }

    #[test]
    fn run_force_kills_after_timeout_without_tty() {
        let (_tempdir, test_dir) = setup_shutdown_fixture(
            "stubborn.sh",
            r#"#!/usr/bin/env bash
set -u
trap '' INT
sh -c 'trap "" INT TERM; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > child.pid
printf "stubborn ready child=%s\n" "$child"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let child_pid_file = test_dir.join("apps/app-a/child.pid");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);

        let started = Instant::now();
        send_signal(child.child_mut().id() as i32, Signal::SIGINT);
        let output = child.into_output(Duration::from_secs(20));

        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_secs(9),
            "non-interactive shutdown should wait for the timeout before force killing, got \
             {elapsed:?}\n{combined}"
        );
        wait_for_process_gone(task_child_pid, Duration::from_secs(5));
        assert!(
            combined.contains("Shutting down Turborepo tasks..."),
            "expected non-interactive shutdown banner\n{combined}"
        );
        assert!(
            combined.matches("1 task shutting down...").count() >= 2,
            "expected repeated shutdown status while waiting for force timeout\n{combined}"
        );
        assert!(
            !combined.contains("Press CTRL+C again to exit forcefully."),
            "non-interactive timeout shutdown should not emit force-exit prompt\n{combined}"
        );
        assert!(
            !combined.contains("Some tasks in your Turborepo are taking awhile to shut down"),
            "old warning should not be emitted\n{combined}"
        );
        assert!(
            combined
                .contains("Graceful shutdown timed out. Force killed Turborepo tasks: app-a#dev"),
            "expected auto-force banner after timeout\n{combined}"
        );
    }

    #[test]
    fn native_turbo_sigterm_waits_for_graceful_task_cleanup() {
        let (_tempdir, test_dir) =
            setup_shutdown_fixture("sigterm-graceful.sh", &blocked_cleanup_script("sigterm"));
        let app_dir = test_dir.join("apps/app-a");
        let ready_file = app_dir.join("ready");
        let cleanup_started_file = app_dir.join("cleanup.started");
        let child_pid_file = app_dir.join("child.pid");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        let release = ReleaseGuard::new(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);
        let mut task_guard = TaskTreeGuard::new(task_child_pid);

        // SIGTERM goes to the native turbo PID. The supervisor turns this into
        // a graceful SIGINT request to the task; this test asserts boundary
        // behavior, not byte-for-byte forwarding of the incoming signal.
        send_signal(child.child_mut().id() as i32, Signal::SIGTERM);

        // Reaching the task trap means turbo accepted the signal and asked the
        // task to shut down.
        child.wait_for_startup_marker(&cleanup_started_file, START_TIMEOUT);
        child.assert_still_running();

        drop(release);

        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            output.status.success(),
            "graceful shutdown on SIGTERM should exit 0\n{combined}"
        );
        let cleanup_start = combined
            .find("sigterm cleanup start")
            .expect("expected task cleanup to start after SIGTERM");
        let cleanup_done = combined
            .find("sigterm cleanup done")
            .expect("expected task cleanup to finish before turbo exits");
        assert!(
            cleanup_start < cleanup_done,
            "cleanup completion should be emitted before turbo exits\n{combined}"
        );
        assert!(
            !combined.contains("Force killed Turborepo tasks")
                && !combined.contains("Graceful shutdown timed out")
                && !combined.contains("Press CTRL+C again to exit forcefully."),
            "graceful SIGTERM shutdown should not emit force-shutdown UX\n{combined}"
        );

        wait_for_process_gone(task_child_pid, Duration::from_secs(5));
        task_guard.disarm();
    }

    /// SIGHUP reaches turbo as a process-directed signal while its output pipes
    /// stay open. This does not model a terminal physically disappearing: the
    /// supervisor's stdout/stderr are redirected pipes, and a disappearing
    /// terminal would not close them. It only exercises the supervisor's hangup
    /// listener.
    #[test]
    fn native_turbo_sighup_waits_for_graceful_task_cleanup() {
        let (_tempdir, test_dir) =
            setup_shutdown_fixture("sighup-graceful.sh", &blocked_cleanup_script("sighup"));
        let app_dir = test_dir.join("apps/app-a");
        let ready_file = app_dir.join("ready");
        let cleanup_started_file = app_dir.join("cleanup.started");
        let child_pid_file = app_dir.join("child.pid");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        let release = ReleaseGuard::new(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);
        let mut task_guard = TaskTreeGuard::new(task_child_pid);

        send_signal(child.child_mut().id() as i32, Signal::SIGHUP);

        child.wait_for_startup_marker(&cleanup_started_file, START_TIMEOUT);
        child.assert_still_running();

        drop(release);

        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            output.status.success(),
            "graceful shutdown on SIGHUP should exit 0\n{combined}"
        );
        let cleanup_start = combined
            .find("sighup cleanup start")
            .expect("expected task cleanup to start after SIGHUP");
        let cleanup_done = combined
            .find("sighup cleanup done")
            .expect("expected task cleanup to finish before turbo exits");
        assert!(
            cleanup_start < cleanup_done,
            "cleanup completion should be emitted before turbo exits\n{combined}"
        );
        assert!(
            !combined.contains("Force killed Turborepo tasks")
                && !combined.contains("Graceful shutdown timed out")
                && !combined.contains("Press CTRL+C again to exit forcefully."),
            "graceful SIGHUP shutdown should not emit force-shutdown UX\n{combined}"
        );

        wait_for_process_gone(task_child_pid, Duration::from_secs(5));
        task_guard.disarm();
    }

    #[test]
    fn native_turbo_second_sigint_after_acknowledged_shutdown_force_kills_stubborn_task() {
        let stubborn_script = format!(
            r#"#!/usr/bin/env bash
set -u
trap 'printf "second sigint shutdown requested\n"; : > cleanup.started' INT
(
  trap '' INT TERM
  : > descendant.ready
  deadline=$((SECONDS + {watchdog}))
  while true; do
    if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
    sleep 0.2 || true
  done
) &
child=$!
printf '%s\n' "$child" > child.pid
deadline=$((SECONDS + {watchdog}))
while [ ! -f descendant.ready ]; do
  if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
  sleep 0.05 || true
done
printf "second sigint ready child=%s\n" "$child"
: > ready
deadline=$((SECONDS + {watchdog}))
while true; do
  if [ "$SECONDS" -ge "$deadline" ]; then kill -KILL 0; fi
  sleep 0.2 || true
done
"#,
            watchdog = FIXTURE_WATCHDOG_SECS,
        );
        let (_tempdir, test_dir) = setup_shutdown_fixture("second-sigint.sh", &stubborn_script);
        let app_dir = test_dir.join("apps/app-a");
        let ready_file = app_dir.join("ready");
        let cleanup_started_file = app_dir.join("cleanup.started");
        let child_pid_file = app_dir.join("child.pid");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        child.wait_for_startup_marker(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);
        let mut task_guard = TaskTreeGuard::new(task_child_pid);

        let turbo_pid = child.child_mut().id() as i32;
        send_signal(turbo_pid, Signal::SIGINT);

        // The stubborn task acknowledges the graceful request but never exits,
        // so turbo must still be waiting before a second signal arrives. Two OS
        // signals here are two real signals, not assumed to be two key presses.
        child.wait_for_startup_marker(&cleanup_started_file, START_TIMEOUT);
        child.assert_still_running();

        send_signal(turbo_pid, Signal::SIGINT);
        let output = child.into_output(EXIT_TIMEOUT);
        let combined = normalize_output(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));

        assert!(
            combined.contains("second sigint ready child="),
            "task output should remain visible until it is force killed\n{combined}"
        );
        assert!(
            combined.contains("Shutting down Turborepo tasks..."),
            "expected non-interactive shutdown banner before force kill\n{combined}"
        );
        assert!(
            combined.contains("Force killed Turborepo tasks: app-a#dev"),
            "the second SIGINT should force kill the stubborn task\n{combined}"
        );
        assert!(
            !combined.contains("Graceful shutdown timed out"),
            "force kill should come from the second signal, not the timeout\n{combined}"
        );

        wait_for_process_gone(task_child_pid, Duration::from_secs(5));
        task_guard.disarm();
    }

    /// Direct native `turbo run` versus the installed npm package, whose
    /// launcher resolves its platform binary like a real install rather than
    /// the source wrapper plus `TURBO_BINARY_PATH` used above.
    mod packaged_parity {
        use std::os::unix::process::ExitStatusExt;

        use super::*;
        use crate::common::{
            combined_output,
            process_launcher::{CapturedProcess, Entrypoint, ProcessLauncher},
        };

        const ENTRIES: [Entrypoint; 2] = [Entrypoint::Standalone, Entrypoint::NpmWrapper];

        #[derive(Clone, Copy, Debug, PartialEq)]
        enum Terminal {
            Pipes,
            Pty,
        }

        #[derive(Clone, Copy, Debug)]
        enum Stop {
            /// A Ctrl+C keypress, or without a terminal SIGINT to the
            /// foreground process group: both reach the wrapper and native
            /// turbo.
            CtrlC,
            /// SIGTERM to the entrypoint PID only, as a process manager sends
            /// it.
            Term,
        }

        enum Run {
            Pipes(Option<CapturedProcess>),
            Pty {
                // Declared first so a failure kills the group before the pty.
                group: TaskTreeGuard,
                child: PtyTurbo,
            },
        }

        struct Outcome {
            code: i32,
            output: String,
            /// stdout and stderr. A pty merges both streams by design.
            streams: [Vec<String>; 2],
        }

        fn pty_pid(child: &PtyTurbo) -> i32 {
            child.child.as_ref().unwrap().process_id().unwrap() as i32
        }

        impl Run {
            fn spawn(mut command: Command, terminal: Terminal) -> Self {
                if terminal == Terminal::Pipes {
                    return Self::Pipes(Some(CapturedProcess::spawn(&mut command)));
                }
                let mut builder = CommandBuilder::new(command.get_program());
                builder.args(command.get_args());
                builder.cwd(command.get_current_dir().unwrap());
                for (key, value) in command.get_envs() {
                    match value {
                        Some(value) => builder.env(key, value),
                        None => builder.env_remove(key),
                    }
                }
                let child = spawn_pty_command(builder);
                Self::Pty {
                    group: TaskTreeGuard::new(pty_pid(&child)),
                    child,
                }
            }

            fn pid(&self) -> i32 {
                match self {
                    Self::Pipes(process) => process.as_ref().unwrap().child.id() as i32,
                    Self::Pty { child, .. } => pty_pid(child),
                }
            }

            fn fail(&mut self, message: &str) -> ! {
                let output = match self {
                    Self::Pipes(process) => {
                        let process = process.take().unwrap();
                        let _ = signal::kill(
                            Pid::from_raw(-(process.child.id() as i32)),
                            Signal::SIGKILL,
                        );
                        combined_output(&process.finish(EXIT_TIMEOUT))
                    }
                    Self::Pty { child, .. } => child.transcript(),
                };
                panic!("{message}\n{output}");
            }

            fn exited(&mut self) -> bool {
                match self {
                    Self::Pipes(process) => process
                        .as_mut()
                        .unwrap()
                        .child
                        .try_wait()
                        .unwrap()
                        .is_some(),
                    Self::Pty { child, .. } => {
                        child.child.as_mut().unwrap().try_wait().unwrap().is_some()
                    }
                }
            }

            fn assert_running(&mut self, context: &str) {
                if self.exited() {
                    self.fail(&format!("entrypoint exited {context}"));
                }
            }

            fn wait_for(&mut self, marker: &Path) {
                let start = Instant::now();
                while !marker.exists() {
                    self.assert_running(&format!("before {}", marker.display()));
                    if start.elapsed() > START_TIMEOUT {
                        self.fail(&format!("timed out waiting for {}", marker.display()));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }

            fn stop(&mut self, stop: Stop) {
                match (stop, self) {
                    (Stop::Term, run) => send_signal(run.pid(), Signal::SIGTERM),
                    (Stop::CtrlC, Self::Pty { child, .. }) => child.send_ctrl_c(),
                    (Stop::CtrlC, run) => send_signal_to_process_group(run.pid(), Signal::SIGINT),
                }
            }

            fn finish(mut self, timeout: Duration) -> Outcome {
                let start = Instant::now();
                // The pty path reports its own timeout with the transcript.
                while matches!(self, Self::Pipes(_)) && !self.exited() {
                    if start.elapsed() > timeout {
                        self.fail(&format!("entrypoint did not exit within {timeout:?}"));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let parity_lines = |text: &str| {
                    let mut lines: Vec<_> = text
                        .lines()
                        .filter_map(|line| line.find("PARITY_").map(|at| line[at..].to_owned()))
                        .collect();
                    lines.sort();
                    lines
                };
                match self {
                    Self::Pipes(process) => {
                        let output = process.unwrap().finish(timeout);
                        let code = output
                            .status
                            .code()
                            .unwrap_or_else(|| panic!("killed by {:?}", output.status.signal()));
                        let streams = [&output.stdout, &output.stderr]
                            .map(|stream| parity_lines(&String::from_utf8_lossy(stream)));
                        let output = normalize_output(&combined_output(&output));
                        Outcome {
                            code,
                            output,
                            streams,
                        }
                    }
                    Self::Pty { mut group, child } => {
                        let (status, output) = child.finish_with_status(timeout);
                        group.disarm();
                        assert_eq!(status.signal(), None, "{output}");
                        Outcome {
                            code: status.exit_code() as i32,
                            streams: [parity_lines(&output), Vec::new()],
                            output,
                        }
                    }
                }
            }
        }

        /// Every process below the entrypoint once the task is ready: the
        /// wrapper's native child, the package manager, the task, and its
        /// descendant. Guards kill the task groups if an assertion fails.
        struct Tree {
            pids: Vec<i32>,
            guards: Vec<TaskTreeGuard>,
        }

        impl Tree {
            fn capture(root: i32, leaf: i32) -> Self {
                let mut pids = Vec::new();
                let mut parents = vec![root];
                while let Some(parent) = parents.pop() {
                    let output = Command::new("pgrep")
                        .arg("-P")
                        .arg(parent.to_string())
                        .output()
                        .expect("failed to run pgrep");
                    for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
                        let pid = pid.parse().unwrap();
                        pids.push(pid);
                        parents.push(pid);
                    }
                }
                let guards = pids.iter().map(|pid| TaskTreeGuard::new(*pid)).collect();
                let tree = Self { pids, guards };
                assert!(tree.pids.contains(&leaf), "task descendant not found");
                tree
            }

            fn assert_gone(mut self, entrypoint: i32) {
                for pid in std::iter::once(entrypoint).chain(self.pids.iter().copied()) {
                    wait_for_process_gone(pid, EXIT_TIMEOUT);
                }
                for guard in &mut self.guards {
                    guard.disarm();
                }
            }
        }

        fn command(launcher: &ProcessLauncher, entry: Entrypoint, root: &Path) -> Command {
            let mut command = launcher.command(entry, root);
            command
                .args(["run", "dev", "--filter=app-a"])
                .env(
                    "PATH",
                    setup::prepend_to_path(&setup::corepack_dir_for_test_dir(root)),
                )
                .env("COREPACK_HOME", setup::corepack_home())
                .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0");
            command
        }

        /// Starts the blocked-cleanup task and returns once its descendant
        /// runs.
        fn start_blocked(
            launcher: &ProcessLauncher,
            entry: Entrypoint,
            terminal: Terminal,
        ) -> (TempDir, PathBuf, Run, Tree) {
            let (tempdir, root) =
                setup_shutdown_fixture("parity.sh", &blocked_cleanup_script("parity"));
            let app = root.join("apps/app-a");
            let mut run = Run::spawn(command(launcher, entry, &root), terminal);
            run.wait_for(&app.join("ready"));
            let leaf = wait_for_pid_file(&app.join("child.pid"), START_TIMEOUT);
            let tree = Tree::capture(run.pid(), leaf);
            (tempdir, root, run, tree)
        }

        #[test]
        fn argv_cwd_env_streams_and_exit_status_match() {
            let launcher = ProcessLauncher::new();
            let argv = [
                "argument with spaces",
                "--literal",
                "$(touch injected)",
                "quote'\"",
                "snowman ☃",
            ];
            for terminal in [Terminal::Pipes, Terminal::Pty] {
                for exit in [0, 3] {
                    let mut results = Vec::new();
                    for entry in ENTRIES {
                        eprintln!("case: {entry:?} {terminal:?} exit {exit}");
                        let (_tempdir, root) =
                            setup_shutdown_fixture("observe.sh", "exec node ./observe.js \"$@\"\n");
                        let app = root.join("apps/app-a");
                        fs::write(
                            app.join("observe.js"),
                            r#"
const observation = {
  argv: process.argv.slice(2), cwd: process.cwd(), value: process.env.PARITY_VALUE,
  empty: process.env.PARITY_EMPTY, absent: process.env.PARITY_ABSENT ?? null,
};
require('fs').writeFileSync('observation.json', JSON.stringify(observation));
console.log('PARITY_STDOUT ' + JSON.stringify(observation.argv));
console.error('PARITY_STDERR ' + observation.value);
process.exit(Number(process.env.PARITY_EXIT));
"#,
                        )
                        .unwrap();
                        // Invoke below the root so discovery, not cwd, finds it.
                        let mut cmd = command(&launcher, entry, &root);
                        cmd.current_dir(&app)
                            .args(["--env-mode=loose", "--"])
                            .args(argv)
                            .env("PARITY_VALUE", "spaces = and ☃")
                            .env("PARITY_EMPTY", "")
                            .env("PARITY_EXIT", exit.to_string())
                            .env_remove("PARITY_ABSENT");
                        let outcome = Run::spawn(cmd, terminal).finish(EXIT_TIMEOUT);
                        let context = format!("{entry:?} {terminal:?}\n{}", outcome.output);
                        assert_eq!(outcome.code, exit, "{context}");
                        let observation: Value = serde_json::from_slice(
                            &fs::read(app.join("observation.json")).unwrap(),
                        )
                        .unwrap();
                        assert_eq!(observation["argv"], json!(argv), "{context}");
                        assert_eq!(
                            Path::new(observation["cwd"].as_str().unwrap())
                                .canonicalize()
                                .unwrap(),
                            app.canonicalize().unwrap()
                        );
                        assert_eq!(observation["value"], "spaces = and ☃", "{context}");
                        assert_eq!(observation["empty"], "", "{context}");
                        assert_eq!(observation["absent"], Value::Null, "{context}");
                        assert!(!app.join("injected").exists(), "{context}");
                        let mut visible = outcome.streams.concat();
                        visible.sort();
                        assert_eq!(
                            visible,
                            [
                                "PARITY_STDERR spaces = and ☃".to_owned(),
                                format!("PARITY_STDOUT {}", observation["argv"]),
                            ],
                            "{context}"
                        );
                        results.push((outcome.code, outcome.streams));
                    }
                    assert_eq!(results[0], results[1], "{terminal:?} exit {exit}");
                }
            }
        }

        #[test]
        fn first_stop_request_waits_for_one_graceful_cleanup() {
            let launcher = ProcessLauncher::new();
            for terminal in [Terminal::Pipes, Terminal::Pty] {
                for stop in [Stop::CtrlC, Stop::Term] {
                    for entry in ENTRIES {
                        eprintln!("case: {entry:?} {terminal:?} {stop:?}");
                        let (_tempdir, root, mut run, tree) =
                            start_blocked(&launcher, entry, terminal);
                        let app = root.join("apps/app-a");
                        let release = ReleaseGuard::new(&root);
                        let entrypoint = run.pid();
                        run.stop(stop);
                        run.wait_for(&app.join("cleanup.started"));
                        // A request duplicated by the wrapper would make native
                        // turbo force kill the blocked task. The task's own trap
                        // count is not checked: pnpm forwards SIGINT to its
                        // script, as it does for Ctrl+C on `pnpm run` without
                        // turbo, so the trap may run twice either way.
                        thread::sleep(Duration::from_millis(500));
                        run.assert_running("while cleanup was blocked");
                        assert!(!app.join("descendant.release").exists());
                        drop(release);
                        let outcome = run.finish(EXIT_TIMEOUT);
                        let output = &outcome.output;
                        let context = format!("{entry:?} {terminal:?} {stop:?}\n{output}");
                        assert_eq!(outcome.code, 0, "{context}");
                        // Check markers, not trap output: pnpm's forwarded SIGINT
                        // can interrupt the trap's `printf` with EINTR.
                        assert!(app.join("descendant.release").exists(), "{context}");
                        assert_eq!(
                            output.matches("Shutting down Turborepo tasks...").count(),
                            1,
                            "{context}"
                        );
                        assert_eq!(
                            output.contains("Press CTRL+C again to exit forcefully."),
                            terminal == Terminal::Pty,
                            "{context}"
                        );
                        assert!(
                            !output.contains("Force killed") && !output.contains("timed out"),
                            "{context}"
                        );
                        tree.assert_gone(entrypoint);
                    }
                }
            }
        }

        #[test]
        fn second_stop_request_force_kills_without_orphans() {
            let launcher = ProcessLauncher::new();
            for terminal in [Terminal::Pipes, Terminal::Pty] {
                // SIGTERM then SIGTERM is not a parity case: see
                // `npm_wrapper_does_not_escalate_repeated_sigterm`.
                for (first, second) in [(Stop::CtrlC, Stop::CtrlC), (Stop::CtrlC, Stop::Term)] {
                    for entry in ENTRIES {
                        eprintln!("case: {entry:?} {terminal:?} {first:?}+{second:?}");
                        // Never released: the second request, not the timeout,
                        // must end the run.
                        let (_tempdir, root, mut run, tree) =
                            start_blocked(&launcher, entry, terminal);
                        let app = root.join("apps/app-a");
                        let _release = ReleaseGuard::new(&root);
                        let entrypoint = run.pid();
                        run.stop(first);
                        run.wait_for(&app.join("cleanup.started"));
                        run.assert_running("before the second request");
                        run.stop(second);
                        let outcome = run.finish(Duration::from_secs(5));
                        let output = &outcome.output;
                        let context =
                            format!("{entry:?} {terminal:?} {first:?}+{second:?}\n{output}");
                        assert_eq!(outcome.code, 0, "{context}");
                        assert!(
                            output.contains("Force killed Turborepo tasks: app-a#dev"),
                            "{context}"
                        );
                        assert!(!output.contains("Graceful shutdown timed out"), "{context}");
                        assert!(!output.contains("parity cleanup done"), "{context}");
                        assert!(!app.join("descendant.release").exists(), "{context}");
                        tree.assert_gone(entrypoint);
                    }
                }
            }
        }

        /// Escalation through the npm wrapper is not supported: the wrapper
        /// forwards only the first SIGTERM to native turbo, so a second one
        /// does not force kill. This pins that behavior so changing it
        /// is deliberate, and checks the run still finishes its
        /// graceful cleanup without orphans.
        #[test]
        fn npm_wrapper_does_not_escalate_repeated_sigterm() {
            let launcher = ProcessLauncher::new();
            for terminal in [Terminal::Pipes, Terminal::Pty] {
                eprintln!("case: {terminal:?}");
                let (_tempdir, root, mut run, tree) =
                    start_blocked(&launcher, Entrypoint::NpmWrapper, terminal);
                let app = root.join("apps/app-a");
                let release = ReleaseGuard::new(&root);
                let entrypoint = run.pid();
                run.stop(Stop::Term);
                run.wait_for(&app.join("cleanup.started"));
                run.assert_running("before the second SIGTERM");
                run.stop(Stop::Term);
                thread::sleep(Duration::from_millis(500));
                run.assert_running("after the second SIGTERM");
                assert!(!app.join("descendant.release").exists());
                drop(release);
                let outcome = run.finish(EXIT_TIMEOUT);
                let output = &outcome.output;
                let context = format!("{terminal:?}\n{output}");
                assert_eq!(outcome.code, 0, "{context}");
                assert!(app.join("descendant.release").exists(), "{context}");
                assert!(
                    !output.contains("Force killed") && !output.contains("timed out"),
                    "{context}"
                );
                tree.assert_gone(entrypoint);
            }
        }

        #[test]
        fn noninteractive_shutdown_timeout_force_kills_without_orphans() {
            let launcher = ProcessLauncher::new();
            for entry in ENTRIES {
                eprintln!("case: {entry:?}");
                let (_tempdir, root, mut run, tree) =
                    start_blocked(&launcher, entry, Terminal::Pipes);
                let app = root.join("apps/app-a");
                let _release = ReleaseGuard::new(&root);
                let entrypoint = run.pid();
                let started = Instant::now();
                run.stop(Stop::CtrlC);
                run.wait_for(&app.join("cleanup.started"));
                let outcome = run.finish(EXIT_TIMEOUT);
                let context = format!("{entry:?}\n{}", outcome.output);
                assert!(started.elapsed() >= Duration::from_secs(9), "{context}");
                assert_eq!(outcome.code, 0, "{context}");
                assert!(
                    outcome.output.contains(
                        "Graceful shutdown timed out. Force killed Turborepo tasks: app-a#dev"
                    ),
                    "{context}"
                );
                assert!(!app.join("descendant.release").exists(), "{context}");
                tree.assert_gone(entrypoint);
            }
        }
    }
}
