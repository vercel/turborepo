mod common;

use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use common::{combined_output, setup};

fn setup_stdin_eof_fixture() -> tempfile::TempDir {
    let tempdir = tempfile::tempdir().unwrap();
    setup::setup_integration_test(tempdir.path(), "stdin_eof_startup", "npm@10.5.0", false)
        .unwrap();
    tempdir
}

fn spawn_turbo_run_dev(test_dir: &Path, config_dir: &Path) -> Child {
    let turbo_bin = assert_cmd::cargo::cargo_bin("turbo");
    let mut cmd = Command::new(turbo_bin);
    cmd.arg("run")
        .arg("dev")
        .env("TURBO_TELEMETRY_MESSAGE_DISABLED", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_PRINT_VERSION_DISABLED", "1")
        .env("TURBO_CONFIG_DIR_PATH", config_dir)
        .env("DO_NOT_TRACK", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .env_remove("CI")
        .env_remove("GITHUB_ACTIONS")
        .current_dir(test_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().expect("failed to spawn turbo")
}

fn read_child_output(child: &mut Child) -> (Vec<u8>, Vec<u8>) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    if let Some(mut reader) = child.stdout.take() {
        reader
            .read_to_end(&mut stdout)
            .expect("failed to read child stdout");
    }
    if let Some(mut reader) = child.stderr.take() {
        reader
            .read_to_end(&mut stderr)
            .expect("failed to read child stderr");
    }

    (stdout, stderr)
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> Output {
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let (stdout, stderr) = read_child_output(&mut child);
                    panic!(
                        "turbo timed out after {timeout:?}\nstdout:\n{}\nstderr:\n{}",
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr),
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(err) => panic!("failed waiting for turbo: {err}"),
        }
    };

    let (stdout, stderr) = read_child_output(&mut child);
    Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn nonpersistent_task_sees_eof_on_stdin_in_stream_mode() {
    let tempdir = setup_stdin_eof_fixture();
    let config_dir = tempfile::tempdir().unwrap();

    let child = spawn_turbo_run_dev(tempdir.path(), config_dir.path());
    let output = wait_with_timeout(child, Duration::from_secs(15));

    assert!(
        output.status.success(),
        "expected turbo to succeed, got {:?}\n{}",
        output.status.code(),
        combined_output(&output)
    );

    let combined = combined_output(&output);
    assert!(
        combined.contains("stdin bytes=0"),
        "expected task output to show EOF on stdin, got:\n{combined}"
    );
    assert!(
        combined.contains("started"),
        "expected task output to show startup completed, got:\n{combined}"
    );
}
