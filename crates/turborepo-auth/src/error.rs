use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    SerdeError(#[from] serde_json::Error),
    #[error(transparent)]
    APIError(#[from] turborepo_api_client::Error),

    #[error(
        "loginUrl is configured to \"{value}\", but cannot be a base URL. This happens in \
         situations like using a `data:` URL."
    )]
    LoginUrlCannotBeABase { value: String },
    #[error("failed to get token")]
    FailedToGetToken,
    #[error("failed to fetch user: {0}")]
    FailedToFetchUser(#[source] turborepo_api_client::Error),
    #[error("url is invalid: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("failed to validate sso token")]
    FailedToValidateSSOToken(#[source] turborepo_api_client::Error),
    #[error("failed to make sso token name")]
    FailedToMakeSSOTokenName(#[source] io::Error),
    #[error("config directory not found")]
    ConfigDirNotFound,
}
