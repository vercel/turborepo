// Windows Job Object wrapper for process tree cleanup.
//
// On Windows, killing a process does not cascade to its children. This is
// especially problematic with ConPTY, which spawns `conhost.exe` as a sibling
// process. By assigning each child to a Job Object configured with
// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, we ensure the entire process tree
// is terminated when the job handle is closed.

use std::{collections::HashSet, io, os::windows::io::RawHandle};

use tracing::debug;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32, Process32First, Process32Next,
            TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            GetProcessId, OpenProcess, OpenThread, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
            ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess,
        },
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
            let assign_error = (result == 0).then(io::Error::last_os_error);
            let close_result = CloseHandle(process_handle);

            if let Some(err) = assign_error {
                debug!("failed to assign process {pid} to job object: {err}");
                return Err(err);
            }
            if close_result == 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(())
        }
    }

    pub fn assign_suspended_process(&self, process_handle: RawHandle) -> io::Result<bool> {
        let process_handle = process_handle as HANDLE;

        let assigned = if unsafe { AssignProcessToJobObject(self.handle, process_handle) } == 0 {
            let err = io::Error::last_os_error();
            debug!("failed to assign suspended process to job object: {err}");
            false
        } else {
            true
        };

        resume_threads(process_handle)?;

        Ok(assigned)
    }

    pub fn active_processes(&self) -> io::Result<u32> {
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            let result = QueryInformationJobObject(
                self.handle,
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );

            if result == 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(info.ActiveProcesses)
        }
    }

    pub fn terminate(&self) -> io::Result<()> {
        unsafe {
            if TerminateJobObject(self.handle, 1) == 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(())
        }
    }
}

pub fn has_descendant_processes(root_pid: u32) -> io::Result<bool> {
    Ok(!descendant_processes(root_pid)?.is_empty())
}

pub fn terminate_descendant_processes(root_pid: u32) -> io::Result<()> {
    let mut first_error = None;
    let mut descendants = descendant_processes(root_pid)?;
    descendants.reverse();

    for pid in descendants {
        if let Err(err) = terminate_process(pid) {
            debug!("failed to terminate descendant process {pid}: {err}");
            first_error.get_or_insert(err);
        }
    }

    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

fn resume_threads(process_handle: HANDLE) -> io::Result<()> {
    let process_id = unsafe { GetProcessId(process_handle) };
    if process_id == 0 {
        return Err(io::Error::last_os_error());
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let result = resume_threads_from_snapshot(snapshot, process_id);
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

pub fn descendant_processes(root_pid: u32) -> io::Result<Vec<u32>> {
    let entries = process_entries()?;
    let mut visited = HashSet::from([root_pid]);
    let mut current_generation = vec![root_pid];
    let mut descendants = Vec::new();

    while !current_generation.is_empty() {
        let mut next_generation = Vec::new();
        for (pid, parent_pid) in &entries {
            if current_generation.contains(parent_pid) && visited.insert(*pid) {
                descendants.push(*pid);
                next_generation.push(*pid);
            }
        }
        current_generation = next_generation;
    }

    Ok(descendants)
}

fn process_entries() -> io::Result<Vec<(u32, u32)>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    let result = process_entries_from_snapshot(snapshot);
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

fn process_entries_from_snapshot(snapshot: HANDLE) -> io::Result<Vec<(u32, u32)>> {
    let mut entry = PROCESSENTRY32 {
        dwSize: std::mem::size_of::<PROCESSENTRY32>() as u32,
        cntUsage: 0,
        th32ProcessID: 0,
        th32DefaultHeapID: 0,
        th32ModuleID: 0,
        cntThreads: 0,
        th32ParentProcessID: 0,
        pcPriClassBase: 0,
        dwFlags: 0,
        szExeFile: [0; 260],
    };

    let mut entries = Vec::new();
    let mut has_entry = unsafe { Process32First(snapshot, &mut entry) } != 0;
    while has_entry {
        entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
        has_entry = unsafe { Process32Next(snapshot, &mut entry) } != 0;
    }

    Ok(entries)
}

fn terminate_process(pid: u32) -> io::Result<()> {
    let process_handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process_handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    let terminate_result = unsafe { TerminateProcess(process_handle, 1) };
    let terminate_error = (terminate_result == 0).then(io::Error::last_os_error);
    let close_result = unsafe { CloseHandle(process_handle) };

    if let Some(err) = terminate_error {
        return Err(err);
    }
    if close_result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn resume_threads_from_snapshot(snapshot: HANDLE, process_id: u32) -> io::Result<()> {
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        cntUsage: 0,
        th32ThreadID: 0,
        th32OwnerProcessID: 0,
        tpBasePri: 0,
        tpDeltaPri: 0,
        dwFlags: 0,
    };

    let mut found_thread = false;
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32OwnerProcessID == process_id {
            found_thread = true;
            resume_thread(entry.th32ThreadID)?;
        }

        has_entry = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }

    if found_thread {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no threads found for process {process_id}"),
        ))
    }
}

fn resume_thread(thread_id: u32) -> io::Result<()> {
    let thread_handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if thread_handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    let resume_result = unsafe { ResumeThread(thread_handle) };
    let resume_error = (resume_result == u32::MAX).then(io::Error::last_os_error);
    let close_result = unsafe { CloseHandle(thread_handle) };

    if let Some(err) = resume_error {
        return Err(err);
    }
    if close_result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}
