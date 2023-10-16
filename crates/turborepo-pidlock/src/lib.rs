#![deny(clippy::all)]
#![feature(assert_matches)]

use std::{
    convert::TryInto,
    fs,
    io::{self, Read, Write},
    num::TryFromIntError,
    path::PathBuf,
    process,
};

use log::warn;
use thiserror::Error;

/// Errors that may occur during the `Pidlock` lifetime.
#[derive(Debug, Error)]
pub enum PidlockError {
    /// A lock already exists
    #[error("lock exists at \"{0}\", please remove it")]
    LockExists(PathBuf),
    /// An operation was attempted in the wrong state, e.g. releasing before
    /// acquiring.
    #[error("invalid state")]
    InvalidState,
    /// The lock is already owned by a running process
    #[error("already owned")]
    AlreadyOwned,
    #[error("pid file error: {0}")]
    File(#[from] PidFileError),
}

/// Errors that can occur when dealing with the file
/// on disk.
#[derive(Debug, Error)]
pub enum PidFileError {
    #[error("Error reading pid file {1}: {0}")]
    IO(io::Error, String),
    #[error("Invalid pid {contents} in file {file}: {error}")]
    Invalid {
        error: String,
        contents: String,
        file: String,
    },
    #[error("Failed to remove stale pid file {1}: {0}")]
    FailedDelete(io::Error, String),
}

/// A result from a Pidlock operation
type PidlockResult = Result<(), PidlockError>;

/// States a Pidlock can be in during its lifetime.
#[derive(Debug, PartialEq)]
enum PidlockState {
    /// A new pidlock, unacquired
    New,
    /// A lock is acquired
    Acquired,
    /// A lock is released
    Released,
}

/// Check whether a process exists, used to determine whether a pid file is
/// stale.
///
/// # Safety
///
/// This function uses unsafe methods to determine process existence. The
/// function itself is private, and the input is validated prior to call.
fn process_exists(pid: i32) -> bool {
    #[cfg(target_os = "windows")]
    unsafe {
        // If GetExitCodeProcess returns STILL_ACTIVE, then the process
        // doesn't have an exit code (...or exited with code 259)
        use windows_sys::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION},
        };
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid as u32);
        let mut code = 0;
        GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        code == STILL_ACTIVE as u32
    }

    #[cfg(not(target_os = "windows"))]
    unsafe {
        // From the POSIX standard: If sig is 0 (the null signal), error checking
        // is performed but no signal is actually sent. The null signal can be
        // used to check the validity of pid.
        let result = libc::kill(pid, 0);
        result == 0
    }
}

/// A pid-centered lock. A lock is considered "acquired" when a file exists on
/// disk at the path specified, containing the process id of the locking
/// process.
pub struct Pidlock {
    /// The current process id
    pid: u32,
    /// A path to the lock file
    path: PathBuf,
    /// Current state of the Pidlock
    state: PidlockState,
}

impl Pidlock {
    /// Create a new Pidlock at the provided path.
    pub fn new(path: PathBuf) -> Self {
        Pidlock {
            pid: process::id(),
            path,
            state: PidlockState::New,
        }
    }

    /// Acquire a lock.
    pub fn acquire(&mut self) -> PidlockResult {
        match self.state {
            PidlockState::New => {}
            _ => {
                return Err(PidlockError::InvalidState);
            }
        }

        // acquiring something with a valid owner is an error
        if self.get_owner()?.is_some() {
            return Err(PidlockError::AlreadyOwned);
        }

        if let Some(p) = self.path.parent() {
            // even if this fails, the next call might not
            std::fs::create_dir_all(p).ok();
        }

        let mut file = match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(self.path.clone())
        {
            Ok(file) => file,
            Err(_) => {
                return Err(PidlockError::LockExists(self.path.clone()));
            }
        };
        file.write_all(&format!("{}", self.pid).into_bytes()[..])
            .unwrap();

        self.state = PidlockState::Acquired;
        Ok(())
    }

    /// Returns true when the lock is in an acquired state.
    pub fn locked(&self) -> bool {
        self.state == PidlockState::Acquired
    }

    /// Release the lock.
    fn release(&mut self) -> PidlockResult {
        match self.state {
            PidlockState::Acquired => {}
            _ => {
                return Err(PidlockError::InvalidState);
            }
        }

        fs::remove_file(self.path.clone()).unwrap();

        self.state = PidlockState::Released;
        Ok(())
    }

    /// Gets the owner of this lockfile, returning the pid. If the lock file
    /// doesn't exist, or the specified pid is not a valid process id on the
    /// system, it clears it.
    pub fn get_owner(&self) -> Result<Option<u32>, PidFileError> {
        let mut file = match fs::OpenOptions::new().read(true).open(self.path.clone()) {
            Ok(file) => file,
            Err(io_err) => {
                // If the file doesn't exist, there's no owner. If, on the
                // other hand, some other IO error occurred, we don't know
                // the situation and need to return an error
                if io_err.kind() == io::ErrorKind::NotFound {
                    return Ok(None);
                } else {
                    return Err(PidFileError::IO(io_err, self.path.display().to_string()));
                }
            }
        };

        let mut contents = String::new();
        if let Err(io_err) = file.read_to_string(&mut contents) {
            warn!("corrupted/invalid pid file at {:?}: {}", self.path, io_err);
            // Return an error, because None implies that we would succeed at
            // creating a pid file, but we won't. We require the file to not
            // exist if we're going to create it.
            // TODO: should we instead try deleting the file like with stale pids?
            return Err(PidFileError::IO(io_err, self.path.display().to_string()));
        }

        match contents.trim().parse::<i32>() {
            Ok(pid) if process_exists(pid) => {
                let pid: u32 =
                    pid.try_into()
                        .map_err(|e: TryFromIntError| PidFileError::Invalid {
                            error: e.to_string(),
                            contents,
                            file: self.path.display().to_string(),
                        })?;
                Ok(Some(pid))
            }
            Ok(_) => {
                warn!("stale pid file at {:?}", self.path);
                if let Err(e) = fs::remove_file(&self.path) {
                    Err(PidFileError::FailedDelete(
                        e,
                        self.path.display().to_string(),
                    ))
                } else {
                    Ok(None)
                }
            }
            Err(e) => Err(PidFileError::Invalid {
                error: e.to_string(),
                contents,
                file: self.path.display().to_string(),
            }),
        }
    }
}

