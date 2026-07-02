use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

// The atomic covers `cargo test`'s in-process parallelism; flock covers
// nextest's process-per-test parallelism.
#[cfg(test)]
static PTY_TEST_LOCK: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
#[derive(Debug)]
pub(super) struct PtyTestGuard {
    #[cfg(unix)]
    file: std::fs::File,
}

#[cfg(test)]
impl PtyTestGuard {
    pub(super) fn acquire() -> Self {
        while PTY_TEST_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::sleep(Duration::from_millis(10));
        }

        #[cfg(unix)]
        {
            use std::{fs::OpenOptions, os::fd::AsRawFd};

            let path = std::env::temp_dir().join("turborepo-process-pty.lock");
            let file = match OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)
            {
                Ok(file) => file,
                Err(err) => {
                    PTY_TEST_LOCK.store(false, Ordering::Release);
                    panic!("failed to open PTY test lock: {err}");
                }
            };
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                PTY_TEST_LOCK.store(false, Ordering::Release);
                panic!(
                    "failed to lock PTY test lock: {}",
                    io::Error::last_os_error()
                );
            }
            Self { file }
        }

        #[cfg(not(unix))]
        {
            Self {}
        }
    }
}

#[cfg(all(test, unix))]
impl Drop for PtyTestGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;

        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        PTY_TEST_LOCK.store(false, Ordering::Release);
    }
}

#[cfg(all(test, not(unix)))]
impl Drop for PtyTestGuard {
    fn drop(&mut self) {
        PTY_TEST_LOCK.store(false, Ordering::Release);
    }
}
