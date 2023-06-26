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
pub mod manual;
pub mod package_deps;
mod status;

#[derive(Debug, Error)]
pub enum Error {
    #[error("git error on {1}: {0}")]
    Git2(git2::Error, String, #[backtrace] backtrace::Backtrace),
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
    #[error("package traversal error: {0}")]
    Ignore(#[from] ignore::Error, #[backtrace] backtrace::Backtrace),
    #[error("invalid glob: {0}")]
    Glob(Box<wax::BuildError>, backtrace::Backtrace),
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

    pub(crate) fn git2_error_context(error: git2::Error, context: String) -> Self {
        Error::Git2(error, context, Backtrace::capture())
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
