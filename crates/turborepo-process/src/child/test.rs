use std::{
    assert_matches, fs, io,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::{StreamExt, stream::FuturesUnordered};
use test_case::test_case;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
};
use tracing_test::traced_test;
use turbopath::AbsoluteSystemPathBuf;

use super::{Child, ChildInput, ChildOutput, ChildStdin, Command};
use crate::{
    PtySize,
    child::{ChildExit, ShutdownStyle},
};

const STARTUP_DELAY: Duration = Duration::from_millis(500);
// We skip testing PTY usage on Windows
const TEST_PTY: bool = !cfg!(windows);

struct ObservedOutput {
    buffer: Arc<Mutex<Vec<u8>>>,
    ready_tx: Option<oneshot::Sender<()>>,
}

impl ObservedOutput {
    fn new() -> (Self, Arc<Mutex<Vec<u8>>>, oneshot::Receiver<()>) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = oneshot::channel();
        (
            Self {
                buffer: buffer.clone(),
                ready_tx: Some(ready_tx),
            },
            buffer,
            ready_rx,
        )
    }
}

impl io::Write for ObservedOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let saw_ready = {
            let mut buffer = self.buffer.lock().unwrap();
            buffer.extend_from_slice(buf);
            String::from_utf8_lossy(&buffer).contains("ready")
        };

        if saw_ready && let Some(ready_tx) = self.ready_tx.take() {
            ready_tx.send(()).ok();
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
const EOT: char = '\u{4}';

fn find_script_dir() -> AbsoluteSystemPathBuf {
    let cwd = AbsoluteSystemPathBuf::cwd().unwrap();
    let mut root = cwd;
    while !root.join_component(".git").exists() {
        root = root.parent().unwrap().to_owned();
    }
    root.join_components(&["crates", "turborepo-process", "test", "scripts"])
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_pid(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    assert_matches!(child.pid(), Some(_));
    child.stop().await;

    let exit = child.wait().await;
    assert_matches!(exit, Some(ChildExit::Killed));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tracing_test::traced_test]
#[tokio::test]
async fn test_wait(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let exit1 = child.wait().await;
    let exit2 = child.wait().await;
    assert_matches!(exit1, Some(ChildExit::Finished(Some(0))));
    assert_matches!(exit2, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_spawn(use_pty: bool) {
    let cmd = {
        let script = find_script_dir().join_component("hello_world.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd
    };

    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    assert!(child.is_running());

    let code = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("child wait should not hang after process exit");
    assert_eq!(code, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_stdout(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    {
        let mut output = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                stdout
                    .read_to_end(&mut output)
                    .await
                    .expect("Failed to read stdout");
            }
            ChildOutput::Pty(mut outputs) => {
                outputs
                    .read_to_end(&mut output)
                    .expect("failed to read stdout");
            }
        };

        let output_str = String::from_utf8(output).expect("Failed to parse stdout");
        let trimmed_output = output_str.trim();
        let trimmed_output = trimmed_output.strip_prefix(EOT).unwrap_or(trimmed_output);

        assert_eq!(trimmed_output, "hello world");
    }

    let exit = child.wait().await;

    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_stdio(use_pty: bool) {
    let script = find_script_dir().join_component("stdin_stdout.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let input = "hello world";
    // drop stdin to close the pipe
    {
        match child.stdin_inner().unwrap() {
            ChildInput::Std(mut stdin) => stdin.write_all(input.as_bytes()).await.unwrap(),
            ChildInput::Pty(mut stdin) => stdin.write_all(input.as_bytes()).unwrap(),
        }
    }

    let mut output = Vec::new();
    match child.outputs().unwrap() {
        ChildOutput::Std { mut stdout, .. } => stdout.read_to_end(&mut output).await.unwrap(),
        ChildOutput::Pty(mut stdout) => stdout.read_to_end(&mut output).unwrap(),
    };

    let output_str = String::from_utf8(output).expect("Failed to parse stdout");
    let trimmed_out = output_str.trim();
    let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);

    assert!(trimmed_out.contains(input), "got: {trimmed_out}");

    let exit = child.wait().await;
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Regression test for #7834: proves that a child process can block
/// before producing any output if stdin is an open pipe instead of EOF.
///
/// This models the v1.13 regression on Windows stream mode:
/// `tsx watch` received an open piped stdin and never started executing.
#[tokio::test]
async fn test_std_open_stdin_blocks_startup_until_eof() {
    let script = find_script_dir().join_component("startup_after_stdin_eof.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let ChildOutput::Std { mut stdout, .. } = child.outputs().unwrap() else {
        panic!("expected stdio child");
    };

    let mut output = Vec::new();
    let result =
        tokio::time::timeout(Duration::from_secs(1), stdout.read_to_end(&mut output)).await;
    assert!(
        result.is_err(),
        "child should stay blocked while stdin is held open"
    );
    assert!(
        output.is_empty(),
        "child should not produce output before stdin reaches EOF"
    );

    // Closing the parent's stdin pipe should unblock the child immediately.
    drop(child.stdin_inner());

    tokio::time::timeout(Duration::from_secs(5), stdout.read_to_end(&mut output))
        .await
        .expect("child should finish reading after stdin is closed")
        .expect("failed to read child output");

    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("child should exit after stdin is closed");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));

    let output = String::from_utf8(output).unwrap().replace("\r\n", "\n");
    assert_eq!(output, "stdin bytes=0\nstarted\n");
}

/// Regression test for #7834: verifies the pre-v1.13 behavior where tasks
/// that do not need input start immediately when stdin is already at EOF.
#[tokio::test]
async fn test_std_null_stdin_allows_startup() {
    let script = find_script_dir().join_component("startup_after_stdin_eof.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

    let mut output = Vec::new();
    let exit = tokio::time::timeout(
        Duration::from_secs(5),
        child.wait_with_piped_outputs(&mut output),
    )
    .await
    .expect("child should not block when stdin is null")
    .expect("failed to wait for child output");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));

    let output = String::from_utf8(output).unwrap().replace("\r\n", "\n");
    assert_eq!(output, "stdin bytes=0\nstarted\n");
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_graceful_shutdown_timeout(use_pty: bool) {
    let cmd = {
        let script = find_script_dir().join_component("sleep_5_ignore.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd
    };

    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    let mut buf = vec![0; 4];
    // wait for the process to print "here"
    match child.outputs().unwrap() {
        ChildOutput::Std { mut stdout, .. } => {
            stdout.read_exact(&mut buf).await.unwrap();
        }
        ChildOutput::Pty(mut stdout) => {
            stdout.read_exact(&mut buf).unwrap();
        }
    };
    child.stop().await;

    let exit = child.wait().await;
    // this should time out and be killed
    assert_matches!(exit, Some(ChildExit::Killed));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_graceful_shutdown(use_pty: bool) {
    let cmd = {
        let script = find_script_dir().join_component("sleep_5_interruptable.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd
    };

    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    // We need to read the child output otherwise the child will be unable to
    // cleanly shut down as it waits for the receiving end of the PTY to read
    // the output before exiting.
    let mut output_child = child.clone();
    tokio::task::spawn(async move {
        let mut output = Vec::new();
        output_child.wait_with_piped_outputs(&mut output).await.ok();
    });

    child.stop().await;
    let exit = child.wait().await;

    if cfg!(windows) && !use_pty {
        assert_matches!(exit, Some(ChildExit::Killed));
    } else {
        assert_matches!(exit, Some(ChildExit::Interrupted));
    }
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_graceful_shutdown_drains_final_output(use_pty: bool) {
    let script = find_script_dir().join_component("graceful_sigint_output.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);

    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    let mut output_child = child.clone();
    let (mut observer, output, ready_rx) = ObservedOutput::new();
    let output_task = tokio::spawn(async move {
        output_child
            .wait_with_piped_outputs(&mut observer)
            .await
            .unwrap()
    });

    tokio::time::timeout(Duration::from_secs(2), ready_rx)
        .await
        .expect("timed out waiting for startup output")
        .expect("ready notification channel closed unexpectedly");
    child.set_closing();
    child.stop().await;
    let exit = output_task.await.unwrap();
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();

    assert!(output.contains("ready"), "missing startup output: {output}");

    if cfg!(windows) && !use_pty {
        assert_matches!(exit, Some(ChildExit::Killed));
    } else {
        assert!(
            output.contains("received SIGINT"),
            "missing SIGINT receipt log: {output}"
        );
        assert!(
            output.contains("exiting after SIGINT"),
            "missing SIGINT exit log: {output}"
        );
        assert_matches!(exit, Some(ChildExit::Interrupted));
    }
}

#[cfg(windows)]
#[tokio::test]
async fn test_windows_pty_graceful_shutdown_receives_ctrl_c() {
    let script = find_script_dir().join_component("graceful_sigint_output.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);

    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        Some(PtySize::default()),
    )
    .unwrap();

    let mut output_child = child.clone();
    let (mut observer, output, ready_rx) = ObservedOutput::new();
    let output_task = tokio::spawn(async move {
        output_child
            .wait_with_piped_outputs(&mut observer)
            .await
            .unwrap()
    });

    tokio::time::timeout(Duration::from_secs(2), ready_rx)
        .await
        .expect("timed out waiting for startup output")
        .expect("ready notification channel closed unexpectedly");
    child.set_closing();
    child.stop().await;
    let exit = output_task.await.unwrap();
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();

    assert!(output.contains("ready"), "missing startup output: {output}");
    assert!(
        output.contains("received SIGINT"),
        "missing SIGINT receipt log: {output}"
    );
    assert!(
        output.contains("exiting after SIGINT"),
        "missing SIGINT exit log: {output}"
    );
    assert_matches!(exit, Some(ChildExit::Interrupted));
}

// Regression test: a wrapper process (simulating npm/pnpm) forwards SIGINT
// to its child. When turbo sends SIGINT to the process group, the child
// gets it twice — once from the group signal, once from the wrapper.
// For PTY children we now signal only the direct PID to avoid this.
#[cfg(unix)]
#[tokio::test]
#[traced_test]
async fn test_pty_child_receives_single_sigint() {
    let script = find_script_dir().join_component("wrapper_count_sigints.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(2000))),
        Some(PtySize::default()),
    )
    .unwrap();

    let mut output_child = child.clone();
    let (mut observer, output, ready_rx) = ObservedOutput::new();
    let output_task = tokio::spawn(async move {
        output_child
            .wait_with_piped_outputs(&mut observer)
            .await
            .unwrap()
    });

    tokio::time::timeout(Duration::from_secs(30), ready_rx)
        .await
        .expect("timed out waiting for ready")
        .expect("ready channel closed");

    child.set_closing();
    child.stop().await;
    output_task.await.unwrap();

    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("SIGINT_COUNT=1"),
        "expected exactly one SIGINT, got output: {output}"
    );
    assert!(
        !output.contains("SIGINT_COUNT=2"),
        "child received SIGINT twice: {output}"
    );
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_detect_killed_someone_else(use_pty: bool) {
    let cmd = {
        let script = find_script_dir().join_component("sleep_5_interruptable.js");
        let mut cmd = Command::new("node");
        cmd.args([script.as_std_path()]);
        cmd
    };

    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    #[cfg(unix)]
    if let Some(pid) = child.pid() {
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    }
    #[cfg(windows)]
    if let Some(pid) = child.pid() {
        unsafe {
            println!("killing");
            windows_sys::Win32::System::Threading::TerminateProcess(
                windows_sys::Win32::System::Threading::OpenProcess(
                    windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
                    0,
                    pid,
                ),
                3,
            );
        }
    }

    let exit = child.wait().await;

    #[cfg(unix)]
    assert_matches!(exit, Some(ChildExit::KilledExternal));
    #[cfg(not(unix))]
    assert_matches!(exit, Some(ChildExit::Finished(Some(3))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_wait_with_output(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut out = Vec::new();

    let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

    let out = String::from_utf8(out).unwrap();
    let trimmed_out = out.trim();
    let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);

    assert_eq!(trimmed_out, "hello world");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_wait_with_single_output(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world_hello_moon.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut buffer = Vec::new();

    let exit = child.wait_with_piped_outputs(&mut buffer).await.unwrap();

    let output = String::from_utf8(buffer).unwrap();

    // There are no ordering guarantees so we just check that both logs made it
    let expected_stdout = "hello world";
    let expected_stderr = "hello moon";
    assert!(output.contains(expected_stdout), "got: {output}");
    assert!(output.contains(expected_stderr), "got: {output}");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_wait_with_with_non_utf8_output(use_pty: bool) {
    let script = find_script_dir().join_component("hello_non_utf8.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut out = Vec::new();

    let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

    let expected = &[0, 159, 146, 150];
    let trimmed_out = out.trim_ascii();
    let trimmed_out = trimmed_out.strip_prefix(&[4]).unwrap_or(trimmed_out);
    assert_eq!(trimmed_out, expected);
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_no_newline(use_pty: bool) {
    let script = find_script_dir().join_component("hello_no_line.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut out = Vec::new();

    let exit = child.wait_with_piped_outputs(&mut out).await.unwrap();

    let output = String::from_utf8(out).unwrap();
    let trimmed_out = output.trim();
    let trimmed_out = trimmed_out.strip_prefix(EOT).unwrap_or(trimmed_out);
    assert!(
        output.ends_with('\n'),
        "expected newline to be added: {output}"
    );
    assert_eq!(trimmed_out, "look ma, no newline!");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[cfg(unix)]
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_kill_process_group(use_pty: bool) {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "while true; do sleep 0.2; done"]);
    let mut child = Child::spawn(
        cmd,
        // Bumping this to give ample time for the process to respond to the SIGINT to reduce
        // flakiness inherent with sending and receiving signals.
        ShutdownStyle::Graceful(Some(Duration::from_millis(1000))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    // We need to read the child output otherwise the child will be unable to
    // cleanly shut down as it waits for the receiving end of the PTY to read
    // the output before exiting.
    let mut output_child = child.clone();
    tokio::task::spawn(async move {
        let mut output = Vec::new();
        output_child.wait_with_piped_outputs(&mut output).await.ok();
    });

    let exit = child.stop().await;

    // On Unix, shell scripts may not respond to SIGINT and will timeout,
    // resulting in being killed rather than interrupted.
    if cfg!(unix) {
        assert_matches!(exit, Some(ChildExit::Killed) | Some(ChildExit::Interrupted));
    } else {
        assert_matches!(exit, Some(ChildExit::Interrupted));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_orphan_process() {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", "echo hello; exec sleep 120"]);
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let child_pid = child.pid().unwrap() as i32;
    // We don't kill the process group to simulate what an external program might do
    unsafe {
        libc::kill(child_pid, libc::SIGKILL);
    }

    let exit = child.wait().await;
    assert_matches!(exit, Some(ChildExit::KilledExternal));

    let mut output = Vec::new();
    match tokio::time::timeout(
        Duration::from_millis(500),
        child.wait_with_piped_outputs(&mut output),
    )
    .await
    {
        Ok(exit_status) => {
            assert_matches!(exit_status, Ok(Some(ChildExit::KilledExternal)));
        }
        Err(_) => panic!("expected wait_with_piped_outputs to exit after it was killed"),
    }
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_graceful_shutdown_waits_for_force_kill(use_pty: bool) {
    let script = find_script_dir().join_component("sleep_5_ignore.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_secs(5))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    let mut buf = vec![0; 4];
    match child.outputs().unwrap() {
        ChildOutput::Std { mut stdout, .. } => {
            stdout.read_exact(&mut buf).await.unwrap();
        }
        ChildOutput::Pty(mut stdout) => {
            stdout.read_exact(&mut buf).unwrap();
        }
    };

    let mut shutdown_child = child.clone();
    let shutdown =
        tokio::spawn(async move { shutdown_child.shutdown(ShutdownStyle::Graceful(None)).await });

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !shutdown.is_finished(),
        "graceful shutdown should keep waiting until explicitly forced"
    );

    assert_eq!(child.kill().await, Some(ChildExit::Killed));
    assert_eq!(shutdown.await.unwrap(), Some(ChildExit::Killed));
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_multistop(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut stops = FuturesUnordered::new();
    for _ in 1..10 {
        let mut child = child.clone();
        stops.push(async move {
            child.stop().await;
        });
    }

    while tokio::time::timeout(Duration::from_secs(5), stops.next())
        .await
        .expect("timed out")
        .is_some()
    {}
}

// Regression tests for https://github.com/vercel/turborepo/issues/11808
//
// On Windows, portable-pty 0.9.0 added PSEUDOCONSOLE_INHERIT_CURSOR to
// ConPTY creation, which requires the host to handle DSR (Device Status
// Report) escape sequences. Turborepo doesn't, causing ConPTY to hang.
//
// Additionally, an unconditional `drop(stdin)` in the PTY path of
// wait_with_piped_outputs would kill ConPTY children on Windows because
// closing ConPTY stdin terminates the session.
//
// These tests verify the fixes: PTY children start, produce output, and
// exit normally without hanging or being killed by stdin closure.

/// Verifies that a PTY-spawned short-lived process produces output and
/// exits cleanly via wait_with_piped_outputs. Uses a timeout to catch
/// the ConPTY hang that occurred with PSEUDOCONSOLE_INHERIT_CURSOR.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_pty_child_does_not_hang(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    let mut out = Vec::new();

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        child.wait_with_piped_outputs(&mut out),
    )
    .await;

    let exit = result
        .expect("PTY child hung — likely PSEUDOCONSOLE_INHERIT_CURSOR regression")
        .unwrap();

    let output = String::from_utf8(out).unwrap();
    let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
    assert_eq!(trimmed, "hello world");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Simulates the persistent-task flow: stdin is taken by the caller
/// (as the TUI does for interactive tasks) BEFORE wait_with_piped_outputs
/// is called. The child should still produce output and exit normally
/// without wait_with_piped_outputs interfering with stdin.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_pty_stdin_taken_before_piped_outputs(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    // Take stdin before piping outputs, simulating TUI taking ownership.
    // For PTY children, this returns Some; for non-PTY, stdin() returns None
    // (Std variant is filtered out), but stdin_inner still removes it.
    let _stdin_guard = child.stdin();

    // Verify stdin_inner is now empty (already taken).
    assert!(
        child.stdin_inner().is_none(),
        "stdin should already be taken"
    );

    let mut out = Vec::new();

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        child.wait_with_piped_outputs(&mut out),
    )
    .await;

    let exit = result
        .expect("child hung — wait_with_piped_outputs likely interfered with taken stdin")
        .unwrap();

    let output = String::from_utf8(out).unwrap();
    let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
    assert_eq!(trimmed, "hello world");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Verifies that a PTY-spawned process with open stdin that has NOT been
/// taken by the caller still completes normally. This is the non-persistent
/// task path where exec.rs does not take stdin before
/// wait_with_piped_outputs.
///
/// Before the fix, on Windows the unconditional stdin drop inside
/// wait_with_piped_outputs would kill the ConPTY child immediately.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_pty_untaken_stdin_does_not_kill_child(use_pty: bool) {
    let script = find_script_dir().join_component("hello_world.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    // Do NOT take stdin — this simulates a non-persistent task where
    // exec.rs skips stdin handling on Windows (closing_stdin_ends_process).
    // wait_with_piped_outputs should still work without killing the child.
    let mut out = Vec::new();

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        child.wait_with_piped_outputs(&mut out),
    )
    .await;

    let exit = result
        .expect("child process hung or was killed by premature stdin closure")
        .unwrap();

    let output = String::from_utf8(out).unwrap();
    let trimmed = output.trim().strip_prefix(EOT).unwrap_or(output.trim());
    assert_eq!(trimmed, "hello world");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Regression test for #12393: proves that dropping stdin causes a
/// persistent-style child (one that exits on stdin EOF) to terminate.
///
/// This documents the mechanism behind the bug: when the task executor
/// took stdin and passed it to `TaskOutput::set_stdin()` in stream mode,
/// the stdin was dropped immediately, sending EOF to the child.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_dropping_stdin_terminates_persistent_child(use_pty: bool) {
    let script = find_script_dir().join_component("persistent_server.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    // Take stdin and immediately drop it — simulates the bug where
    // TaskOutput::stream().set_stdin() dropped stdin in stream mode.
    {
        let _dropped = child.stdin();
    }

    // The child should exit because it received EOF on stdin.
    let mut out = Vec::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        child.wait_with_piped_outputs(&mut out),
    )
    .await;

    let exit = result
        .expect("child should have exited after stdin was dropped")
        .unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("server ready"),
        "expected 'server ready' in output, got: {output:?}"
    );
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Regression test for #12393: proves that holding stdin in a guard
/// keeps a persistent-style child alive.
///
/// This is the correct behavior after the fix: in stream mode, stdin
/// is held by `_stdin_guard` instead of being passed to
/// `TaskOutput::set_stdin()` which would drop it.
///
/// This covers the writable PTY path used for interactive input.
#[tokio::test]
async fn test_held_stdin_keeps_persistent_child_alive() {
    if !TEST_PTY {
        return;
    }
    let script = find_script_dir().join_component("persistent_server.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, Some(PtySize::default())).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    // Hold stdin in a guard — simulates the correct persistent task flow.
    let _stdin_guard = child.stdin();
    assert!(
        _stdin_guard.is_some(),
        "PTY child should return Some from stdin()"
    );

    // The child should NOT exit while we hold stdin. Give it a moment
    // and verify it's still alive by checking that wait times out.
    let result = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    assert!(
        result.is_err(),
        "child should still be alive while stdin is held"
    );

    // Now drop the guard — child should exit.
    drop(_stdin_guard);

    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("child should exit after stdin guard is dropped");

    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[tokio::test]
async fn test_taken_pty_stdin_guard_keeps_persistent_child_alive() {
    if !TEST_PTY {
        return;
    }
    let script = find_script_dir().join_component("persistent_server.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, Some(PtySize::default())).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let stdin_guard = child.take_stdin();
    assert!(
        matches!(stdin_guard, Some(ChildStdin::Writable(_))),
        "PTY child should return a writable stdin guard"
    );

    let result = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    assert!(
        result.is_err(),
        "child should still be alive while stdin is held"
    );

    drop(stdin_guard);

    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("child should exit after stdin guard is dropped");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

#[tokio::test]
async fn test_non_pty_stdin_guard_keeps_persistent_child_alive() {
    let script = find_script_dir().join_component("persistent_server.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, None).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let stdin_guard = child.take_stdin();
    assert!(
        matches!(stdin_guard, Some(ChildStdin::Guard(_))),
        "non-PTY child should return a stdin guard"
    );

    let result = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    assert!(
        result.is_err(),
        "child should still be alive while stdin is held"
    );

    drop(stdin_guard);

    let exit = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("child should exit after stdin guard is dropped");
    assert_matches!(exit, Some(ChildExit::Finished(Some(0))));
}

/// Verifies that stopping a parent process also kills its child processes.
///
/// On Unix this works via process groups (setpgid + kill(-pgid)).
/// On Windows this works via Job Objects
/// (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
///
/// The test spawns a Node.js script that itself spawns a long-running child
/// process, captures the grandchild's PID from stdout, stops the parent,
/// and then checks that the grandchild is no longer alive.
#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
#[traced_test]
async fn test_process_tree_cleanup(use_pty: bool) {
    let script = find_script_dir().join_component("spawn_child_sleep.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
        use_pty.then(PtySize::default),
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    // Read stdout to get the grandchild PID
    let grandchild_pid = {
        let mut out = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                let mut buf = vec![0u8; 256];
                let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                    .await
                    .expect("timed out reading grandchild PID")
                    .expect("failed to read stdout");
                out.extend_from_slice(&buf[..n]);
            }
            ChildOutput::Pty(mut reader) => {
                let mut buf = vec![0u8; 256];
                let n = reader.read(&mut buf).expect("failed to read pty output");
                out.extend_from_slice(&buf[..n]);
            }
        };
        let output = String::from_utf8(out).unwrap();
        let pid_line = output
            .lines()
            .find(|line| line.contains("CHILD_PID="))
            .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
        pid_line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    };

    // Verify grandchild is alive before we stop
    assert!(
        is_process_alive(grandchild_pid),
        "grandchild process {grandchild_pid} should be alive before stop"
    );

    // Stop the parent process
    child.stop().await;

    // Give the OS a moment to clean up
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify grandchild is dead
    assert!(
        !is_process_alive(grandchild_pid),
        "grandchild process {grandchild_pid} should have been killed"
    );
}

#[test_case(false)]
#[test_case(TEST_PTY)]
#[tokio::test]
async fn test_force_kill_process_tree_cleanup(use_pty: bool) {
    let script = find_script_dir().join_component("spawn_child_sleep.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(cmd, ShutdownStyle::Kill, use_pty.then(PtySize::default)).unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let grandchild_pid = {
        let mut out = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                let mut buf = vec![0u8; 256];
                let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                    .await
                    .expect("timed out reading grandchild PID")
                    .expect("failed to read stdout");
                out.extend_from_slice(&buf[..n]);
            }
            ChildOutput::Pty(mut reader) => {
                let mut buf = vec![0u8; 256];
                let n = reader.read(&mut buf).expect("failed to read pty output");
                out.extend_from_slice(&buf[..n]);
            }
        };
        let output = String::from_utf8(out).unwrap();
        let pid_line = output
            .lines()
            .find(|line| line.contains("CHILD_PID="))
            .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
        pid_line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    };

    assert!(
        is_process_alive(grandchild_pid),
        "grandchild process {grandchild_pid} should be alive before force kill"
    );

    assert_eq!(child.kill().await, Some(ChildExit::Killed));
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !is_process_alive(grandchild_pid),
        "grandchild process {grandchild_pid} should have been force killed"
    );
}

// Regression tests for the pre_exec/setsid -> process_group(0) migration.
//
// We replaced an unsafe pre_exec callback that called setsid() with tokio's
// safe process_group(0) API. These tests verify the critical invariants:
//
// 1. The child gets its own process group (PGID == child PID, not parent's)
// 2. Grandchildren inherit the child's process group
// 3. kill(-pgid, SIGINT) reaches both child and grandchild
// 4. The child is NOT a session leader (regression guard against setsid)

#[cfg(unix)]
#[tokio::test]
async fn test_child_has_own_process_group() {
    let script = find_script_dir().join_component("sleep_5_interruptable.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
        None,
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;
    let child_pgid = unsafe { libc::getpgid(child_pid) };
    let parent_pgid = unsafe { libc::getpgid(0) };

    // process_group(0) should make the child's PGID equal its own PID
    assert_eq!(
        child_pgid, child_pid,
        "child PGID ({child_pgid}) should equal child PID ({child_pid})"
    );

    // The child's process group must differ from the parent's
    assert_ne!(
        child_pgid, parent_pgid,
        "child PGID ({child_pgid}) must differ from parent PGID ({parent_pgid})"
    );

    child.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_grandchild_inherits_child_process_group() {
    let script = find_script_dir().join_component("spawn_child_sleep.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
        None,
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;

    // Read the grandchild PID from stdout
    let grandchild_pid = {
        let mut out = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                let mut buf = vec![0u8; 256];
                let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                    .await
                    .expect("timed out reading grandchild PID")
                    .expect("failed to read stdout");
                out.extend_from_slice(&buf[..n]);
            }
            ChildOutput::Pty(_) => unreachable!("test uses non-PTY mode"),
        };
        let output = String::from_utf8(out).unwrap();
        let pid_line = output
            .lines()
            .find(|line| line.contains("CHILD_PID="))
            .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
        pid_line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .parse::<libc::pid_t>()
            .unwrap()
    };

    let child_pgid = unsafe { libc::getpgid(child_pid) };
    let grandchild_pgid = unsafe { libc::getpgid(grandchild_pid) };

    // Grandchild should be in the same process group as the child
    assert_eq!(
        grandchild_pgid, child_pgid,
        "grandchild PGID ({grandchild_pgid}) should match child PGID ({child_pgid})"
    );

    // Both should use child_pid as the group ID
    assert_eq!(
        child_pgid, child_pid,
        "process group ID ({child_pgid}) should equal child PID ({child_pid})"
    );

    child.stop().await;
    // Give OS time to clean up
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn test_pty_graceful_shutdown_signals_process_group() {
    let marker_file = std::env::temp_dir().join(format!(
        "turbo-pty-process-group-sigint-{}",
        std::process::id()
    ));
    let ready_file = std::env::temp_dir().join(format!(
        "turbo-pty-process-group-ready-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&marker_file);
    let _ = fs::remove_file(&ready_file);
    let marker_file = marker_file.to_string_lossy().into_owned();
    let ready_file = ready_file.to_string_lossy().into_owned();

    let script = r#"
const { spawn } = require("child_process");
process.on("SIGINT", () => {});
const child = spawn(process.execPath, [
  "-e",
  "process.on('SIGINT', () => { require('fs').writeFileSync(process.argv[1], 'interrupted'); process.exit(0); }); require('fs').writeFileSync(process.argv[2], 'ready'); setInterval(() => {}, 1000);",
  process.argv[1],
  process.argv[2],
], { stdio: "inherit" });
child.on("exit", () => process.exit(0));
"#;
    let mut cmd = Command::new("node");
    cmd.args(["-e", script, marker_file.as_str(), ready_file.as_str()]);
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_secs(2))),
        Some(PtySize::default()),
    )
    .unwrap();

    for _ in 0..50 {
        if fs::metadata(&ready_file).is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        fs::metadata(&ready_file).is_ok(),
        "node child should become ready before shutdown"
    );

    let exit = child.stop().await;

    assert_eq!(exit, Some(ChildExit::Interrupted));
    assert!(
        fs::metadata(&marker_file).is_ok(),
        "node child should receive SIGINT from PTY process-group shutdown"
    );

    let _ = fs::remove_file(marker_file);
    let _ = fs::remove_file(ready_file);
}

#[cfg(unix)]
#[tokio::test]
async fn test_sigint_to_process_group_reaches_grandchild() {
    let script = find_script_dir().join_component("spawn_child_sleep.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    cmd.open_stdin();
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(2000))),
        None,
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let child_pid = child.pid().expect("child should have a pid");

    // Read the grandchild PID
    let grandchild_pid = {
        let mut out = Vec::new();
        match child.outputs().unwrap() {
            ChildOutput::Std { mut stdout, .. } => {
                let mut buf = vec![0u8; 256];
                let n = tokio::time::timeout(Duration::from_secs(5), stdout.read(&mut buf))
                    .await
                    .expect("timed out reading grandchild PID")
                    .expect("failed to read stdout");
                out.extend_from_slice(&buf[..n]);
            }
            ChildOutput::Pty(_) => unreachable!("test uses non-PTY mode"),
        };
        let output = String::from_utf8(out).unwrap();
        let pid_line = output
            .lines()
            .find(|line| line.contains("CHILD_PID="))
            .unwrap_or_else(|| panic!("CHILD_PID not found in output: {output}"));
        pid_line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap()
    };

    assert!(
        is_process_alive(grandchild_pid),
        "grandchild should be alive before signal"
    );

    // Send SIGINT to the process group (negative PID), exactly as
    // ShutdownStyle::Graceful does in production code
    let pgid = -(child_pid as i32);
    unsafe {
        libc::kill(pgid, libc::SIGINT);
    }

    // Wait for processes to die
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        !is_process_alive(grandchild_pid),
        "grandchild should be dead after SIGINT to process group"
    );

    // Consume the exit
    child.wait().await;
}

// Guard against accidentally reverting to setsid(). With process_group(0),
// the child calls setpgid(0, 0) which creates a new process group but does
// NOT create a new session. If someone reintroduces setsid(), the child's
// SID would equal its PID. With setpgid, the SID is inherited from the
// parent.
#[cfg(unix)]
#[tokio::test]
async fn test_child_is_not_session_leader() {
    let script = find_script_dir().join_component("sleep_5_interruptable.js");
    let mut cmd = Command::new("node");
    cmd.args([script.as_std_path()]);
    let mut child = Child::spawn(
        cmd,
        ShutdownStyle::Graceful(Some(Duration::from_millis(500))),
        None,
    )
    .unwrap();

    tokio::time::sleep(STARTUP_DELAY).await;

    let child_pid = child.pid().expect("child should have a pid") as libc::pid_t;
    let child_sid = unsafe { libc::getsid(child_pid) };
    let parent_sid = unsafe { libc::getsid(0) };

    // With process_group(0), the child inherits the parent's session.
    // If setsid() were used instead, child_sid would equal child_pid.
    assert_ne!(
        child_sid, child_pid,
        "child SID ({child_sid}) should NOT equal child PID ({child_pid}) — that would mean \
         setsid() was called"
    );
    assert_eq!(
        child_sid, parent_sid,
        "child SID ({child_sid}) should equal parent SID ({parent_sid})"
    );

    child.stop().await;
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            // Process handle opened — check if it's actually still running
            let mut exit_code: u32 = 0;
            let result =
                windows_sys::Win32::System::Threading::GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            // STILL_ACTIVE (259) means the process is still running
            result != 0 && exit_code == 259
        }
    }
}
