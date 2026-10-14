//! `verify-system` tools: turbo.lock records the exact version and the
//! executables of an installed tool, and setup verifies the first matching
//! executable on PATH instead of installing it. Directories inside the
//! repository never count, so a repository cannot supply its own tool. Probes
//! run `<tool> --version` outside the repository with Corepack downloads
//! disabled; they never resolve, download, or install anything.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use semver::Version;

use crate::{
    lock::{self, Document, Installation, Lock, Snapshot, Tool, reconcile::Resolution},
    package_manager::Manager,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_BYTES: u64 = 4096;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not on PATH outside the repository; install it or use managed tools")]
    Missing(String),
    #[error("cannot read the version of {name} at {}", path.display())]
    Probe { name: String, path: PathBuf },
    #[error(
        "{name} at {} is {found}, but turbo.lock requires {expected}; install {expected}, or run \
         turbo setup --update-lock locally to lock the installed version", path.display()
    )]
    Mismatch {
        name: String,
        path: PathBuf,
        found: Version,
        expected: String,
    },
    #[error("installed {id} {version} does not satisfy its native declaration")]
    Unsatisfied { id: String, version: Version },
    #[error(
        "turbo.lock records {0} as a managed tool, but the setup policy selects verify-system; \
         run turbo setup --update-lock locally and commit turbo.lock"
    )]
    Managed(String),
    #[error("verify-system supports Node.js and pnpm without Corepack integrity pins")]
    Unsupported,
    #[error("verify-system requires turbo.lock; run turbo setup locally and commit turbo.lock")]
    MissingLock,
    #[error(
        "native declarations disagree with turbo.lock; run turbo setup locally and commit \
         turbo.lock"
    )]
    Drift,
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(transparent)]
    Lock(#[from] lock::Error),
}

/// A locked tool found on PATH at its exact locked version.
#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
    pub id: String,
    pub version: String,
    pub path: PathBuf,
}

/// The first executable `name` in `path` outside `repo`.
pub fn find(name: &str, path: Option<&OsStr>, repo: &Path) -> Option<PathBuf> {
    let repo = repo.canonicalize().ok()?;
    let names = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| {
            let (Some(dir), Ok(actual)) = (candidate.parent(), candidate.canonicalize()) else {
                return false;
            };
            dir.canonicalize().is_ok_and(|dir| !dir.starts_with(&repo))
                && !actual.starts_with(&repo)
                && executable(&actual)
        })
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    let runnable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let runnable = true;
    metadata.is_file() && runnable
}

/// Finds `name` and reads its `--version` output, such as `v24.0.0`.
pub fn probe(name: &str, path: Option<&OsStr>, repo: &Path) -> Result<(PathBuf, Version), Error> {
    let found = find(name, path, repo).ok_or_else(|| Error::Missing(name.to_owned()))?;
    let failed = || Error::Probe {
        name: name.to_owned(),
        path: found.clone(),
    };
    let mut child = Command::new(&found)
        .arg("--version")
        .current_dir(std::env::temp_dir())
        .env("COREPACK_ENABLE_NETWORK", "0")
        .env("COREPACK_ENABLE_DOWNLOAD_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failed())?;
    let mut stdout = child.stdout.take().ok_or_else(failed)?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        (&mut stdout)
            .take(MAX_OUTPUT_BYTES)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed());
            }
        }
    };
    let output = reader.join().ok().and_then(Result::ok).ok_or_else(failed)?;
    let version = String::from_utf8(output)
        .ok()
        .filter(|_| status.success())
        .and_then(|text| {
            let line = text.lines().next()?.trim();
            Version::parse(line.strip_prefix('v').unwrap_or(line)).ok()
        })
        .ok_or_else(failed)?;
    Ok((found, version))
}

/// Executables setup verifies for each supported tool.
fn executables(id: &str) -> Option<&'static str> {
    match id {
        "node" => Some("node"),
        "pnpm" => Some("pnpm"),
        _ => None,
    }
}

