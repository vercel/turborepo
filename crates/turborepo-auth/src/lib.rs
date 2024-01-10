#![feature(cow_is_borrowed)]
#![feature(fs_try_exists)] // Used in tests
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth_file;
mod config_token;
mod error;
mod login;
mod login_server;
mod logout;
pub mod mocks;
mod sso;
mod sso_server;
mod ui;

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

pub use self::{
    auth_file::*, config_token::ConfigToken, error::Error, login::*, login_server::*, logout::*,
    sso::*, sso_server::*,
};

pub const TURBOREPO_AUTH_FILE_NAME: &str = "auth.json";
pub const TURBOREPO_LEGACY_AUTH_FILE_NAME: &str = "config.json";
pub const TURBOREPO_CONFIG_DIR: &str = "turborepo";
pub const VERCEL_CONFIG_DIR: &str = "com.vercel.cli";
pub const VERCEL_AUTH_FILE_NAME: &str = "auth.json";

pub const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
pub const DEFAULT_API_URL: &str = "https://vercel.com/api";

/// AuthSource determines where the auth file should be read from. Each of the
/// variants has different initialization and permissions.
pub enum AuthSource {
    /// A token passed in via the CLI. This is the most ephemeral of the auth
    /// sources. It will no-op on any and all fs write operations.
    CLI(String),
    /// Our custom auth file. This is allowed to read/write.
    Turborepo(AbsoluteSystemPathBuf),
    /// The Vercel auth file. This is a read-only source issued from the Vercel
    /// CLI. Write operations will no-op.
    Vercel(AbsoluteSystemPathBuf),
}

/// AuthProvider is an enum that contains either a token or a file. This is used
/// for holding a/many token(s), depending on the variant.
pub enum AuthProvider {
    Token(AuthToken),
    File(AuthFile),
}
impl AuthProvider {
    /// Creates a new Auth enum from an AuthSource.
    /// ## Arguments
    /// * `source`: The AuthSource to create the Auth enum from.
    /// ## Returns
    /// * `Auth`: The Auth enum.
    /// ## Examples
    /// ```
    /// use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
    /// use turborepo_auth::{AuthProvider, AuthSource};
    ///
    /// let auth_file_path = AbsoluteSystemPath::new("/path/to/auth/file").unwrap();
    /// let vercel_file_path = AbsoluteSystemPath::new("/path/to/vercel/file").unwrap();
    ///
    /// // Create an Auth enum from a Vercel file.
    /// let vercel_auth_file: AuthProvider = AuthProvider::new(AuthSource::Turborepo(vercel_file_path));
    /// // Create an Auth enum from a file.
    /// let turbo_auth_file: AuthProvider = AuthProvider::new(AuthSource::Turborepo(auth_file_path));
    /// // Create an Auth enum from a token.
    /// let auth_token: AuthProvider = AuthProvider::new(AuthSource::CLI("test-token".to_string()));
    ///
    /// assert!(turbo_auth_file.is_file());
    /// assert!(vercel_auth_file.is_file());
    /// assert!(!auth_token.is_file()
    /// ```
    pub fn new(source: AuthSource) -> Self {
        match source {
            // Any token coming in from the CLI is a one-off, so we don't give it any file checks,
            // api keys, or permissions. If we add functionality, like refreshing tokens
            // or anything that might allow for a passed in token to be written to disk,
            // we'll update this arm.
            AuthSource::CLI(t) => Self::Token(AuthToken {
                token: t,
                api: "".to_string(),
            }),
            AuthSource::Turborepo(source) => Self::File(AuthFile::new(source)),
            AuthSource::Vercel(source) => Self::File(AuthFile::new(source)),
        }
    }
    /// Determines if this enum is a file or a token.
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }
    /// Returns the underlying token. If the enum is a `Token`, it will return
    /// the token used to construct it. Otherwise, the `api` argument is used to
    /// look up the token in the file, if it exists.
    pub fn get_token(&self, api: &str) -> Option<AuthToken> {
        match self {
            Self::Token(t) => {
                if t.api == api {
                    Some(t.clone())
                } else {
                    None
                }
            }
            Self::File(f) => f.get_token(api),
        }
    }
}