impl Drop for Pidlock {
    fn drop(&mut self) {
        if self.locked() {
            self.release().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, fs, io::Write, path::PathBuf};

    use rand::{distributions::Alphanumeric, thread_rng, Rng};

    use super::{PidFileError, Pidlock, PidlockError, PidlockState};

    // This was removed from the library itself, but retained here
    // to assert backwards compatibility with std::process::id
    fn getpid() -> u32 {
        unsafe { libc::getpid() as u32 }
    }

    fn make_pid_path() -> (tempdir::TempDir, PathBuf) {
        let tmp = tempdir::TempDir::new("pidlock").unwrap();
        let path = tmp.path().join("pidfile");
        (tmp, path)
    }

    #[test]
    fn test_new() {
        let (_tmp, pid_path) = make_pid_path();
        let pidfile = Pidlock::new(pid_path.clone());

        assert_eq!(pidfile.pid, getpid());
        assert_eq!(pidfile.path, pid_path);
        assert_eq!(pidfile.state, PidlockState::New);
    }

    #[test]
    fn test_acquire_and_release() {
        let (_tmp, pid_path) = make_pid_path();
        let mut pidfile = Pidlock::new(pid_path);
        pidfile.acquire().unwrap();

        assert_eq!(pidfile.state, PidlockState::Acquired);

        pidfile.release().unwrap();

        assert_eq!(pidfile.state, PidlockState::Released);
    }

    #[test]
    fn test_acquire_lock_exists() {
        let (_tmp, pid_path) = make_pid_path();
        let mut orig_pidfile = Pidlock::new(pid_path);
        orig_pidfile.acquire().unwrap();

        let mut pidfile = Pidlock::new(orig_pidfile.path.clone());
        match pidfile.acquire() {
            Err(err) => {
                orig_pidfile.release().unwrap();
                assert_matches!(err, PidlockError::AlreadyOwned);
            }
            _ => {
                orig_pidfile.release().unwrap();
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_acquire_already_acquired() {
        let (_tmp, pid_path) = make_pid_path();
        let mut pidfile = Pidlock::new(pid_path);
        pidfile.acquire().unwrap();
        match pidfile.acquire() {
            Err(err) => {
                pidfile.release().unwrap();
                assert_matches!(err, PidlockError::InvalidState);
            }
            _ => {
                pidfile.release().unwrap();
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_release_bad_state() {
        let (_tmp, pid_path) = make_pid_path();
        let mut pidfile = Pidlock::new(pid_path);
        match pidfile.release() {
            Err(err) => {
                assert_matches!(err, PidlockError::InvalidState);
            }
            _ => {
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_locked() {
        let (_tmp, pid_path) = make_pid_path();
        let mut pidfile = Pidlock::new(pid_path);
        pidfile.acquire().unwrap();
        assert!(pidfile.locked());
    }

    #[test]
    fn test_locked_not_locked() {
        let (_tmp, pid_path) = make_pid_path();
        let pidfile = Pidlock::new(pid_path);
        assert!(!pidfile.locked());
    }

    #[test]
    fn test_stale_pid() {
        let (_tmp, path) = make_pid_path();
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path.clone())
            .expect("Could not open file for writing");

        file.write_all(&format!("{}", thread_rng().gen::<i32>()).into_bytes()[..])
            .unwrap();

        drop(file);

        // expect a stale pid file to be cleaned up
        let mut pidfile = Pidlock::new(path.clone());
        // We clear stale pid files when acquiring them, we expect this to succeed
        assert!(pidfile.acquire().is_ok());
    }

    #[test]
    fn test_stale_pid_invalid_contents() {
        let (_tmp, path) = make_pid_path();
        let contents: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect();
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path.clone())
            .expect("Could not open file for writing");

        file.write_all(&contents.into_bytes()).unwrap();

        drop(file);

        let mut pidfile = Pidlock::new(path.clone());
        // Contents are invalid
        assert_matches!(
            pidfile.acquire(),
            Err(PidlockError::File(PidFileError::Invalid { .. }))
        );
    }

    #[test]
    fn test_stale_pid_corrupted_contents() {
        let (_tmp, path) = make_pid_path();
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path.clone())
            .expect("Could not open file for writing");

        let invalid_utf8 = vec![0xff, 0xff, 0xff, 0xff];
        file.write_all(&invalid_utf8).unwrap();

        drop(file);

        let mut pidfile = Pidlock::new(path.clone());
        // We expect an IO error from trying to read as a utf8 string when it's just
        // bytes
        assert_matches!(
            pidfile.acquire(),
            Err(PidlockError::File(PidFileError::IO(..)))
        );
    }
}