/// Verifies every lock entry as a `verify-system` tool at its exact version.
pub fn verify(lock: &Lock, path: Option<&OsStr>, repo: &Path) -> Result<Vec<Verified>, Error> {
    let mut verified = Vec::new();
    for (id, tool) in lock.tools() {
        let Installation::VerifySystem { executables: names } = &tool.installation else {
            return Err(Error::Managed(id.clone()));
        };
        if executables(id).is_none() || id != &tool.adapter {
            return Err(Error::Unsupported);
        }
        for name in names {
            let (found, version) = probe(name, path, repo)?;
            if version.to_string() != tool.version {
                return Err(Error::Mismatch {
                    name: name.clone(),
                    path: found,
                    found: version,
                    expected: tool.version.clone(),
                });
            }
            verified.push(Verified {
                id: id.clone(),
                version: tool.version.clone(),
                path: found,
            });
        }
    }
    Ok(verified)
}

/// Verifies the captured lock: unchanged native declarations, locked versions
/// that satisfy them, and installed tools at exactly those versions.
pub fn verify_locked(
    snapshot: &Snapshot,
    path: Option<&OsStr>,
    repo: &Path,
) -> Result<Vec<Verified>, Error> {
    let lock = snapshot.previous_lock().ok_or(Error::MissingLock)?;
    if !lock.matches_native(snapshot.declarations())? {
        return Err(Error::Drift);
    }
    for (id, tool) in lock.tools() {
        let version = Version::parse(&tool.version).map_err(|_| Error::Unsupported)?;
        if !satisfies(snapshot, id, &version)? {
            return Err(Error::Unsatisfied {
                id: id.clone(),
                version,
            });
        }
    }
    let verified = verify(lock, path, repo)?;
    snapshot.ensure_current()?;
    Ok(verified)
}

fn satisfies(snapshot: &Snapshot, id: &str, version: &Version) -> Result<bool, Error> {
    Ok(match id {
        "pnpm" => snapshot
            .package_manager()?
            .is_some_and(|declaration| declaration.matches(version)),
        _ => snapshot
            .node_requirements()?
            .matches_locked_version(version),
    })
}

/// Resolver for `lock::reconcile`: probes the installed tools for requested
/// IDs, checks them against native declarations, and preserves every other
/// selection. A preserved managed selection must be refreshed explicitly.
pub fn resolve(request: Resolution<'_>, path: Option<&OsStr>, repo: &Path) -> Result<Lock, Error> {
    let snapshot = request.snapshot();
    snapshot.ensure_current()?;
    let manager = snapshot.package_manager()?;
    if manager.as_ref().is_some_and(|declaration| {
        declaration.manager != Manager::Pnpm
            || declaration
                .package_manager
                .iter()
                .chain(&declaration.dev_engines)
                .any(|request| request.integrity.is_some())
    }) {
        return Err(Error::Unsupported);
    }
    let mut tools = BTreeMap::new();
    for (id, declarations) in snapshot.declarations() {
        let name = executables(id).ok_or(Error::Unsupported)?;
        if !request.version_ids().contains(id) {
            let previous = snapshot
                .previous_lock()
                .and_then(|lock| lock.tools().get(id))
                .ok_or(Error::Unsupported)?;
            if !matches!(previous.installation, Installation::VerifySystem { .. }) {
                return Err(Error::Managed(id.clone()));
            }
            tools.insert(id.clone(), previous.clone());
            continue;
        }
        let (_, version) = probe(name, path, repo)?;
        if !satisfies(snapshot, id, &version)? {
            return Err(Error::Unsatisfied {
                id: id.clone(),
                version,
            });
        }
        tools.insert(
            id.clone(),
            Tool {
                adapter: id.clone(),
                version: version.to_string(),
                declarations: declarations.clone(),
                options: BTreeMap::new(),
                installation: Installation::VerifySystem {
                    executables: vec![name.to_owned()],
                },
            },
        );
    }
    snapshot.ensure_current()?;
    Ok(Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools,
    })?)
}

#[cfg(all(test, unix))]
mod tests;
