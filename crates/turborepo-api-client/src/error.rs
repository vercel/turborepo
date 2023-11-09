use std::backtrace::Backtrace;

use reqwest::header::ToStrError;
use thiserror::Error;

use crate::CachingStatus;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error making HTTP request: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("skipping HTTP Request, too many failures have occurred.\nLast error: {0}")]
    TooManyFailures(#[from] Box<reqwest::Error>),
    #[error("Unable to set up TLS.")]
    TlsError(#[source] reqwest::Error),
    #[error("Error parsing header: {0}")]
    InvalidHeader(#[from] ToStrError),
    #[error("Error parsing URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unknown caching status: {0}")]
    UnknownCachingStatus(String, #[backtrace] Backtrace),
    #[error("unknown status {code}: {message}")]
    UnknownStatus {
        code: String,
        message: String,
        #[backtrace]
        backtrace: Backtrace,
    },
    #[error("{message}")]
    CacheDisabled {
        status: CachingStatus,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
