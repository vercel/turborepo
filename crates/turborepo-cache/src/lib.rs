pub mod http;
mod signature_authentication;
#[cfg(test)]
mod signature_authentication_test_cases;

use thiserror::Error;

use crate::signature_authentication::SignatureError;

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
    #[error("artifact verification failed: {0}")]
    ApiClientError(#[from] turborepo_api_client::Error),
    #[error("signing artifact failed: {0}")]
    SignatureError(#[from] SignatureError),
}
