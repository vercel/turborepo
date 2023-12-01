use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::Client;

use crate::Error;

#[derive(Serialize, Deserialize, Debug, Default)]
/// AuthFile contains a list of domains, each with a token.
pub struct AuthFile {
    pub tokens: HashMap<String, String>,
}

impl AuthFile {
    /// Create an empty auth file. Caller must invoke `write_to_disk` to
    /// actually write it to disk.
    pub fn new() -> Self {
        AuthFile {
            tokens: HashMap::new(),
        }
    }
    /// Writes the contents of the auth file to disk. Will override whatever is
    /// there with what's in the struct.
    pub fn write_to_disk(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        path.ensure_dir().map_err(|e| Error::PathError(e.into()))?;

        let mut pretty_content = serde_json::to_string_pretty(self)
            .map_err(|e| Error::FailedToSerializeAuthFile { source: e })?;
        // to_string_pretty doesn't add terminating line endings, so do that.
        pretty_content.push('\n');

        path.create_with_contents(pretty_content)
            .map_err(|e| crate::Error::FailedToWriteAuth {
                auth_path: path.to_owned(),
                error: e,
            })?;

        Ok(())
    }
    pub fn get_token(&self, api: &str) -> Option<AuthToken> {
        self.tokens.get(api).map(|raw_token| AuthToken {
            token: raw_token.to_owned(),
            api: api.to_owned(),
        })
    }
    /// Adds a token to the auth file. Attempts to match exclusively on `api`.
    /// If the api matches a token already in the file, it will be updated with
    /// the new token.
    pub fn add_or_update_token(&mut self, api: String, token: String) {
        self.tokens.insert(api, token.to_string());
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
/// Contains the token itself and a list of teams the token is valid for.
pub struct AuthToken {
    /// The token itself.
    pub token: String,
    /// The API URL the token was issued from / for.
    pub api: String,
}

impl AuthToken {
    /// Validates the token by checking the expiration date and the signature.
    pub async fn validate(&self, _client: impl Client) -> bool {
        todo!("validate token")
    }
    pub fn friendly_token_display(&self) -> String {
        format!(
            "{}...{}",
            &self.token[..3],
            &self.token[self.token.len() - 3..]
        )
    }
    pub fn friendly_api_display(&self) -> String {
        if self.api.contains("vercel.com") {
            // We're Vercel, let's make it look nice ;)
            return "▲ Vercel Remote Cache".to_owned();
        }
        self.api.clone()
    }
}

/// Converts our old style of token held in `config.json` into the new schema.
pub async fn convert_to_auth_token(token: &str, client: &impl Client) -> Result<AuthToken, Error> {
    let auth_token = AuthToken {
        token: token.to_string(),
        api: client.base_url().to_owned(),
    };
    Ok(auth_token)
}

#[cfg(test)]
mod tests {
    use std::{fs, ops::Deref};

    use tempfile::tempdir;

    use super::*;
    use crate::{mocks::*, TURBOREPO_AUTH_FILE_NAME, TURBOREPO_CONFIG_DIR};

    #[tokio::test]
    async fn test_convert_to_auth_token() {
        // Setup: Create a mock client and a fake token
        let mock_client = MockApiClient::new();
        let token = "test-token";
        let temp_dir = tempfile::tempdir().unwrap();

        // Create the temp dir files.
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();

        // Test: Call the convert_to_auth_file function and check the result
        let result = convert_to_auth_token(token, &mock_client).await;
        assert!(result.is_ok());
        let auth_token = result.unwrap();

        // Check that the AuthFile contains the correct data
        assert_eq!(auth_token.token, "test-token".to_string());
        assert_eq!(auth_token.api, "custom-domain".to_string());
    }

    #[tokio::test]
    async fn test_write_to_disk_and_read_back() {
        // Setup: Use temp dirs to avoid polluting the user's config dir
        let temp_dir = tempdir().unwrap();
        let auth_file_path = temp_dir
            .path()
            .join(TURBOREPO_CONFIG_DIR)
            .join(TURBOREPO_AUTH_FILE_NAME);

        // unwrapping is fine because we know the path exists
        let absolute_auth_path = AbsoluteSystemPath::new(auth_file_path.to_str().unwrap()).unwrap();

        // Make sure the temp dir exists before writing to it.
        fs::create_dir_all(temp_dir.path().join(TURBOREPO_CONFIG_DIR)).unwrap();

        // Add a token to auth file
        let mut auth_file = AuthFile::default();
        auth_file.add_or_update_token("test-api".to_string(), "test-token".to_string());

        // Test: Write the auth file to disk and then read it back.
        auth_file.write_to_disk(&absolute_auth_path).unwrap();

        let read_back: AuthFile =
            serde_json::from_str(&fs::read_to_string(absolute_auth_path.clone()).unwrap()).unwrap();
        assert_eq!(read_back.tokens.len(), 1);
        assert!(read_back.tokens.contains_key("test-api"));
        assert_eq!(
            read_back.tokens.get("test-api").unwrap().deref(),
            "test-token".to_owned()
        );
    }

    #[tokio::test]
    async fn test_get_token() {
        let mut auth_file = AuthFile::default();
        auth_file.add_or_update_token("test-api".to_string(), "test-token".to_string());

        let token = auth_file.get_token("test-api");
        assert!(token.is_some());
        assert_eq!(token.unwrap().token, "test-token");
    }

    #[tokio::test]
    async fn test_add_token() {
        // Setup: Create an empty auth file.
        let mut auth_file = AuthFile::default();
        assert_eq!(auth_file.tokens.len(), 0);

        // Test: Add a token to the auth file, then add same key with a different value
        // to ensure update happens.
        auth_file.add_or_update_token("test-api".to_string(), "test-token".to_string());
        auth_file.add_or_update_token("test-api".to_string(), "some new token".to_string());

        assert_eq!(auth_file.tokens.len(), 1);
        let mut token = auth_file.get_token("test-api");
        assert!(token.is_some());

        let mut t = token.unwrap();
        assert!(t.token == *"some new token");

        auth_file.add_or_update_token("some vercel api".to_string(), "a second token".to_string());
        assert_eq!(auth_file.tokens.len(), 2);

        token = auth_file.get_token("some vercel api");
        assert!(token.is_some());

        t = token.unwrap();
        assert!(t.token == *"a second token");
    }
}
