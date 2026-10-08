//! Final process termination after explicit CLI cleanup.

/// Exit with the CLI's status, preserving all 32 status bits on Windows.
///
/// Callers must finish child-process and signal cleanup, drain required
/// logging, and finish profilers before calling this function. Like
/// `std::process::exit`, this does not drop stack locals. On Windows it also
/// skips TLS destructors, CRT/atexit handlers, and DLL process-detach
/// callbacks: required cleanup must not depend on any of those callbacks.
pub fn exit(code: i32) -> ! {
    #[cfg(not(windows))]
    std::process::exit(code);

    #[cfg(windows)]
    {
        use std::io::{self, Write};

        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

        // Preserve the stdio flushing normally performed by Rust's exit cleanup.
        // As with that cleanup, output errors must not change the requested status.
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();

        // SAFETY: GetCurrentProcess returns a valid pseudo-handle with termination
        // access. The cast preserves the full DWORD bit pattern, including negative
        // i32 statuses. TerminateProcess deliberately skips DLL/TLS teardown:
        // ExitProcess kills other threads first, so a TLS destructor can deadlock
        // forever on a global mutex held by an already-killed worker.
        unsafe { TerminateProcess(GetCurrentProcess(), code as u32) };

        // Successful self-termination never returns. If it fails, do not fall back
        // to ExitProcess (including std::process::exit or normal main return).
        // Rust's Windows abort uses fast-fail, not DLL/TLS process-exit cleanup.
        std::process::abort();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{
        cell::RefCell,
        fs::{self, File},
        io::Write,
        process::{Child, Command, ExitStatus, Stdio},
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    const FIXTURE: &str = "process_exit::tests::exit_fixture";
    const CODE_ENV: &str = "TURBO_PROCESS_EXIT_TEST_CODE";
    const MARKER_ENV: &str = "TURBO_PROCESS_EXIT_TEST_MARKER";
    const STDERR: &[u8] = b"stderr-buffer-tail";
    static SHARED: Mutex<()> = Mutex::new(());
    static DROP_STARTED: AtomicBool = AtomicBool::new(false);

    struct Guard(File);

    impl Drop for Guard {
        fn drop(&mut self) {
            DROP_STARTED.store(true, Ordering::Release);
            // The file is already open: no TLS initialization, allocation, or
            // unrelated locks in this destructor. A nonempty marker proves entry.
            let _ = self.0.write_all(b"TLS destructor entered");
            let _held = SHARED.lock().unwrap();
        }
    }

    thread_local! {
        static GUARD: RefCell<Option<Guard>> = const { RefCell::new(None) };
    }

    fn payload() -> Vec<u8> {
        let mut bytes: Vec<_> = (0..1024 * 1024).map(|i| b'a' + (i % 26) as u8).collect();
        bytes.extend_from_slice(b"stdout-buffer-tail");
        bytes
    }

    // Only the parent below may invoke this subprocess fixture. No production
    // flags, extra release binaries, or grandchildren are needed.
    #[test]
    #[ignore = "subprocess fixture for windows_exit_preserves_status_and_output"]
    fn exit_fixture() {
        let Ok(code) = std::env::var(CODE_ENV) else {
            return;
        };
        let code: u32 = code.parse().unwrap();
        let marker = File::create(std::env::var_os(MARKER_ENV).unwrap()).unwrap();
        GUARD.with(|guard| *guard.borrow_mut() = Some(Guard(marker)));

        let (ready_tx, ready_rx) = mpsc::sync_channel(0);
        thread::spawn(move || {
            let held = SHARED.lock().unwrap();
            ready_tx.send(()).unwrap();
            // If TLS cleanup ran while workers were alive this would release
            // the lock. ExitProcess kills this worker before invoking TLS Drop.
            while !DROP_STARTED.load(Ordering::Acquire) {
                thread::yield_now();
            }
            drop(held);
        });
        ready_rx.recv().unwrap();

        let bytes = payload();
        let (bulk, tail) = bytes.split_at(1024 * 1024);
        {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(bulk).unwrap();
            // A separate small, newline-free write stays in stdout's buffer;
            // a single oversized write could bypass that buffer altogether.
            stdout.write_all(tail).unwrap();
        }
        std::io::stderr().lock().write_all(STDERR).unwrap();
        // Intentionally no explicit flush here: test the same public helper
        // used by the CLI, including its final buffered stdout tail.
        super::exit(code as i32);
    }

    struct OwnedChild(Child);

    impl OwnedChild {
        fn wait_bounded(&mut self) -> ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(status) = self.0.try_wait().unwrap() {
                    return status;
                }
                assert!(Instant::now() < deadline, "Windows exit fixture hung");
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            // Also reap on assertion/IO failure; never leave an owned fixture
            // spinning or hung in DLL teardown after the test has failed.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn windows_exit_preserves_status_and_output() {
        let executable = std::env::current_exe().unwrap();
        let expected = payload();
        for code in [0_u32, 1, 255, 256, 0x1234_5678, 0x8000_0005, u32::MAX] {
            let temp = tempfile::tempdir().unwrap();
            let stdout_path = temp.path().join("stdout");
            let stderr_path = temp.path().join("stderr");
            let marker_path = temp.path().join("tls-drop");
            let mut child = OwnedChild(
                Command::new(&executable)
                    .args(["--exact", FIXTURE, "--ignored", "--nocapture", "--quiet"])
                    .env(CODE_ENV, code.to_string())
                    .env(MARKER_ENV, &marker_path)
                    .stdin(Stdio::null())
                    // Files avoid blocking on pipe capacity while the parent
                    // waits, including for the 1 MiB stdout payload.
                    .stdout(File::create(&stdout_path).unwrap())
                    .stderr(File::create(&stderr_path).unwrap())
                    .spawn()
                    .unwrap(),
            );
            let status = child.wait_bounded();
            // Windows ExitStatus::code exposes the DWORD as i32; cast back
            // rather than truncating to a byte or rejecting high-bit statuses.
            assert_eq!(status.code().map(|actual| actual as u32), Some(code));
            let stdout = fs::read(&stdout_path).unwrap();
            // libtest emits a short preamble before entering the fixture.
            // The complete payload and unflushed tail must remain byte-exact.
            assert!(
                stdout.ends_with(&expected),
                "stdout corrupted for {code:#x}"
            );
            assert!(stdout.len() <= expected.len() + 1024);
            assert_eq!(fs::read(&stderr_path).unwrap(), STDERR);
            assert!(fs::read(&marker_path).unwrap().is_empty(), "TLS Drop ran");
        }
    }
}
