//! Locked uv workspace sync with the managed uv launcher: `uv sync --locked
//! --all-packages` only, never a build/test/lint or Turborepo task. The
//! committed `uv.lock` is required and read-only: uv rejects a stale lock, and
//! if anything still changes it, it is restored and reported. The launcher
//! pins the repo-managed interpreter. Repeating a sync is safe; it is not a
//! promised no-op.
//!
//! `turbo setup` runs this only when Python dependency setup is enabled
//! (never with `--tools-only`), after reporting the effective lifecycle
//! policy, with the managed activation environment.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::Path,
    process::{Command, Stdio},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("uv.lock is missing; run `uv lock` locally, commit uv.lock, and retry")]
    MissingLock,
    #[error("uv sync changed the committed uv.lock; it was restored. Run `uv lock` and commit it")]
    LockChanged,
    #[error(
        "`uv sync --locked` exited with status {0}; if uv.lock is stale, run `uv lock` locally \
         and commit it"
    )]
    Failed(i32),
    #[error("uv sync I/O failed")]
    Io(#[from] io::Error),
}

/// uv runs no install-time hook scripts; its only package code execution is
/// building source distributions that lack a compatible locked wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builds {
    /// uv's default: build source distributions with their PEP 517 backends.
    Native,
    /// `--no-build`: refuse source builds, so wheel-less packages fail.
    Skip,
}

pub struct UvSync<'a> {
    /// The managed `uv` launcher from the setup generation, not a PATH lookup.
    pub uv: &'a Path,
    pub root: &'a Path,
    /// Use only uv's cache; never contact package indexes.
    pub offline: bool,
    pub builds: Builds,
    /// Set over the inherited environment, such as the managed activation's
    /// `PATH` and uv variables.
    pub env: &'a BTreeMap<String, OsString>,
}

impl UvSync<'_> {
    /// The effective execution policy to show before running.
    pub fn disclosure(&self) -> &'static str {
        match self.builds {
            Builds::Native => {
                "uv installs locked wheels without running package code, but builds source \
                 distributions without a compatible wheel by running their build backends"
            }
            Builds::Skip => "uv refuses source builds (--no-build); wheel-less packages fail",
        }
    }

    pub fn args(&self) -> Vec<&'static str> {
        let mut args = vec!["sync", "--locked", "--all-packages"];
        if self.offline {
            args.push("--offline");
        }
        if self.builds == Builds::Skip {
            args.push("--no-build");
        }
        args
    }

    pub fn run(&self) -> Result<(), Error> {
        let root = self.root.canonicalize()?;
        let lock = root.join("uv.lock");
        let before = match fs::read(&lock) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Error::MissingLock);
            }
            Err(error) => return Err(error.into()),
        };
        // Inherit the caller's environment and output so native index
        // credentials, settings and uv's own diagnostics keep applying; the
        // launcher owns interpreter selection.
        let status = Command::new(self.uv)
            .current_dir(&root)
            .args(self.args())
            .envs(self.env)
            .stdin(Stdio::null())
            .status();
        let changed = fs::read(&lock).ok().as_deref() != Some(&before[..]);
        if changed {
            fs::write(&lock, &before)?;
        }
        let status = status?;
        if changed {
            return Err(Error::LockChanged);
        }
        if !status.success() {
            return Err(Error::Failed(status.code().unwrap_or(-1)));
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests;
