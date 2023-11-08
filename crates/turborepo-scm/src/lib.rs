#![feature(error_generic_member_access)]
#![feature(io_error_more)]
#![feature(assert_matches)]
#![deny(clippy::all)]

use std::{
    backtrace::{self, Backtrace},
    io::Read,
    process::{Child, Command},
};

use bstr::io::BufReadExt;
use thiserror::Error;
use tracing::debug;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, PathError, RelativeUnixPathBuf};

pub mod git;
mod hash_object;
mod ls_tree;
pub mod manual;
pub mod package_deps;
mod status;

#[derive(Debug, Error)]
pub enum Error {
    #[error("git error on {1}: {0}")]
    Git2(
        #[source] git2::Error,
        String,
        #[backtrace] backtrace::Backtrace,
    ),
    #[error("git error: {0}")]
    Git(String, #[backtrace] backtrace::Backtrace),
    #[error(
        "{0} is not part of a git repository. git is required for operations based on source \
         control"
    )]
    GitRequired(AbsoluteSystemPathBuf),
    #[error(
        "git command failed due to unsupported git version. Upgrade to git 2.18 or newer: {0}"
    )]
    GitVersion(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error, #[backtrace] backtrace::Backtrace),
    #[error("path error: {0}")]
    Path(#[from] PathError, #[backtrace] backtrace::Backtrace),
    #[error("could not find git binary")]
    GitBinaryNotFound(#[from] which::Error),
    #[error("encoding error: {0}")]
    Encoding(
        #[from] std::string::FromUtf8Error,
        #[backtrace] backtrace::Backtrace,
    ),
    #[error("package traversal error: {0}")]
    Ignore(#[from] ignore::Error, #[backtrace] backtrace::Backtrace),
    #[error("invalid glob: {0}")]
    Glob(#[source] Box<wax::BuildError>, backtrace::Backtrace),
    #[error(transparent)]
    Walk(#[from] globwalk::WalkError),
}

impl From<wax::BuildError> for Error {
    fn from(value: wax::BuildError) -> Self {
        Error::Glob(Box::new(value), Backtrace::capture())
    }
}

impl Error {
    pub(crate) fn git_error(s: impl Into<String>) -> Self {
        Error::Git(s.into(), Backtrace::capture())
    }

    pub(crate) fn git2_error_context(error: git2::Error, error_context: String) -> Self {
        Error::Git2(error, error_context, Backtrace::capture())
    }
}

fn read_git_error_to_string<R: Read>(stderr: &mut R) -> Option<String> {
    let mut buf = String::new();
    let bytes_read = stderr.read_to_string(&mut buf).ok()?;
    if bytes_read > 0 {
        // something failed with git, report that error
        Some(buf)
    } else {
        None
    }
}

pub(crate) fn wait_for_success<R: Read, T>(
    mut child: Child,
    stderr: &mut R,
    command: &str,
    root_path: impl AsRef<AbsoluteSystemPath>,
    parse_result: Result<T, Error>,
) -> Result<T, Error> {
    let exit_status = child.wait()?;
    if exit_status.success() && parse_result.is_ok() {
        return parse_result;
    }
    let stderr_output = read_git_error_to_string(stderr);
    let stderr_text = stderr_output
        .map(|stderr| format!(" stderr: {}", stderr))
        .unwrap_or_default();
    if matches!(exit_status.code(), Some(129)) {
        return Err(Error::GitVersion(stderr_text));
    }
    let exit_text = if exit_status.success() {
        "".to_string()
    } else {
        let code = exit_status
            .code()
            .map(|code| code.to_string())
            .unwrap_or("unknown".to_string());
        format!(" exited with code {}", code)
    };
    let parse_error_text = if let Err(parse_error) = parse_result {
        format!(" had a parse error {}", parse_error)
    } else {
        "".to_string()
    };
    let path_text = root_path.as_ref();
    let err_text = format!(
        "'{}' in {}{}{}{}",
        command, path_text, parse_error_text, exit_text, stderr_text
    );
    Err(Error::Git(err_text, Backtrace::capture()))
}

#[derive(Debug)]
pub struct Git {
    root: AbsoluteSystemPathBuf,
    bin: AbsoluteSystemPathBuf,
}

#[derive(Debug, Error)]
enum GitError {
    #[error("failed to find git binary: {0}")]
    Binary(#[from] which::Error),
    #[error("failed to find .git folder for path {0}: {1}")]
    Root(AbsoluteSystemPathBuf, Error),
}

impl Git {
    fn find(path_in_repo: &AbsoluteSystemPath) -> Result<Self, GitError> {
        let bin = which::which("git")?;
        // If which produces an invalid absolute path, it's not an execution error, it's
        // a programming error. We expect it to always give us an absolute path
        // if it gives us any path. If that's not the case, we should crash.
        let bin = AbsoluteSystemPathBuf::try_from(bin.as_path()).unwrap_or_else(|_| {
            panic!(
                "which git produced an invalid absolute path {}",
                bin.display()
            )
        });
        let root =
            find_git_root(path_in_repo).map_err(|e| GitError::Root(path_in_repo.to_owned(), e))?;
        Ok(Self { root, bin })
    }
}

fn find_git_root(turbo_root: &AbsoluteSystemPath) -> Result<AbsoluteSystemPathBuf, Error> {
    let rev_parse = Command::new("git")
        .args(["rev-parse", "--show-cdup"])
        .current_dir(turbo_root)
        .output()?;
    if !rev_parse.status.success() {
        let stderr = String::from_utf8_lossy(&rev_parse.stderr);
        return Err(Error::git_error(format!(
            "git rev-parse --show-cdup error: {}",
            stderr
        )));
    }
    let cursor = std::io::Cursor::new(rev_parse.stdout);
    let mut lines = cursor.byte_lines();
    if let Some(line) = lines.next() {
        let line = String::from_utf8(line?)?;
        let tail = RelativeUnixPathBuf::new(line)?;
        turbo_root.join_unix_path(tail).map_err(|e| e.into())
    } else {
        let stderr = String::from_utf8_lossy(&rev_parse.stderr);
        Err(Error::git_error(format!(
            "git rev-parse --show-cdup error: no values on stdout. stderr: {}",
            stderr
        )))
    }
}

#[derive(Debug)]
pub enum SCM {
    Git(Git),
    Manual,
}

impl SCM {
    #[tracing::instrument]
    pub fn new(path_in_repo: &AbsoluteSystemPath) -> SCM {
        Git::find(path_in_repo).map(SCM::Git).unwrap_or_else(|e| {
            debug!("{}, continuing with manual hashing", e);
            SCM::Manual
        })
    }

    pub fn is_manual(&self) -> bool {
        matches!(self, SCM::Manual)
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, process::Command};

    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    use super::find_git_root;
    use crate::Error;

    fn tmp_dir() -> (tempfile::TempDir, AbsoluteSystemPathBuf) {
        let tmp_dir = tempfile::tempdir().unwrap();
        let dir = AbsoluteSystemPathBuf::try_from(tmp_dir.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        (tmp_dir, dir)
    }

    fn require_git_cmd(repo_root: &AbsoluteSystemPath, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(repo_root);
        assert!(cmd.output().unwrap().status.success());
    }

    fn setup_repository(repo_root: &AbsoluteSystemPath) {
        let cmds: &[&[&str]] = &[
            &["init", "."],
            &["config", "--local", "user.name", "test"],
            &["config", "--local", "user.email", "test@example.com"],
        ];
        for cmd in cmds {
            require_git_cmd(repo_root, cmd);
        }
    }

    #[test]
    fn test_symlinked_git_root() {
        let (_, tmp_root) = tmp_dir();
        let git_root = tmp_root.join_component("actual_repo");
        git_root.create_dir_all().unwrap();
        setup_repository(&git_root);
        git_root.join_component("inside").create_dir_all().unwrap();
        let link = tmp_root.join_component("link");
        link.symlink_to_dir("actual_repo").unwrap();
        let turbo_root = link.join_component("inside");
        let result = find_git_root(&turbo_root).unwrap();
        assert_eq!(result, link);
    }

    #[test]
    fn test_no_git_root() {
        let (_, tmp_root) = tmp_dir();
        tmp_root.create_dir_all().unwrap();
        let result = find_git_root(&tmp_root);
        assert_matches!(result, Err(Error::Git(_, _)));
    }
}