/// Checks the auth file path first, then the config file path, and does the
/// following:
/// 1) If the auth file exists, read it and return the contents from it, if
///    possible. Otherwise return a FailedToReadAuthFile error.
/// 2) If the auth file does not exist, but the config file does, read it and
///    convert it to an auth file, then return the contents from it, if
///    possible. Otherwise return a FailedToReadConfigFile error.
/// 3) If neither file exists, return an empty auth file and write a blank one
///    to disk.
///
/// Note that we have a potential TOCTOU race condition in this function. If
/// this is invoked and the file we're trying to read is deleted after a
/// condition is met, we should simply error out on reading a file that no
/// longer exists.
pub fn read_or_create_auth_file(
    auth_file_path: &AbsoluteSystemPath,
    config_file_path: &AbsoluteSystemPath,
    api: &str,
) -> Result<AuthFile, Error> {
    if auth_file_path.try_exists()? {
        let content = auth_file_path
            .read_to_string()
            .map_err(|e| Error::FailedToReadAuthFile {
                source: e,
                path: auth_file_path.to_owned(),
            })?;
        let tokens: AuthFile = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeAuthFile { source: e })?;
        let mut auth_file = AuthFile::new(auth_file_path.to_owned());
        for (api, token) in tokens.tokens() {
            auth_file.insert(api.to_owned(), token.to_owned());
        }
        return Ok(auth_file);
    } else if config_file_path.try_exists()? {
        let content =
            config_file_path
                .read_to_string()
                .map_err(|e| Error::FailedToReadConfigFile {
                    source: e,
                    path: config_file_path.to_owned(),
                })?;
        let config_token: ConfigToken = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeConfigToken { source: e })?;

        let auth_token = convert_to_auth_token(&config_token.token, api);

        let mut auth_file = AuthFile::new(auth_file_path.to_owned());
        auth_file.insert(api.to_owned(), auth_token.token);
        auth_file.write_to_disk(auth_file_path)?;
        return Ok(auth_file);
    }

    // If neither file exists, return an empty auth file and write a blank one to
    // disk.
    let auth_file = AuthFile::default();
    auth_file.write_to_disk(auth_file_path)?;
    Ok(auth_file)
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use turborepo_api_client::Client;

    use super::*;
    use crate::mocks::MockApiClient;

    #[tokio::test]
    async fn test_read_or_create_auth_file_existing_auth_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let tempdir_path = tempdir.path().join(TURBOREPO_AUTH_FILE_NAME);
        let auth_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create auth file path");
        let config_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create config file path");

        // Create auth file
        let mut mock_auth_file = AuthFile::new(auth_file_path.to_owned());
        mock_auth_file.insert("mock-api".to_owned(), "mock-token".to_owned());
        mock_auth_file.write_to_disk(auth_file_path).unwrap();

        let client = MockApiClient::new();

        let result = read_or_create_auth_file(auth_file_path, config_file_path, client.base_url());

        assert!(result.is_ok());
        let auth_file = result.unwrap();
        assert_eq!(auth_file.tokens().len(), 1);
    }

    #[tokio::test]
    async fn test_read_or_create_auth_file_no_file_exists() {
        let tempdir = tempfile::tempdir().unwrap();
        let tempdir_path = tempdir.path().join(TURBOREPO_AUTH_FILE_NAME);
        let auth_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create auth file path");
        let config_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create config file path");

        let client = MockApiClient::new();
        let result = read_or_create_auth_file(auth_file_path, config_file_path, client.base_url());

        assert!(result.is_ok());
        assert!(std::fs::try_exists(auth_file_path).unwrap_or(false));
        assert!(result.unwrap().tokens().is_empty());
    }

    #[tokio::test]
    async fn test_read_or_create_auth_file_existing_config_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let tempdir_path = tempdir.path();
        let auth_file_path = tempdir_path.join(TURBOREPO_AUTH_FILE_NAME);
        let config_file_path = tempdir_path.join(TURBOREPO_LEGACY_AUTH_FILE_NAME);
        let full_auth_file_path = AbsoluteSystemPath::new(auth_file_path.to_str().unwrap())
            .expect("Failed to create auth file path");
        let full_config_file_path = AbsoluteSystemPath::new(config_file_path.to_str().unwrap())
            .expect("Failed to create config file path");

        // Create config file data
        let mock_config_file_data = serde_json::to_string(&ConfigToken {
            token: "mock-token".to_string(),
        })
        .unwrap();

        // Write config file data to system.
        let mut file = File::create(full_config_file_path).unwrap();
        file.write_all(mock_config_file_data.as_bytes()).unwrap();

        let client = MockApiClient::new();

        // Test: Get the result of reading the auth file
        let result = read_or_create_auth_file(
            full_auth_file_path,
            full_config_file_path,
            client.base_url(),
        );

        // Make sure no errors come back
        assert!(result.is_ok());
        // And then make sure the file was actually created on the fs
        assert!(std::fs::try_exists(full_auth_file_path).unwrap_or(false));

        // Then make sure the auth file contains the correct data
        let auth_file_check: AuthFile =
            serde_json::from_str(&full_auth_file_path.read_to_string().unwrap()).unwrap();

        let auth_file = result.unwrap();

        assert_eq!(auth_file_check, auth_file);
        assert_eq!(auth_file.tokens().len(), 1);
    }
}
