use std::io;

use thiserror::Error;
use turborepo_api_client::Error as APIError;

#[derive(Debug, Error)]
pub enum Error {
    // For conversion from APIError
    #[error(transparent)]
    APIError(#[from] APIError),

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
    #[error("failed to find token for api: {api}")]
    FailedToFindTokenForAPI { api: String },

    // File write errors
    #[error("failed to write to auth file at {auth_path}: {error}")]
    FailedToWriteAuth {
        auth_path: turbopath::AbsoluteSystemPathBuf,
        error: io::Error,
    },

    #[error(transparent)]
    PathError(#[from] turbopath::PathError),

    // File conversion errors
    #[error("failed to convert config token to auth file: {0}")]
    FailedToConvertConfigTokenToAuthFile(turborepo_api_client::Error),
    #[error("failed to serialize auth file: {error}")]
    FailedToSerializeAuthFile { error: serde_json::Error },
}
