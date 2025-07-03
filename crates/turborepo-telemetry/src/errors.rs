use std::backtrace::Backtrace;

use reqwest::header::ToStrError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error making HTTP request: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Skipping HTTP Request. Too many failures have occurred.\nLast error: {0}")]
    TooManyFailures(#[from] Box<reqwest::Error>),
    #[error("Unable to set up TLS.")]
    TlsError(#[source] reqwest::Error),
    #[error("Error parsing header: {0}")]
    InvalidHeader(#[from] ToStrError),
    #[error("Error parsing URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("Unknown status {code}: {message}")]
    UnknownStatus {
        code: String,
        message: String,
        #[backtrace]
        backtrace: Backtrace,
    },
    #[error("Error making retryable request: {0}")]
    RetryError(#[from] turborepo_api_client::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
