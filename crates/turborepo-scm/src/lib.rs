use std::path::PathBuf;

use thiserror::Error;

pub mod git;

#[derive(Debug, Error)]
pub enum Error {
    #[error("non utf-8 path encountered: {0}")]
    NonUtf8Path(PathBuf),
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("repository not found")]
    RepositoryNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path error: {0}")]
    PathError(#[from] anyhow::Error),
    #[error("strip prefix error: {0}")]
    StripPrefixError(#[from] std::path::StripPrefixError),
}
