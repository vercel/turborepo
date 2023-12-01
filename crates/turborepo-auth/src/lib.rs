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

use std::collections::HashMap;

use turbopath::AbsoluteSystemPath;
use turborepo_api_client::Client;

pub use self::{
    auth_file::*, config_token::ConfigToken, error::Error, login::*, login_server::*, logout::*,
    sso::*, sso_server::*,
};

pub const TURBOREPO_AUTH_FILE_NAME: &str = "auth.json";
pub const TURBOREPO_LEGACY_AUTH_FILE_NAME: &str = "config.json";
pub const TURBOREPO_CONFIG_DIR: &str = "turborepo";

pub const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
pub const DEFAULT_API_URL: &str = "https://vercel.com/api";

/// Checks the auth file path first, then the config file path, and does the
/// following:
/// 1) If the auth file exists, read it and return the contents from it, if
///    possible. Otherwise return a FailedToReadAuthFile error.
/// 2) If the auth file does not exist, but the config file does, read it and
///    convert it to an auth file, then return the contents from it, if
///    possible. Otherwise return a FailedToReadConfigFile error.
/// 3) If neither file exists, return an empty auth file and write a blank one
///    to disk.
pub async fn read_or_create_auth_file(
    auth_file_path: &AbsoluteSystemPath,
    config_file_path: &AbsoluteSystemPath,
    client: &impl Client,
) -> Result<AuthFile, Error> {
    if auth_file_path.try_exists()? {
        let content = auth_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| Error::FailedToReadAuthFile {
                source: e,
                path: auth_file_path.to_owned(),
            })?;
        let tokens: HashMap<String, String> = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeAuthFile { source: e })?;
        let auth_file = AuthFile { tokens };
        return Ok(auth_file);
    } else if config_file_path.try_exists()? {
        let content = config_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| Error::FailedToReadConfigFile {
                source: e,
                path: config_file_path.to_owned(),
            })?;
        let config_token: ConfigToken = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeConfigToken { source: e })?;

        let auth_token = convert_to_auth_token(&config_token.token, client).await?;
        let auth_file = AuthFile {
            tokens: HashMap::from([(client.base_url().to_owned(), auth_token.token)]),
        };
        auth_file.write_to_disk(auth_file_path)?;
        return Ok(auth_file);
    }

    // If neither file exists, return an empty auth file and write a blank one to
    // disk.
    let auth_file = AuthFile {
        tokens: HashMap::new(),
    };
    auth_file.write_to_disk(auth_file_path)?;
    Ok(AuthFile {
        tokens: HashMap::new(),
    })
}

/// Promote a token with new scopes.
pub async fn promote(_token: &str, _client: &impl Client) {}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

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
        let mock_auth_file = AuthFile {
            tokens: HashMap::from([("mock-token".to_owned(), "mock-api".to_owned())]),
        };
        mock_auth_file.write_to_disk(auth_file_path).unwrap();

        let client = MockApiClient::new();

        let result = read_or_create_auth_file(auth_file_path, config_file_path, &client).await;

        assert!(result.is_ok());
        let auth_file = result.unwrap();
        assert_eq!(auth_file.tokens.len(), 1);
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
        let result = read_or_create_auth_file(auth_file_path, config_file_path, &client).await;

        assert!(result.is_ok());
        assert!(std::fs::try_exists(auth_file_path).is_ok_and(|b| b));
        assert!(result.unwrap().tokens.is_empty());
    }

    #[tokio::test]
    async fn test_read_or_create_auth_file_existing_config_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let tempdir_path = tempdir.path().join(TURBOREPO_AUTH_FILE_NAME);
        let auth_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create auth file path");
        let config_file_path = AbsoluteSystemPath::new(tempdir_path.to_str().unwrap())
            .expect("Failed to create config file path");

        // Create config file data
        let mock_config_file_data = serde_json::to_string(&ConfigToken {
            token: "mock-token".to_string(),
        })
        .unwrap();

        // Write config file data to system.
        let mut file = File::create(config_file_path).unwrap();
        file.write_all(mock_config_file_data.as_bytes()).unwrap();

        let client = MockApiClient::new();
        let result = read_or_create_auth_file(auth_file_path, config_file_path, &client).await;

        // Make sure no errors come back
        assert!(result.is_ok());
        // And then make sure the file was actually created on the fs
        assert!(std::fs::try_exists(auth_file_path).is_ok_and(|b| b));

        let auth_file = result.unwrap();
        assert_eq!(auth_file.tokens.len(), 1);
    }
}
