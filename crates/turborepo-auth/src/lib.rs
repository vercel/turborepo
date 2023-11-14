#![feature(cow_is_borrowed)]
#![feature(fs_try_exists)] // Used in tests
#![deny(clippy::all)]

mod auth_file;
mod config_token;
mod error;
mod login;
mod login_server;
mod logout;
// Make this publicly avaliable for testing in other crates.
pub mod mocks;
mod sso;
mod sso_server;
mod ui;

use turbopath::AbsoluteSystemPathBuf;
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
    auth_file_path: &AbsoluteSystemPathBuf,
    config_file_path: &AbsoluteSystemPathBuf,
    client: &impl Client,
) -> Result<AuthFile, Error> {
    if auth_file_path.exists() {
        let content = auth_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| Error::FailedToReadAuthFile {
                source: e,
                path: auth_file_path.clone(),
            })?;
        let auth_file: AuthFile = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeAuthFile { source: e })?;
        return Ok(auth_file);
    } else if config_file_path.exists() {
        let content = config_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| Error::FailedToReadConfigFile {
                source: e,
                path: config_file_path.clone(),
            })?;
        let config_token: ConfigToken = serde_json::from_str(&content)
            .map_err(|e| Error::FailedToDeserializeConfigToken { source: e })?;

        let auth_token = convert_to_auth_token(&config_token.token, client, None).await?;
        let auth_file = AuthFile {
            tokens: vec![auth_token],
        };
        auth_file.write_to_disk(auth_file_path)?;
        return Ok(auth_file);
    }

    // If neither file exists, return an empty auth file and write a blank one to
    // disk.
    let auth_file = AuthFile { tokens: Vec::new() };
    auth_file.write_to_disk(auth_file_path)?;
    Ok(AuthFile { tokens: Vec::new() })
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use turborepo_vercel_api::{Membership, Role, Space, User};

    use super::*;
    use crate::mocks::MockApiClient;

    #[tokio::test]
    async fn test_read_or_create_auth_file_existing_auth_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .expect("Failed to create auth file path");
        let config_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_LEGACY_AUTH_FILE_NAME))
                .expect("Failed to create config file path");

        // Create auth file
        let mock_auth_file = AuthFile {
            tokens: vec![AuthToken {
                token: "mock-token".to_string(),
                api: "mock-api".to_string(),
                created_at: Some(0),
                user: User {
                    id: 0.to_string(),
                    email: "mock-email".to_string(),
                    username: "mock-username".to_string(),
                    name: Some("mock-name".to_string()),
                    created_at: Some(0),
                },
                teams: vec![Team {
                    id: "team-id".to_string(),
                    spaces: vec![Space {
                        id: "space-id".to_string(),
                        name: "space1 name".to_string(),
                    }],
                    slug: "team1 slug".to_string(),
                    name: "maximum effort".to_string(),
                    created_at: 0,
                    created: chrono::Utc::now(),
                    membership: Membership::new(Role::Developer),
                }],
            }],
        };
        mock_auth_file.write_to_disk(&auth_file_path).unwrap();

        let client = MockApiClient::new();

        let result = read_or_create_auth_file(&auth_file_path, &config_file_path, &client).await;

        assert!(result.is_ok());
        let auth_file = result.unwrap();
        assert_eq!(auth_file.tokens.len(), 1);
    }

    #[tokio::test]
    async fn test_read_or_create_auth_file_no_file_exists() {
        let tempdir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .expect("Failed to create auth file path");
        let config_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_LEGACY_AUTH_FILE_NAME))
                .expect("Failed to create config file path");

        let client = MockApiClient::new();
        let result = read_or_create_auth_file(&auth_file_path, &config_file_path, &client).await;

        assert!(result.is_ok());
        assert!(std::fs::try_exists(auth_file_path).is_ok_and(|b| b));
        assert!(result.unwrap().tokens.is_empty());
    }

    #[tokio::test]
    async fn test_read_or_create_auth_file_existing_config_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .expect("Failed to create auth file path");
        let config_file_path =
            AbsoluteSystemPathBuf::try_from(tempdir.path().join(TURBOREPO_LEGACY_AUTH_FILE_NAME))
                .expect("Failed to create config file path");

        // Create config file data
        let mock_config_file_data = serde_json::to_string(&ConfigToken {
            token: "mock-token".to_string(),
        })
        .unwrap();

        // Write config file data to system.
        let mut file = File::create(&config_file_path).unwrap();
        file.write_all(mock_config_file_data.as_bytes()).unwrap();

        let client = MockApiClient::new();
        let result = read_or_create_auth_file(&auth_file_path, &config_file_path, &client).await;

        // Make sure no errors come back
        assert!(result.is_ok());
        // And then make sure the file was actually created on the fs
        assert!(std::fs::try_exists(auth_file_path).is_ok_and(|b| b));

        let auth_file = result.unwrap();
        assert_eq!(auth_file.tokens.len(), 1);
        assert_eq!(auth_file.tokens[0].token, "mock-token");
    }
}
