pub mod http;
mod signature_authentication;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Timestamp is invalid {0}")]
    Timestamp(#[from] std::num::TryFromIntError),
    #[error(
        "artifact verification failed: Downloaded artifact is missing required x-artifact-tag \
         header"
    )]
    ArtifactTagMissing,
    #[error("artifact verification failed: artifact tag does not match expected tag {0}")]
    InvalidTag(String),
    #[error("cannot untar file to {0}")]
    InvalidFilePath(String),
}
