use std::backtrace::Backtrace;

use reqwest::header::ToStrError;
use thiserror::Error;

use crate::CachingStatus;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error reading from disk: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Error making HTTP request: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("skipping HTTP Request, too many failures have occurred.\nLast error: {0}")]
    TooManyFailures(#[from] Box<reqwest::Error>),
    #[error("Unable to set up TLS.")]
    TlsError(#[source] reqwest::Error),
    #[error("Error parsing header: {0}")]
    InvalidHeader(#[from] ToStrError),
    #[error("Error parsing '{url}' as URL: {err}")]
    InvalidUrl { url: String, err: url::ParseError },
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
    #[error("unable to parse '{text}' as JSON: {err}")]
    InvalidJson {
        err: serde_json::Error,
        text: String,
    },
    #[error(
        "[HTTP {status}] request to {url} returned \"{message}\" \ntry logging in again, or force \
         a new token (turbo login <--sso-team your_team> -f)."
    )]
    InvalidToken {
        status: u16,
        url: String,
        message: String,
    },
    #[error("[HTTP 403] token is forbidden from accessing {url}")]
    ForbiddenToken { url: String },
}

pub type Result<T> = std::result::Result<T, Error>;
