#![feature(error_generic_member_access)]
#![feature(provide_any)]
#![feature(assert_matches)]

use std::backtrace;

use thiserror::Error;
use turbopath::PathError;

pub mod git;

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
}
