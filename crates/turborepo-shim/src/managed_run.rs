//! Capability-based selection only. Root metadata and run dispatch are
//! consumers.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{
    io::Read,
    os::fd::AsRawFd,
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

use miette::Diagnostic;
use thiserror::Error;

use crate::capabilities::{Capabilities, Requirement};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::capabilities::{MAX_RESPONSE_BYTES, QUERY_FLAG};

const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Global,
    Local(PathBuf),
}

#[derive(Debug, Error, Diagnostic)]
#[error("{cli} cannot run {requirement}: {reason}")]
#[diagnostic(help(
    "Upgrade the repository's turbo dependency to a version that supports these managed tools, \
     then reinstall dependencies. An installed repository-local turbo is never replaced by the \
     global CLI or downloaded implicitly."
))]
pub struct Error {
    cli: String,
    local: Option<PathBuf>,
    requirement: Requirement,
    reason: String,
}

impl Error {
    fn new(requirement: &Requirement, local: Option<&Path>, reason: String) -> Self {
        Self {
            cli: match local {
                Some(path) => format!("repository-local turbo at {}", path.display()),
                None => "global CLI".to_owned(),
            },
            local: local.map(Path::to_owned),
            requirement: requirement.clone(),
            reason,
        }
    }

    /// Names a pin by the version its package declares, as users know it.
    pub fn declared(mut self, version: &str) -> Self {
        if let Some(path) = &self.local {
            self.cli = format!("repository-local turbo {version} at {}", path.display());
        }
        self
    }
}

/// Caller must first validate the exact-root managed selection that
/// `requirement` describes. This selector does not validate tools,
/// declarations or installations itself. `local` is an already resolved
/// native binary, not an npm launcher or runner. Only an actually
/// absent/uninstalled pin is `None`. Resolution failures for an installed pin
/// must not be converted to absence by the caller.
pub fn select(
    requirement: &Requirement,
    global: &Capabilities,
    local: Option<&Path>,
) -> Result<Selection, Error> {
    let Some(local) = local else {
        return require(requirement, global, None).map(|()| Selection::Global);
    };
    let fail = |reason: String| Error::new(requirement, Some(local), reason);
    let binary = fs::canonicalize(local)
        .map_err(|err| fail(format!("its native binary cannot be resolved: {err}")))?;
    // Self and copied peers must meet the same contract, not merely parse locks.
    let is_self = std::env::current_exe()
        .ok()
        .and_then(|p| fs::canonicalize(p).ok())
        == Some(binary.clone());
    let peer = if is_self {
        global.clone()
    } else {
        probe(&binary, QUERY_TIMEOUT)
            .map_err(|reason| fail(format!("it does not report managed-run support ({reason})")))?
    };
    require(requirement, &peer, Some(local))?;
    Ok(Selection::Local(binary))
}

fn require(
    requirement: &Requirement,
    capabilities: &Capabilities,
    local: Option<&Path>,
) -> Result<(), Error> {
    match capabilities.shortfall(requirement) {
        None => Ok(()),
        Some(reason) => Err(Error::new(requirement, local, reason)),
    }
}

/// Keep the root unreaped until group cleanup: its PID cannot be recycled even
/// when it exits before descendants close the pipes.
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Query {
    child: Child,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for Query {
    fn drop(&mut self) {
        // We created this group with process_group(0). The unreaped root still
        // reserves its identity, so this cannot kill a recycled process group.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exited(child: &Child) -> Result<Option<bool>, &'static str> {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // WNOWAIT observes completion without releasing the PID/group identity.
    if unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err("capability process wait failed");
    }
    #[cfg(target_os = "macos")]
    let (pid, status) = (info.si_pid, info.si_status);
    #[cfg(target_os = "linux")]
    let (pid, status) = unsafe { (info.si_pid(), info.si_status()) };
    Ok((pid != 0).then_some(info.si_code == libc::CLD_EXITED && status == 0))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn probe(_binary: &Path, _timeout: Duration) -> Result<Capabilities, &'static str> {
    Err(
        "non-self native capability probing is not qualified on this platform; no pin \
         substitution is permitted",
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn probe(binary: &Path, timeout: Duration) -> Result<Capabilities, &'static str> {
    let mut command = Command::new(binary);
    command
        .arg(QUERY_FLAG)
        .current_dir(binary.parent().ok_or("native binary has no parent")?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Old CLIs may not implement the preflight. Suppress normal startup's
    // download/update/telemetry/agent paths without altering the real dispatch env.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("TURBO_")
            || name.starts_with("__TURBO_")
            || matches!(
                name.as_ref(),
                "AI_AGENT" | "CLAUDECODE" | "CODEX_THREAD_ID" | "NODE_OPTIONS" | "NODE_PATH"
            )
        {
            command.env_remove(key);
        }
    }
    command
        .env("DO_NOT_TRACK", "1")
        .env("TURBO_TELEMETRY_DISABLED", "1")
        .env("TURBO_NO_UPDATE_NOTIFIER", "1")
        .env("TURBO_GLOBAL_WARNING_DISABLED", "1")
        .env("TURBO_DOWNLOAD_LOCAL_ENABLED", "0");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|_| "capability query could not start")?;
    let mut query = Query { child };
    let mut stdout = query.child.stdout.take().ok_or("missing stdout pipe")?;
    let mut stderr = query.child.stderr.take().ok_or("missing stderr pipe")?;
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err("could not configure nonblocking capability pipes");
        }
    }
    let started = Instant::now();
    let mut status = None;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let (mut stdout_closed, mut stderr_closed) = (false, false);
    loop {
        if !stdout_closed {
            stdout_closed = drain(&mut stdout, &mut output, MAX_RESPONSE_BYTES + 1)?;
        }
        if output.len() > MAX_RESPONSE_BYTES {
            return Err("capability response exceeded its byte limit");
        }
        if !stderr_closed {
            stderr_closed = drain(&mut stderr, &mut errors, 1)?;
        }
        if !errors.is_empty() {
            return Err("capability query wrote unexpected stderr");
        }
        if status.is_none() {
            status = exited(&query.child)?;
        }
        if let Some(status) = status
            && stderr_closed
            && stdout_closed
        {
            if !status {
                return Err("capability query exited unsuccessfully");
            }
            return Capabilities::parse(&output)
                .map_err(|_| "capability response is malformed or uses an unsupported ABI");
        }
        if started.elapsed() >= timeout {
            return Err("capability query timed out");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn drain(reader: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> Result<bool, &'static str> {
    let mut buffer = [0; 512];
    while bytes.len() < limit {
        let count = buffer.len().min(limit - bytes.len());
        match reader.read(&mut buffer[..count]) {
            Ok(0) => return Ok(true),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(false);
            }
            Err(_) => return Err("capability output read failed"),
        }
    }
    Ok(false)
}
