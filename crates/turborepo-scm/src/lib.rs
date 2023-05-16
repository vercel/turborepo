#![feature(error_generic_member_access)]
#![feature(provide_any)]
#![feature(assert_matches)]

use std::{
    backtrace::{self, Backtrace},
    io::Read,
    process::Child,
};

use thiserror::Error;
use turbopath::{AbsoluteSystemPath, PathError};

pub mod git;
mod hash_object;
mod ls_tree;
pub mod package_deps;
mod status;

#[derive(Debug, Error)]
pub enum Error {
    #[error("git error: {0}")]
    Git2(#[from] git2::Error, #[backtrace] backtrace::Backtrace),
    #[error("git error: {0}")]
    Git(String, #[backtrace] backtrace::Backtrace),
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
}

impl Error {
    pub(crate) fn git_error(s: impl Into<String>) -> Self {
        Error::Git(s.into(), Backtrace::capture())
    }

    fn from_git_exit_code(cmd: &str, path: &AbsoluteSystemPath, exit_code: Option<i32>) -> Error {
        let s = format!(
            "'{}' in {} exited with status code {}",
            cmd,
            path.to_string(),
            exit_code
                .map(|code| code.to_string())
                .unwrap_or("unknown".to_string())
        );
        Error::Git(s, Backtrace::capture())
    }
}

pub(crate) fn read_git_error<R: Read>(stderr: &mut R) -> Option<Error> {
    let mut buf = String::new();
    if let Ok(bytes_read) = stderr.read_to_string(&mut buf) {
        if bytes_read > 0 {
            // something failed with git, report that error
            Some(Error::git_error(buf))
        } else {
            None
        }
    } else {
        None
    }
}

pub(crate) fn wait_for_success<R: Read>(
    mut child: Child,
    stderr: &mut R,
    command: &str,
    root_path: impl AsRef<AbsoluteSystemPath>,
) -> Result<(), Error> {
    let exit_status = child.wait()?;
    if !exit_status.success() {
        Err(read_git_error(stderr).unwrap_or_else(|| {
            Error::from_git_exit_code(command, root_path.as_ref(), exit_status.code())
        }))
    } else {
        Ok(())
    }
}
