use std::{backtrace::Backtrace, process::Command};

use turbopath::AbsoluteSystemPathBuf;

use crate::{Error, GitRepo};

pub enum CloneMode {
    /// Cloning locally, do a blobless clone (good UX and reasonably fast)
    Local,
    /// Cloning on CI, do a treeless clone (worse UX but fastest)
    CI,
}

// Provide a sane maximum depth for cloning. If a user needs more history than
// this, they can override or fetch it themselves.
const MAX_CLONE_DEPTH: usize = 64;

/// A wrapper around the git binary that is not tied to a specific repo.
pub struct Git {
    bin: AbsoluteSystemPathBuf,
}

impl Git {
    pub fn find() -> Result<Self, Error> {
        Ok(Self {
            bin: GitRepo::find_bin()?,
        })
    }

    pub fn spawn_git_command(
        &self,
        cwd: &AbsoluteSystemPathBuf,
        args: &[&str],
        pathspec: &str,
    ) -> Result<(), Error> {
        let mut command = Command::new(self.bin.as_std_path());
        command
            .args(args)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0");

        if !pathspec.is_empty() {
            command.arg("--").arg(pathspec);
        }

        let output = command.spawn()?.wait_with_output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(Error::Git(stderr, Backtrace::capture()))
        } else {
            Ok(())
        }
    }

    pub fn clone(
        &self,
        url: &str,
        cwd: AbsoluteSystemPathBuf,
        dir: Option<&str>,
        branch: Option<&str>,
        mode: CloneMode,
        depth: Option<usize>,
    ) -> Result<(), Error> {
        let depth = depth.unwrap_or(MAX_CLONE_DEPTH).to_string();
        let mut args = vec!["clone", "--depth", &depth];
        if let Some(branch) = branch {
            args.push("--branch");
            args.push(branch);
        }
        match mode {
            CloneMode::Local => {
                args.push("--filter=blob:none");
            }
            CloneMode::CI => {
                args.push("--filter=tree:0");
            }
        }
        args.push(url);
        if let Some(dir) = dir {
            args.push(dir);
        }

        self.spawn_git_command(&cwd, &args, "")?;

        Ok(())
    }
}
