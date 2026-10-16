//! How setup starts and waits for a JavaScript dependency installer. Unix
//! keeps the plain wait.
//!
//! On Windows the installer shares setup's console, so a console Ctrl+C
//! reaches it directly, once. Setup swallows the event only while it waits,
//! so it still restores protected lockfiles and returns the installer's own
//! status, as `turbo exec` does. The installer tree runs in a kill-on-close
//! job, so nothing it started outlives it or an early end of setup (the npm
//! wrapper's relayed Ctrl+C, or a supervisor's TerminateProcess). Batch
//! launchers start through std's `cmd.exe` escaping, which refuses
//! arguments it cannot pass on safely; `cmd.exe` would still expand `%` in
//! the launcher's own path, so such a path never starts.

use std::{
    io,
    process::{Command, ExitStatus},
};

#[cfg(not(windows))]
pub(super) fn status(command: &mut Command) -> io::Result<ExitStatus> {
    command.status()
}

#[cfg(windows)]
pub(super) fn status(command: &mut Command) -> io::Result<ExitStatus> {
    use std::os::windows::{io::AsRawHandle, process::CommandExt};

    use windows_sys::Win32::{
        Foundation::{BOOL, FALSE, TRUE},
        System::{
            Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
            Threading::CREATE_SUSPENDED,
        },
    };

    unsafe extern "system" fn swallow(event: u32) -> BOOL {
        BOOL::from(matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT))
    }
    struct Swallowing;
    impl Drop for Swallowing {
        fn drop(&mut self) {
            // SAFETY: removes the handler this guard registered.
            unsafe { SetConsoleCtrlHandler(Some(swallow), FALSE) };
        }
    }

    let program = command.get_program().to_string_lossy().to_ascii_lowercase();
    if (program.ends_with(".cmd") || program.ends_with(".bat")) && program.contains('%') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "cmd.exe would expand `%` in the managed installer's path; move the repository to a \
             path without `%`",
        ));
    }
    let job = turborepo_process::JobObject::new()?;
    // SAFETY: `swallow` is a valid handler for the life of the process.
    let _swallowing =
        (unsafe { SetConsoleCtrlHandler(Some(swallow), TRUE) } != 0).then_some(Swallowing);
    // Suspended until it is in the job, so no descendant can start outside it.
    let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
    // Like task processes, an installer whose job assignment Windows refuses
    // still runs; the shared console keeps delivering Ctrl+C to it.
    if let Err(error) = job.assign_suspended_process(child.as_raw_handle()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    // Dropping the job afterwards ends anything the installer left running.
    child.wait()
}

#[cfg(all(test, windows))]
#[path = "installer_windows_tests.rs"]
mod windows_tests;
