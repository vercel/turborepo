use thiserror::Error;
use turbopath::PathValidationError;

pub mod git;

#[derive(Debug, Error)]
pub enum Error {
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("repository not found")]
    RepositoryNotFound,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("path error: {0}")]
    Path(#[from] PathValidationError),
}
