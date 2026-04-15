mod common;

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        io::{Read, Write},
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

    use crate::common::{self, setup};

    const START_TIMEOUT: Duration = Duration::from_secs(15);
    const EXIT_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_PTY_OUTPUT_BYTES: usize = 256 * 1024;

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

        fn into_output(mut self, timeout: Duration) -> Output {
            let _ = wait_for_process_exit(self.child_mut(), timeout);
            self.child
                .take()
                .expect("child guard consumed")
                .wait_with_output()
                .expect("failed waiting for child output")
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

        fn finish(mut self, timeout: Duration) -> String {
            let start = Instant::now();
            loop {
                let status = self
                    .child
                    .as_mut()
                    .expect("pty child guard consumed")
                    .try_wait()
                    .expect("failed waiting for pty child");
                if status.is_some() {
                    break;
                }
                if start.elapsed() > timeout {
                    panic!("timed out waiting for pty child exit");
                }
                thread::sleep(Duration::from_millis(100));
            }

            let _ = self.child.take();
            drop(self.writer.take());
            if let Some(reader_thread) = self.reader_thread.take() {
                reader_thread.join().expect("pty reader thread panicked");
            }

            normalize_output(String::from_utf8_lossy(&self.output.lock().unwrap()).as_ref())
        }
    }

    impl Drop for PtyTurbo {
        fn drop(&mut self) {
            drop(self.writer.take());
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(reader_thread) = self.reader_thread.take() {
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

    fn example_fixture_dir() -> PathBuf {
        common::manifest_dir().join("../../examples/with-shell-commands")
    }

    fn setup_shutdown_example(script_name: &str, script_contents: &str) -> (TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().expect("failed to create tempdir");
        let test_dir = tempdir.path().to_path_buf();
        setup::copy_dir_all(&example_fixture_dir(), &test_dir).expect("failed to copy example");
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

    fn turbo_bin() -> PathBuf {
        assert_cmd::cargo::cargo_bin("turbo")
    }

    fn spawn_noninteractive_turbo(test_dir: &Path) -> ChildGuard {
        let mut cmd = Command::new(turbo_bin());
        cmd.arg("run")
            .arg("dev")
            .arg("--filter=app-a")
            .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
            .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
            .env("TURBO_PRINT_VERSION_DISABLED", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
            .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
            .env_remove("CI")
            .env_remove("GITHUB_ACTIONS")
            .current_dir(test_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ChildGuard::new(cmd.spawn().expect("failed to spawn turbo"))
    }

    fn spawn_interactive_turbo(test_dir: &Path) -> PtyTurbo {
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

        let mut command = CommandBuilder::new(turbo_bin());
        command.arg("run");
        command.arg("dev");
        command.arg("--filter=app-a");
        command.cwd(test_dir);
        command.env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1");
        command.env("TURBO_GLOBAL_WARNING_DISABLED", "1");
        command.env("TURBO_PRINT_VERSION_DISABLED", "1");
        command.env("DO_NOT_TRACK", "1");
        command.env("NPM_CONFIG_UPDATE_NOTIFIER", "false");
        command.env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0");
        command.env_remove("CI");
        command.env_remove("GITHUB_ACTIONS");

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

    fn wait_for_path(path: &Path, timeout: Duration) {
        let start = Instant::now();
        while !path.exists() {
            if start.elapsed() > timeout {
                panic!("timed out waiting for {}", path.display());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status,
                Ok(None) if start.elapsed() <= timeout => thread::sleep(Duration::from_millis(100)),
                Ok(None) => panic!("timed out waiting for child exit"),
                Err(err) => panic!("failed waiting for child exit: {err}"),
            }
        }
    }

    fn send_signal(pid: i32, signal_kind: Signal) {
        signal::kill(Pid::from_raw(pid), signal_kind).expect("failed to send signal");
    }

    fn process_exists(pid: i32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn wait_for_process_gone(pid: i32, timeout: Duration) {
        let start = Instant::now();
        while process_exists(pid) {
            if start.elapsed() > timeout {
                panic!("timed out waiting for process {pid} to exit");
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn normalize_output(text: &str) -> String {
        text.replace("\r\n", "\n").replace('\r', "\n")
    }

    #[test]
    fn run_finishes_successfully_without_shutdown_banner() {
        let (_tempdir, test_dir) = setup_shutdown_example(
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
    fn run_gracefully_shuts_down_on_first_sigint_in_tty() {
        let (_tempdir, test_dir) = setup_shutdown_example(
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
        wait_for_path(&ready_file, START_TIMEOUT);

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
    }

    #[test]
    fn run_gracefully_shuts_down_on_first_sigint_without_tty_and_exits_zero() {
        let (_tempdir, test_dir) = setup_shutdown_example(
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
        wait_for_path(&ready_file, START_TIMEOUT);

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
    fn run_force_kills_on_second_sigint_in_tty() {
        let (_tempdir, test_dir) = setup_shutdown_example(
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
        wait_for_path(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);

        child.send_ctrl_c();
        thread::sleep(Duration::from_millis(3500));
        child.send_ctrl_c();

        let transcript = child.finish(Duration::from_secs(5));
        wait_for_process_gone(task_child_pid, Duration::from_secs(5));

        assert!(
            transcript.contains("stubborn ready child="),
            "expected task output to remain visible in the TUI transcript\n{transcript}"
        );
    }

    #[test]
    fn run_force_kills_after_timeout_without_tty() {
        let (_tempdir, test_dir) = setup_shutdown_example(
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
        wait_for_path(&ready_file, START_TIMEOUT);
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
            combined
                .contains("Some tasks in your Turborepo are taking awhile to shut down: app-a#dev"),
            "expected slow-task warning before the forced shutdown\n{combined}"
        );
        assert!(
            combined.contains("Shutting down forcibly in 7s..."),
            "expected remaining timeout warning before the forced shutdown\n{combined}"
        );
        assert!(
            combined
                .contains("Graceful shutdown timed out. Force killing Turborepo tasks: app-a#dev"),
            "expected auto-force banner after timeout\n{combined}"
        );
    }

    #[test]
    fn run_kills_task_tree_when_turbo_is_sigkilled() {
        let (_tempdir, test_dir) = setup_shutdown_example(
            "orphanable.sh",
            r#"#!/usr/bin/env bash
set -u
sh -c 'trap "" TERM INT; while true; do sleep 0.2 || true; done' &
child=$!
printf '%s\n' "$child" > child.pid
printf "orphanable ready child=%s\n" "$child"
: > ready
while true; do sleep 0.2 || true; done
"#,
        );

        let ready_file = test_dir.join("apps/app-a/ready");
        let child_pid_file = test_dir.join("apps/app-a/child.pid");

        let mut child = spawn_noninteractive_turbo(&test_dir);
        wait_for_path(&ready_file, START_TIMEOUT);
        let task_child_pid = wait_for_pid_file(&child_pid_file, START_TIMEOUT);

        send_signal(child.child_mut().id() as i32, Signal::SIGKILL);
        let _status = wait_for_process_exit(child.child_mut(), Duration::from_secs(5));
        let _ = child.child.take();

        wait_for_process_gone(task_child_pid, Duration::from_secs(5));
    }
}
