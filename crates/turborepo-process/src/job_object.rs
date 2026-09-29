// Windows Job Object wrapper for process tree cleanup.
//
// On Windows, killing a process does not cascade to its children. This is
// especially problematic with ConPTY, which spawns `conhost.exe` as a sibling
// process. By assigning each child to a Job Object configured with
// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, we ensure the entire process tree
// is terminated when the job handle is closed.

use std::{
    collections::{HashMap, HashSet},
    fmt, io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
};

use tracing::debug;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DuplicateHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
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
            GetCurrentProcess, GetProcessId, GetProcessTimes, OpenProcess, OpenThread,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE, ResumeThread,
            THREAD_SUSPEND_RESUME, TerminateProcess, WaitForSingleObject,
        },
    },
};

use crate::process_tree::{ProcessEntry, ProcessTimes, descendants};

const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;

pub struct JobObject {
    handle: HANDLE,
}

/// A query-only handle retained from spawn so the root PID cannot be reused
/// while descendant cleanup is still pending.
pub(crate) struct ProcessIdentity {
    pid: u32,
    handle: OwnedHandle,
}

// OwnedHandle closes the handle on drop and is Send + Sync. Windows process
// handles are valid in every thread of the owning process.

impl fmt::Debug for ProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessIdentity")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl ProcessIdentity {
    pub(crate) fn duplicate_from(pid: u32, handle: RawHandle) -> io::Result<Self> {
        let current_process = unsafe { GetCurrentProcess() };
        let mut duplicate = std::ptr::null_mut();
        let result = unsafe {
            DuplicateHandle(
                current_process,
                handle as HANDLE,
                current_process,
                &mut duplicate,
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                0,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: DuplicateHandle returned a new owned process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(duplicate as RawHandle) };
        Ok(Self { pid, handle })
    }

    fn times(&self) -> io::Result<ProcessTimes> {
        process_times(self.handle.as_raw_handle() as HANDLE)
    }
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

pub fn has_descendant_processes(root: &ProcessIdentity) -> io::Result<bool> {
    Ok(!descendant_processes(root)?.0.is_empty())
}

pub fn terminate_descendant_processes(root: &ProcessIdentity) -> io::Result<()> {
    let mut first_error = None;
    let (mut descendant_pids, mut candidates) = descendant_processes(root)?;
    descendant_pids.reverse();

    for pid in descendant_pids {
        let Some(candidate) = candidates.remove(&pid) else {
            continue;
        };
        if let Err(err) = candidate.terminate() {
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

struct CandidateProcess {
    handle: OwnedHandle,
}

impl CandidateProcess {
    fn open(pid: u32) -> io::Result<Self> {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: OpenProcess returned a new owned process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) };
        Ok(Self { handle })
    }

    fn times(&self) -> io::Result<ProcessTimes> {
        process_times(self.handle.as_raw_handle() as HANDLE)
    }

    fn terminate(self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.handle.as_raw_handle() as HANDLE, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn descendant_processes(
    root: &ProcessIdentity,
) -> io::Result<(Vec<u32>, HashMap<u32, CandidateProcess>)> {
    let initial_entries = process_entries()?;
    let mut candidates = HashMap::new();

    for pid in candidate_pids(root.pid, &initial_entries) {
        match CandidateProcess::open(pid) {
            Ok(candidate) => {
                candidates.insert(pid, candidate);
            }
            Err(err) => debug!("failed to pin descendant candidate {pid}: {err}"),
        }
    }

    // Re-snapshot after candidates are pinned. Entries absent from the first
    // snapshot have no retained handle and cannot participate in traversal.
    let entries = process_entries()?
        .into_iter()
        .map(|(pid, parent_pid)| ProcessEntry {
            pid,
            parent_pid,
            times: candidates.get(&pid).and_then(|candidate| {
                candidate
                    .times()
                    .map_err(|err| debug!("failed to query descendant candidate {pid}: {err}"))
                    .ok()
            }),
        })
        .collect::<Vec<_>>();
    let descendant_pids = descendants(root.pid, root.times()?, &entries);

    Ok((descendant_pids, candidates))
}

fn candidate_pids(root_pid: u32, entries: &[(u32, u32)]) -> Vec<u32> {
    let mut visited = HashSet::from([root_pid]);
    let mut current_generation = vec![root_pid];
    let mut candidates = Vec::new();

    while !current_generation.is_empty() {
        let mut next_generation = Vec::new();
        for (pid, parent_pid) in entries {
            if current_generation.contains(parent_pid) && visited.insert(*pid) {
                candidates.push(*pid);
                next_generation.push(*pid);
            }
        }
        current_generation = next_generation;
    }

    candidates
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

fn process_times(handle: HANDLE) -> io::Result<ProcessTimes> {
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let exited = match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_OBJECT_0 => true,
        WAIT_TIMEOUT => false,
        _ => return Err(io::Error::last_os_error()),
    };
    if exited
        && unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
    {
        return Err(io::Error::last_os_error());
    }

    Ok(ProcessTimes {
        creation: filetime_to_u64(creation),
        exit: exited.then(|| filetime_to_u64(exit)),
    })
}

fn filetime_to_u64(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
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
