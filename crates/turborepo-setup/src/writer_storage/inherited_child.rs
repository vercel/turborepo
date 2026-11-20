//! An owned child holds the actual inherited descriptors until told to exec.
//! Only async-signal-safe syscalls run after fork; all parent I/O is bounded.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::{self, Read, Write},
    os::unix::{io::AsRawFd, net::UnixStream, process::CommandExt},
    process::{Child, Command},
    thread::{self, JoinHandle},
    time::Duration,
};

pub(super) struct InheritedChild {
    control: UnixStream,
    spawning: Option<JoinHandle<io::Result<Child>>>,
}

impl InheritedChild {
    pub(super) fn spawn(descriptors: &[i32]) -> Self {
        // SAFETY: the no-op callback cannot allocate or touch thread locks.
        let mut child = unsafe { Self::spawn_with(descriptors, || Ok(())) };
        child.assert_open();
        child
    }

    /// # Safety
    /// Teardown runs only in the fork copy, exactly once, before exec. It must
    /// not allocate, deallocate heap fields, panic, or touch process/thread
    /// locks. Any captured addresses must stay valid until the child
    /// acknowledges them.
    pub(super) unsafe fn spawn_with(
        descriptors: &[i32],
        teardown: impl Fn() -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        let (control, child_control) = UnixStream::pair().unwrap();
        control
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        control
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let fd = child_control.as_raw_fd();
        let descriptors = descriptors.to_vec();
        let mut command = Command::new("/usr/bin/true");
        // SAFETY: captured data is allocated before fork. After the caller's
        // allocation-free teardown, the child only uses poll/read/write/fcntl.
        unsafe {
            command.pre_exec(move || {
                teardown()?;
                loop {
                    let mut poll = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // Child watchdog also bounds parent cleanup after a panic.
                    if libc::poll(&mut poll, 1, 10_000) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
                    }
                    let mut byte = 0u8;
                    if libc::read(fd, (&mut byte as *mut u8).cast(), 1) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::EPIPE));
                    }
                    if byte == 0 {
                        return Ok(());
                    }
                    // Supplied fds were opened before fork, not installed in
                    // the child by this fixture. CLOEXEC has not run yet.
                    for descriptor in &descriptors {
                        if libc::fcntl(*descriptor, libc::F_GETFD) == -1 {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    if libc::write(fd, (&byte as *const u8).cast(), 1) != 1 {
                        return Err(io::Error::from_raw_os_error(libc::EPIPE));
                    }
                }
            });
        }
        let spawning = thread::spawn(move || {
            let result = command.spawn();
            drop(child_control);
            result
        });
        Self {
            control,
            spawning: Some(spawning),
        }
    }

    pub(super) fn assert_open(&mut self) {
        self.control.write_all(&[1]).unwrap();
        let mut reply = [0];
        self.control.read_exact(&mut reply).unwrap();
        assert_eq!(reply, [1], "child acknowledged retained owner descriptors");
    }
}

impl Drop for InheritedChild {
    fn drop(&mut self) {
        // Always release/reap our child, including on a regression assertion.
        let _ = self.control.write_all(&[0]);
        if let Some(spawning) = self.spawning.take() {
            let result = spawning.join().unwrap();
            match result {
                Ok(mut child) => assert!(child.wait().unwrap().success()),
                Err(error) if thread::panicking() => eprintln!("child cleanup: {error}"),
                Err(error) => panic!("owned child failed to spawn: {error}"),
            }
        }
    }
}
