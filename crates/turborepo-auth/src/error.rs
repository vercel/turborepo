use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // HTTP / API errors
    #[error(
        "loginUrl is configured to \"{value}\", but cannot be a base URL. This happens in \
         situations like using a `data:` URL."
    )]
    LoginUrlCannotBeABase { value: String },
    #[error("failed to get token")]
    FailedToGetToken,
    #[error("failed to fetch user: {0}")]
    FailedToFetchUser(turborepo_api_client::Error),
    #[error("url is invalid: {0}")]
    InvalidUrl(#[from] url::ParseError),

    // SSO errors
    #[error("failed to validate sso token")]
    FailedToValidateSSOToken(turborepo_api_client::Error),
    #[error("failed to make sso token name")]
    FailedToMakeSSOTokenName(io::Error),

    // File read errors
    #[error("failed to find config directory")]
    FailedToFindConfigDir,
    #[error("failed to read config file: {0}")]
    FailedToReadConfigFile(io::Error),
    #[error("failed to read auth file: {0}")]
    FailedToReadAuthFile(io::Error),
}
