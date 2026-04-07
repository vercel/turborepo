use std::io;

use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, PathError};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    SerdeError(#[from] serde_json::Error),
    #[error(transparent)]
    APIError(#[from] turborepo_api_client::Error),
    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),

    #[error(
        "`loginUrl` is configured to \"{value}\", but cannot be a base URL. This happens in \
         situations like using a `data:` URL."
    )]
    LoginUrlCannotBeABase { value: String },
    #[error("login callback listener failed: {0}")]
    CallbackListenerFailed(#[source] io::Error),
    #[error("login callback timed out waiting for browser redirect")]
    CallbackTimeout,
    #[error("login callback task panicked or was cancelled")]
    CallbackTaskFailed,
    #[error("CSRF state parameter mismatch on SSO redirect")]
    CsrfStateMismatch,
    #[error("login callback returned an error from the remote server")]
    LoginCallbackError,
    #[error("login callback redirect did not include a token")]
    TokenMissingFromCallback,
    #[error("token refresh failed with HTTP {status}")]
    TokenRefreshFailed { status: u16 },
    #[error("failed to fetch user: {0}")]
    FailedToFetchUser(#[source] turborepo_api_client::Error),
    #[error("url is invalid: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("failed to validate sso token")]
    FailedToValidateSSOToken(#[source] turborepo_api_client::Error),
    #[error("failed to make sso token name")]
    FailedToMakeSSOTokenName(#[source] io::Error),
    #[error("sso team cannot be empty for login")]
    EmptySSOTeam,
    #[error("sso team not found: {0}")]
    SSOTeamNotFound(String),
    #[error("sso token expired for team: {0}")]
    SSOTokenExpired(String),
    #[error("token not found")]
    TokenNotFound,
    #[error("'{path}' is an invalid token file: {source}")]
    InvalidTokenFileFormat {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("config directory not found")]
    ConfigDirNotFound,
    #[error("failed to read auth file path: {path}")]
    FailedToReadAuthFilePath {
        path: AbsoluteSystemPathBuf,
        error: io::Error,
    },

    #[error(transparent)]
    Path(#[from] PathError),

    #[error("OIDC discovery failed: {message}")]
    DiscoveryFailed { message: String },
    #[error("device authorization failed: {message}")]
    DeviceAuthorizationFailed { message: String },
    #[error("authorization timed out — the device code expired")]
    DeviceCodeExpired,
    #[error("authorization denied by user")]
    AuthorizationDenied,
    #[error("OAuth error: {code}{}", description.as_ref().map(|d| format!(": {d}")).unwrap_or_default())]
    OAuthError {
        code: String,
        description: Option<String>,
    },
    #[error("SSO requires an existing login. Please run `turbo login` first.")]
    SSORequiresLogin,
    #[error("token introspection failed: {message}")]
    IntrospectionFailed { message: String },
}
