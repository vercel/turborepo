// Windows Job Object wrapper for process tree cleanup.
//
// On Windows, killing a process does not cascade to its children. This is
// especially problematic with ConPTY, which spawns `conhost.exe` as a sibling
// process. By assigning each child to a Job Object configured with
// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, we ensure the entire process tree
// is terminated when the job handle is closed.

use std::io;

use tracing::debug;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    },
};

pub struct JobObject {
    handle: HANDLE,
}

// SAFETY: Job object handles can be sent between threads.
// The Windows API allows any thread to use a job object handle.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Create a new anonymous Job Object that will kill all assigned processes
    /// when the handle is closed.
    pub fn new() -> io::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let result = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );

            if result == 0 {
                let err = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(err);
            }

            Ok(Self { handle })
        }
    }

    /// Assign a process to this job object by its PID.
    ///
    /// Once assigned, the process (and any children it spawns after assignment)
    /// will be terminated when this `JobObject` is dropped.
    pub fn assign_pid(&self, pid: u32) -> io::Result<()> {
        unsafe {
            let process_handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process_handle.is_null() {
                let err = io::Error::last_os_error();
                debug!("failed to open process {pid} for job assignment: {err}");
                return Err(err);
            }

            let result = AssignProcessToJobObject(self.handle, process_handle);
            CloseHandle(process_handle);

            if result == 0 {
                let err = io::Error::last_os_error();
                debug!("failed to assign process {pid} to job object: {err}");
                return Err(err);
            }

            Ok(())
        }
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
