//! Frozen JavaScript dependency installs with the managed package manager:
//! the manager's native frozen command only, never a build, test, or
//! Turborepo task. The committed lockfile is required and read-only. The
//! frozen command rejects a stale lock; if the manager still changes a
//! lockfile, setup restores the committed bytes and fails. Repeating an
//! install is safe but is not a promised no-op, so callers never skip it
//! because the lock is unchanged.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::Path,
    process::{Command, ExitStatus, Stdio},
};

use semver::Version;

use crate::package_manager::Manager;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "{manager} dependency setup requires a committed {expected} at the repository root; run \
         `{fix}` locally, commit the lockfile, and retry"
    )]
    MissingLock {
        manager: &'static str,
        expected: String,
        fix: &'static str,
    },
    #[error(
        "`{command}` changed the committed {changed}; it was restored. Run `{fix}` locally and \
         commit the result"
    )]
    LockChanged {
        command: String,
        changed: String,
        fix: &'static str,
    },
    #[error("JavaScript dependency setup I/O failed: {0}")]
    Io(#[from] io::Error),
}

/// A package manager with a frozen install adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frozen {
    manager: Manager,
    /// Every lockfile the manager reads, in its precedence order. All are
    /// protected, including absent ones, so it can neither rewrite nor
    /// create one.
    lockfiles: &'static [&'static str],
    regenerate: &'static str,
}

impl Frozen {
    /// `None` for a manager without an adapter yet.
    pub fn new(manager: Manager) -> Option<Self> {
        let (lockfiles, regenerate): (&'static [_], _) = match manager {
            Manager::Npm => (&["npm-shrinkwrap.json", "package-lock.json"], "npm install"),
            Manager::Pnpm => (&["pnpm-lock.yaml"], "pnpm install"),
            Manager::Yarn => (&["yarn.lock"], "yarn install"),
            // Bun reads its text lock first and can migrate the binary one.
            Manager::Bun => (&["bun.lock", "bun.lockb"], "bun install"),
            _ => return None,
        };
        Some(Self {
            manager,
            lockfiles,
            regenerate,
        })
    }

    pub fn manager(self) -> Manager {
        self.manager
    }

    /// The managed executable's name, which is also its turbo.lock tool ID.
    pub fn program(self) -> &'static str {
        self.manager.name()
    }

    /// The native frozen command's arguments for the locked version.
    pub fn args(self, version: &Version) -> &'static [&'static str] {
        match self.manager {
            // Installs exactly the lockfile and fails if package.json disagrees.
            Manager::Npm => &["ci"],
            // Installs the whole workspace and fails instead of updating the lock.
            Manager::Pnpm => &["install", "--frozen-lockfile"],
            // Yarn classic and modern Yarn spell the same refusal differently.
            Manager::Yarn if version.major < 2 => &["install", "--frozen-lockfile"],
            Manager::Yarn => &["install", "--immutable"],
            Manager::Bun => &["install", "--frozen-lockfile"],
            _ => unreachable!("Frozen::new admits only adapted managers"),
        }
    }

    /// The committed lockfile the frozen command will read. A missing lock is
    /// an error, never permission to create one.
    pub fn lockfile(self, root: &Path) -> Result<&'static str, Error> {
        self.lockfiles
            .iter()
            .copied()
            .find(|name| fs::metadata(root.join(name)).is_ok_and(|m| m.is_file()))
            .ok_or_else(|| Error::MissingLock {
                manager: self.program(),
                expected: self.lockfiles.join(" or "),
                fix: self.regenerate,
            })
    }

    /// Runs `program` (the managed executable) in `root` with `env` over the
    /// inherited environment, so native registry credentials and settings
    /// still apply. stdin is closed and output goes to the terminal. Returns
    /// the manager's exit status after checking every protected lockfile.
    pub fn run(
        self,
        program: &Path,
        args: &[&str],
        root: &Path,
        env: &BTreeMap<String, OsString>,
    ) -> Result<ExitStatus, Error> {
        let mut protected = Vec::new();
        for name in self.lockfiles {
            let path = root.join(name);
            let before = read(&path)?;
            protected.push((*name, path, before));
        }
        let status = Command::new(program)
            .current_dir(root)
            .args(args)
            .envs(env)
            .stdin(Stdio::null())
            .status();
        let mut changed = Vec::new();
        for (name, path, before) in &protected {
            if read(path)? != *before {
                match before {
                    Some(bytes) => fs::write(path, bytes)?,
                    None => fs::remove_file(path)?,
                }
                changed.push(*name);
            }
        }
        let status = status?;
        if !changed.is_empty() {
            let command = std::iter::once(self.program()).chain(args.iter().copied());
            return Err(Error::LockChanged {
                command: command.collect::<Vec<_>>().join(" "),
                changed: changed.join(", "),
                fix: self.regenerate,
            });
        }
        Ok(status)
    }
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(all(test, unix))]
mod tests;
