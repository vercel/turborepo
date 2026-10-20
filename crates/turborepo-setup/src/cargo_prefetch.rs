//! Locked Cargo dependency prefetch with the managed toolchain. `cargo fetch
//! --locked` only downloads: it never resolves, rewrites `Cargo.lock`, builds,
//! or runs build scripts, so there are no native lifecycle hooks to authorize.
//! `turbo setup` runs this when Rust dependency setup is enabled, with the
//! managed activation environment.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_LOCK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIAGNOSTIC_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Cargo dependency setup requires a committed Cargo.lock at the workspace root; run `cargo \
         generate-lockfile` locally and commit it"
    )]
    MissingLock,
    #[error(
        "Cargo.lock is out of date with the workspace manifests; run `cargo update --workspace` \
         locally and commit Cargo.lock"
    )]
    StaleLock,
    #[error(
        "cargo could not fetch the locked dependencies (exit {0}); check registry access, \
         credentials and network, or rerun without --offline"
    )]
    Fetch(String),
    #[error("cargo modified Cargo.lock during a locked fetch; restore it from version control")]
    LockChanged,
    #[error("unsafe private CARGO_HOME; remove {0} and rerun turbo setup")]
    UnsafeHome(PathBuf),
    #[error("Cargo dependency setup I/O failed")]
    Io(#[from] io::Error),
}

/// Where the clone's private Cargo home lives, without creating or checking
/// it. Activation exposes this same directory to tasks as `CARGO_HOME`.
pub fn cargo_home_path(root: &Path) -> PathBuf {
    // Split components: `/` is not a separator under a `\\?\` root.
    root.join(".turbo").join("cargo-home")
}

/// The repository's private Cargo home, beside managed tools rather than in
/// the user's `~/.cargo`, created if missing.
pub fn cargo_home(root: &Path) -> Result<PathBuf, Error> {
    let home = cargo_home_path(root);
    let mut builder = fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(&home) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e.into()),
        _ => {}
    }
    let metadata = fs::symlink_metadata(&home)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::UnsafeHome(home));
    }
    Ok(home)
}

fn read_lock(root: &Path) -> Result<Option<Vec<u8>>, Error> {
    let path = root.join("Cargo.lock");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(metadata) if !metadata.is_file() => return Err(Error::MissingLock),
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_LOCK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Fetch every locked workspace dependency into `cargo_home` using the
/// managed `cargo`, with `env` (such as the managed `PATH` and rustup
/// variables) over the inherited environment. Offline mode uses only what
/// that home already holds. Cargo's diagnostics are forwarded to stderr.
pub fn prefetch(
    root: &Path,
    cargo: &Path,
    cargo_home: &Path,
    offline: bool,
    env: &BTreeMap<String, OsString>,
) -> Result<(), Error> {
    let before = read_lock(root)?.ok_or(Error::MissingLock)?;
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args(["fetch", "--locked"])
        .args(offline.then_some("--offline"))
        .envs(env)
        .env("CARGO_HOME", cargo_home)
        .env_remove("CARGO_NET_OFFLINE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let mut diagnostics = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        (&mut stderr)
            .take(MAX_DIAGNOSTIC_BYTES)
            .read_to_end(&mut diagnostics)?;
        // Keep draining so a chatty cargo can never block on a full pipe.
        io::copy(&mut stderr, &mut io::sink())?;
    }
    let status = child.wait()?;
    io::stderr().write_all(&diagnostics)?;
    if read_lock(root)?.as_ref() != Some(&before) {
        return Err(Error::LockChanged);
    }
    if !status.success() {
        let text = String::from_utf8_lossy(&diagnostics);
        // Both cargo spellings: "needs to be updated but --locked was passed"
        // and "cannot update the lock file ... because --locked was passed".
        if text.contains("--locked was passed") {
            return Err(Error::StaleLock);
        }
        return Err(Error::Fetch(
            status
                .code()
                .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
        ));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, windows))]
mod windows_tests;
