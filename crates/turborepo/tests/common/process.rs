use std::{
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> ExitStatus {
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

#[cfg(unix)]
pub mod unix {
    #[cfg(target_os = "linux")]
    use std::fs;
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use nix::{
        sys::signal::{self, Signal},
        unistd::{Pid, getpgid},
    };

    /// Owns a fixture's process group, including descendants holding output
    /// pipes. Never signal the test runner's own group, or a recycled group
    /// after teardown.
    pub struct TaskTreeGuard {
        pgid: Option<i32>,
    }

    impl TaskTreeGuard {
        pub fn new(task_pid: i32) -> Self {
            let own_group = getpgid(None).map(Pid::as_raw).unwrap_or(0);
            let pgid = getpgid(Some(Pid::from_raw(task_pid)))
                .ok()
                .map(Pid::as_raw)
                .filter(|pgid| *pgid > 0 && *pgid != own_group);
            Self { pgid }
        }

        pub fn disarm(&mut self) {
            self.pgid = None;
        }
    }

    impl Drop for TaskTreeGuard {
        fn drop(&mut self) {
            if let Some(pgid) = self.pgid.take() {
                let _ = signal::kill(Pid::from_raw(-pgid), Signal::SIGKILL);
            }
        }
    }

    pub fn process_exists(pid: i32) -> bool {
        let exists = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);

        if !exists {
            return false;
        }

        #[cfg(target_os = "linux")]
        if fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, fields)| fields.starts_with('Z'))
        }) {
            // Container init processes do not always reap killed descendants promptly.
            return false;
        }

        true
    }

    pub fn wait_for_process_gone(pid: i32, timeout: Duration) {
        let start = Instant::now();
        while process_exists(pid) {
            if start.elapsed() > timeout {
                panic!("timed out waiting for process {pid} to exit");
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
