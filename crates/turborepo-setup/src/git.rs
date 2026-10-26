//! The Git questions setup asks, behind one boundary. Production uses
//! [`GitCli`]; unit tests inject an in-memory fake instead of spawning `git`.
//! Callers keep their own policy: this module only reports Git's answers.

use std::{io, path::Path, process::Command};

/// Exit code and output of one Git query, with no interpretation applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// `None` when Git was terminated by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Reply {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

pub trait Git: Send + Sync {
    /// `git rev-parse --show-toplevel`, run in `dir`.
    fn toplevel(&self, dir: &Path) -> io::Result<Reply>;
}

/// Setup shells out to Git to find the worktree root. Explain a missing
/// executable instead of surfacing a bare "No such file or directory".
pub fn git_spawn_error(error: io::Error) -> io::Error {
    if error.kind() != io::ErrorKind::NotFound {
        return error;
    }
    io::Error::new(
        io::ErrorKind::NotFound,
        "could not run `git`: it was not found on PATH. turbo setup uses Git to find the \
         repository root; install Git or add it to PATH, then retry",
    )
}

/// The `git` executable on PATH. Inherited repository redirections are removed
/// so every query answers for the directory it runs in.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitCli;

impl GitCli {
    fn run(dir: &Path, arguments: &[&str]) -> io::Result<Reply> {
        let output = Command::new("git")
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args(arguments)
            .output()
            .map_err(git_spawn_error)?;
        Ok(Reply {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

impl Git for GitCli {
    fn toplevel(&self, dir: &Path) -> io::Result<Reply> {
        Self::run(dir, &["rev-parse", "--show-toplevel"])
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::{Failure, FakeGit, fake_repository};

#[cfg(any(test, feature = "test-support"))]
mod fake;

#[cfg(test)]
mod tests;
