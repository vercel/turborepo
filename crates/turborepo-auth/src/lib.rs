#![feature(cow_is_borrowed)]
#![feature(assert_matches)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]
//! Turborepo's library for authenticating with the Vercel API.
//! Handles logging into Vercel, verifying SSO, and storing the token.

mod auth;
mod error;
mod login_server;
mod ui;

pub use auth::*;
pub use error::Error;
pub use login_server::*;
use serde::Deserialize;
use turbopath::AbsoluteSystemPath;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_vercel_api::{User, token::ResponseTokenMetadata};

pub struct TeamInfo<'a> {
    pub id: &'a str,
    pub slug: &'a str,
}

pub const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
pub const VERCEL_TOKEN_FILE: &str = "auth.json";
pub const TURBO_TOKEN_DIR: &str = "turborepo";
pub const TURBO_TOKEN_FILE: &str = "config.json";

const VERCEL_OAUTH_CLIENT_ID: &str = "cl_HYyOPBNtFMfHhaUn9L4QPfTZz6TP47bp";
const VERCEL_OAUTH_TOKEN_URL: &str = "https://vercel.com/api/login/oauth/token";

#[derive(Debug, Clone)]
pub struct AuthTokens {
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
}

/// Token.
///
/// It's the result of a successful login or an existing token. This acts as
/// a wrapper for a bunch of token operations, like validation. We explicitly do
/// not store any information about the underlying token for a few reasons, like
/// having a token invalidated on the web but not locally.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// An existing token on the filesystem
    Existing(String),
    /// A token that was just created, but not yet written to the filesystem
    New(String),
}
impl Token {
    pub fn new(token: String) -> Self {
        Self::New(token)
    }
    pub fn existing(token: String) -> Self {
        Self::Existing(token)
    }
    /// Reads a token from a file. If the file is a JSON object with a
    /// `token` field, we read that in. If no such field exists, we error out.
    ///
    /// ## Errors
    /// * `Error::TokenNotFound` - If the file does not exist.
    /// * `Error::InvalidTokenFileFormat` - If the file does not contain a
    ///   properly formatted JSON object with a `token` field.
    pub fn from_file(path: &AbsoluteSystemPath) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct TokenWrapper {
            token: Option<String>,
        }

        match path.read_existing_to_string()? {
            Some(content) => {
                let wrapper = serde_json::from_str::<TokenWrapper>(&content).map_err(|err| {
                    Error::InvalidTokenFileFormat {
                        path: path.to_string(),
                        source: err,
                    }
                })?;
                if let Some(token) = wrapper.token {
                    Ok(Self::Existing(token))
                } else {
                    Err(Error::TokenNotFound)
                }
            }
            None => Err(Error::TokenNotFound),
        }
    }

    /// Reads token, refresh token, and expiration from auth.json
    pub fn from_auth_file(path: &AbsoluteSystemPath) -> Result<AuthTokens, Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct AuthWrapper {
            token: Option<String>,
            refresh_token: Option<String>,
            expires_at: Option<u64>,
        }

        match path.read_existing_to_string()? {
            Some(content) => {
                let wrapper = serde_json::from_str::<AuthWrapper>(&content).map_err(|err| {
                    Error::InvalidTokenFileFormat {
                        path: path.to_string(),
                        source: err,
                    }
                })?;
                Ok(AuthTokens {
                    token: wrapper.token,
                    refresh_token: wrapper.refresh_token,
                    expires_at: wrapper.expires_at,
                })
            }
            None => Ok(AuthTokens {
                token: None,
                refresh_token: None,
                expires_at: None,
            }),
        }
    }

    /// Checks if the token is still valid. The checks ran are:
    /// 1. If the token is active.
    /// 2. If the token has access to the cache.
    ///     - If the token is forbidden from accessing the cache, we consider it
    ///       invalid.
    /// 3. We are able to fetch the user associated with the token.
    ///
    /// ## Arguments
    /// * `client` - The client to use for API calls.
    /// * `valid_message_fn` - An optional callback that gets called if the
    ///   token is valid. It will be passed the user's email.
    // TODO(voz): This should do a `get_user` or `get_teams` instead of the caller
    // doing it. The reason we don't do it here is because the caller
    // needs to do printing and requires the user struct, which we don't want to
    // return here.
    pub async fn is_valid<T: Client + TokenClient + CacheClient>(
        &self,
        client: &T,
        // Making this optional since there are cases where we don't want to do anything after
        // validation.
        // A callback that gets called if the token is valid. This will be
        // passed in a user's email if the token is valid.
        valid_message_fn: Option<impl FnOnce(&str)>,
    ) -> Result<bool, Error> {
        let (is_active, has_cache_access) = tokio::try_join!(
            self.is_active(client),
            self.has_cache_access(client, None, None)
        )?;
        if !is_active || !has_cache_access {
            return Ok(false);
        }

        if let Some(message_callback) = valid_message_fn {
            let user = self.user(client).await?;
            message_callback(&user.email);
        }
        Ok(true)
    }

    async fn handle_sso_token_error<T: TokenClient>(
        &self,
        client: &T,
        error: reqwest::Error,
    ) -> Result<bool, Error> {
        if error.status() == Some(reqwest::StatusCode::FORBIDDEN) {
            let metadata = self.fetch_metadata(client).await?;
            if !metadata.token_type.is_empty() {
                return Err(Error::APIError(turborepo_api_client::Error::InvalidToken {
                    status: error
                        .status()
                        .unwrap_or(reqwest::StatusCode::FORBIDDEN)
                        .as_u16(),
                    url: error
                        .url()
                        .map(|u| u.to_string())
                        .unwrap_or("Unknown url".to_string()),
                    message: error.to_string(),
                }));
            }
        }

        Err(Error::APIError(turborepo_api_client::Error::ReqwestError(
            error,
        )))
    }

    /// This is the same as `is_valid`, but also checks if the token is valid
    /// for SSO.
    ///
    /// ## Arguments
    /// * `client` - The client to use for API calls.
    /// * `sso_team` - The team to validate the token against.
    /// * `valid_message_fn` - An optional callback that gets called if the
    ///   token is valid. It will be passed the user's email.
    pub async fn is_valid_sso<T: Client + TokenClient + CacheClient>(
        &self,
        client: &T,
        sso_team: &str,
        // Making this optional since there are cases where we don't want to do anything after
        // validation.
        // A callback that gets called if the token is valid. This will be
        // passed in a user's email if the token is valid.
        valid_message_fn: Option<impl FnOnce(&str)>,
    ) -> Result<bool, Error> {
        let is_active = self.is_active(client).await?;
        let (result_user, result_team) = tokio::join!(
            client.get_user(self.into_inner()),
            client.get_team(self.into_inner(), sso_team)
        );

        match (result_user, result_team) {
            (Ok(response_user), Ok(response_team)) => {
                let team =
                    response_team.ok_or_else(|| Error::SSOTeamNotFound(sso_team.to_owned()))?;
                let info = TeamInfo {
                    id: &team.id,
                    slug: &team.slug,
                };
                if info.slug != sso_team {
                    return Err(Error::SSOTeamNotFound(sso_team.to_owned()));
                }

                let has_cache_access = self
                    .has_cache_access(client, Some(info.id), Some(info.slug))
                    .await?;
                if !is_active || !has_cache_access {
                    return Ok(false);
                }

                if let Some(message_callback) = valid_message_fn {
                    message_callback(&response_user.user.email);
                };

                Ok(true)
            }
            (Err(e), _) | (_, Err(e)) => match e {
                turborepo_api_client::Error::ReqwestError(e) => {
                    self.handle_sso_token_error(client, e).await
                }
                e => Err(Error::APIError(e)),
            },
        }
    }

    /// Checks if the token is active. We do a few checks:
    /// 1. Fetch the token metadata.
    /// 2. From the metadata, check if the token is active.
    /// 3. If the token is a SAML SSO token, check if it's expired.
    pub async fn is_active<T: TokenClient>(&self, client: &T) -> Result<bool, Error> {
        let metadata = self.fetch_metadata(client).await?;
        let current_time = current_unix_time();
        let active = is_token_active(&metadata, current_time);
        Ok(active)
    }

    /// Checks if the token has access to the cache. This is a separate check
    /// from `is_active` because it's possible for a token to be active but not
    /// have access to the cache.
    pub async fn has_cache_access<T: CacheClient>(
        &self,
        client: &T,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<bool, Error> {
        match client
            .get_caching_status(self.into_inner(), team_id, team_slug)
            .await
        {
            // If we get a successful response, we have cache access and therefore consider it good.
            // TODO: In the future this response should include something that tells us what actions
            // this token can perform.
            Ok(_) => Ok(true),
            // An error can mean that we were unable to fetch the cache status, or that the token is
            // forbidden from accessing the cache. A forbidden means we should return a `false`,
            // otherwise we return an actual error.
            Err(e) => match e {
                // Check to make sure the code is "forbidden" before returning a `false`.
                turborepo_api_client::Error::UnknownStatus { code, .. } if code == "forbidden" => {
                    Ok(false)
                }
                // If the entire request fails with 403 also return false
                turborepo_api_client::Error::ReqwestError(e)
                    if e.status() == Some(reqwest::StatusCode::FORBIDDEN) =>
                {
                    Ok(false)
                }
                _ => Err(e.into()),
            },
        }
    }

    /// Fetches the user associated with the token.
    pub async fn user(&self, client: &impl Client) -> Result<User, Error> {
        let user_response = client.get_user(self.into_inner()).await?;
        Ok(user_response.user)
    }

    async fn fetch_metadata(
        &self,
        client: &impl TokenClient,
    ) -> Result<ResponseTokenMetadata, Error> {
        client
            .get_metadata(self.into_inner())
            .await
            .map_err(Error::from)
    }

    /// Invalidates the token on the server.
    pub async fn invalidate<T: TokenClient>(&self, client: &T) -> Result<(), Error> {
        client.delete_token(self.into_inner()).await?;
        Ok(())
    }
    /// Returns the underlying token string.
    pub fn into_inner(&self) -> &str {
        match self {
            Self::Existing(token) | Self::New(token) => token.as_str(),
        }
    }
}

fn current_unix_time() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

fn current_unix_time_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

// As of the time of writing, this should always be true, since a token that
// isn't active returns an error when fetching metadata for the token.
fn is_token_active(metadata: &ResponseTokenMetadata, current_time: u128) -> bool {
    let active_at = metadata.active_at;

    let earliest_expiration = metadata
        .scopes
        .iter()
        .filter_map(|scope| scope.expires_at)
        .min();

    // Not all scopes have an expiration date, so we need to check if all of them
    // are expired. If there isn't an expiration date, we assume they are infinite
    // and therefore cannot be expired.
    let all_scopes_active = earliest_expiration.is_none_or(|expiration| current_time < expiration);

    all_scopes_active && (active_at <= current_time)
}

impl AuthTokens {
    /// Checks if the access token has expired based on expiresAt field
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let current_time = current_unix_time_secs();
            current_time >= expires_at
        } else {
            false
        }
    }

    /// Attempts to refresh the access token using the refresh token
    pub async fn refresh_token(&self) -> Result<AuthTokens, Error> {
        let refresh_token = self
            .refresh_token
            .as_ref()
            .ok_or_else(|| Error::TokenNotFound)?;

        let client = reqwest::Client::new();
        let params = [
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
            ("client_id", VERCEL_OAUTH_CLIENT_ID),
        ];

        let response = client
            .post(VERCEL_OAUTH_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            return Err(Error::FailedToGetToken);
        }

        let response_text = response.text().await?;

        let oauth_response: OAuthTokenResponse = serde_json::from_str(&response_text)?;

        Ok(AuthTokens {
            token: Some(oauth_response.access_token),
            refresh_token: Some(oauth_response.refresh_token),
            expires_at: Some(current_unix_time_secs() + 8 * 60 * 60), // 8 hours from now
        })
    }

    /// Writes the auth tokens to the auth.json file
    pub fn write_to_auth_file(&self, path: &AbsoluteSystemPath) -> Result<(), Error> {
        use serde_json::json;

        let content = json!({
            "// Note": "This is your Vercel credentials file. DO NOT SHARE!",
            "// Docs": "https://vercel.com/docs/projects/project-configuration/global-configuration#auth.json",
            "token": self.token,
            "refreshToken": self.refresh_token,
            "expiresAt": self.expires_at,
        });

        let json_string = serde_json::to_string_pretty(&content)?;
        path.ensure_dir()?;
        path.create_with_contents(json_string)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::backtrace::Backtrace;

    use insta::assert_snapshot;
    use reqwest::{Method, RequestBuilder, Response};
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_vercel_api::{
        CachingStatus, CachingStatusResponse, Team, TeamsResponse, User, UserResponse,
        VerifiedSsoUser, token::Scope,
    };
    use url::Url;

    use super::*;

    // Shared mock client that can be reused across tests
    struct MockUserClient {
        should_succeed: bool,
    }

    impl MockUserClient {
        fn new(should_succeed: bool) -> Self {
            Self { should_succeed }
        }
    }

    impl Client for MockUserClient {
        async fn get_user(&self, token: &str) -> turborepo_api_client::Result<UserResponse> {
            if !self.should_succeed {
                return Err(turborepo_api_client::Error::UnknownStatus {
                    code: "unauthorized".to_string(),
                    message: "Invalid token".to_string(),
                    backtrace: Backtrace::capture(),
                });
            }

            if token.is_empty() {
                return Err(turborepo_api_client::Error::UnknownStatus {
                    code: "empty_token".to_string(),
                    message: "Token cannot be empty".to_string(),
                    backtrace: Backtrace::capture(),
                });
            }

            Ok(UserResponse {
                user: User {
                    id: "test_user_id".to_string(),
                    username: "test_user".to_string(),
                    email: "test@example.com".to_string(),
                    name: Some("Test User".to_string()),
                    created_at: Some(123456789),
                },
            })
        }

        async fn get_teams(&self, _token: &str) -> turborepo_api_client::Result<TeamsResponse> {
            unimplemented!("get_teams")
        }
        async fn get_team(
            &self,
            _token: &str,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn verify_sso_token(
            &self,
            _token: &str,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
            unimplemented!("verify_sso_token")
        }
        async fn handle_403(_response: Response) -> turborepo_api_client::Error {
            unimplemented!("handle_403")
        }
        fn make_url(&self, _endpoint: &str) -> turborepo_api_client::Result<Url> {
            unimplemented!("make_url")
        }
    }

    #[test]
    fn test_is_token_active() {
        let current_time = current_unix_time();
        let quick_scope = |expiry| Scope {
            expires_at: expiry,
            scope_type: "".to_string(),
            created_at: 0,
            team_id: None,
        };
        let mock_response = |active_at, scopes| ResponseTokenMetadata {
            active_at,
            scopes,
            // These fields don't matter in the test
            id: "".to_string(),
            name: "".to_string(),
            token_type: "".to_string(),
            created_at: 0,
        };

        let cases = vec![
            // Case: Token active, no scopes (implicitly infinite)
            (current_time - 100, vec![], true),
            // Case: Token active, one scope without expiration
            (current_time - 100, vec![quick_scope(None)], true),
            // Case: Token active, one scope expired
            (
                current_time - 100,
                vec![quick_scope(Some(current_time - 1))],
                false,
            ),
            // Case: Token active, one scope not expired
            (
                current_time - 100,
                vec![quick_scope(Some(current_time + 11))],
                true,
            ),
            // Case: Token active, all scopes not expired
            (
                current_time - 100,
                vec![
                    quick_scope(Some(current_time + 11)),
                    quick_scope(Some(current_time + 10)),
                ],
                true,
            ),
            // Case: Token inactive (future `active_at`)
            (
                current_time + 1000,
                vec![quick_scope(Some(current_time + 20))],
                false,
            ),
        ];

        for (active_at, scopes, expected) in cases {
            let metadata = mock_response(active_at, scopes);
            assert_eq!(
                is_token_active(&metadata, current_time),
                expected,
                "Test failed for active_at: {active_at}"
            );
        }
    }

    #[test]
    fn test_from_file_with_valid_token() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("valid_token.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");
        file_path
            .create_with_contents(r#"{"token": "valid_token_here"}"#)
            .unwrap();

        let result = Token::from_file(&file_path).expect("Failed to read token from file");

        assert!(matches!(result, Token::Existing(ref t) if t == "valid_token_here"));
    }

    #[test]
    fn test_from_file_with_invalid_json() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("invalid_token.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");
        file_path.create_with_contents("not a valid json").unwrap();

        let result = Token::from_file(&file_path);
        let mut err = result.unwrap_err();
        if let Error::InvalidTokenFileFormat { path, .. } = &mut err {
            // Overriding temporary path so we can use snapshot for error message
            *path = "/path/to/bad.json".to_owned();
        }
        assert_snapshot!(err, @"'/path/to/bad.json' is an invalid token file: expected ident at line 1 column 2");
    }

    #[test]
    fn test_from_file_with_no_file() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("nonexistent.json"); // No need to create this file

        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");
        let result = Token::from_file(&file_path);

        assert!(matches!(result, Err(Error::TokenNotFound)));
    }

    #[test]
    fn test_auth_tokens_is_expired() {
        let current_time = current_unix_time_secs();

        // Test with no expiry (should not be expired)
        let tokens_no_expiry = AuthTokens {
            token: Some("test_token".to_string()),
            refresh_token: Some("refresh_token".to_string()),
            expires_at: None,
        };
        assert!(!tokens_no_expiry.is_expired());

        // Test with future expiry (should not be expired)
        let tokens_future_expiry = AuthTokens {
            token: Some("test_token".to_string()),
            refresh_token: Some("refresh_token".to_string()),
            expires_at: Some(current_time + 3600), // 1 hour in the future
        };
        assert!(!tokens_future_expiry.is_expired());

        // Test with past expiry (should be expired)
        let tokens_past_expiry = AuthTokens {
            token: Some("test_token".to_string()),
            refresh_token: Some("refresh_token".to_string()),
            expires_at: Some(current_time - 3600), // 1 hour in the past
        };
        assert!(tokens_past_expiry.is_expired());

        // Test edge case: exactly at expiry time (should be expired)
        let tokens_exact_expiry = AuthTokens {
            token: Some("test_token".to_string()),
            refresh_token: Some("refresh_token".to_string()),
            expires_at: Some(current_time),
        };
        assert!(tokens_exact_expiry.is_expired());
    }

    #[test]
    fn test_from_auth_file_with_valid_data() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("auth.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");

        let auth_content = r#"{
            "token": "vca_test_token_123",
            "refreshToken": "refresh_token_456",
            "expiresAt": 1234567890
        }"#;
        file_path.create_with_contents(auth_content).unwrap();

        let result = Token::from_auth_file(&file_path).expect("Failed to read auth from file");

        assert_eq!(result.token, Some("vca_test_token_123".to_string()));
        assert_eq!(result.refresh_token, Some("refresh_token_456".to_string()));
        assert_eq!(result.expires_at, Some(1234567890));
    }

    #[test]
    fn test_from_auth_file_with_missing_fields() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("auth.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");

        // Test with only token field
        let auth_content = r#"{"token": "legacy_token_123"}"#;
        file_path.create_with_contents(auth_content).unwrap();

        let result = Token::from_auth_file(&file_path).expect("Failed to read auth from file");

        assert_eq!(result.token, Some("legacy_token_123".to_string()));
        assert_eq!(result.refresh_token, None);
        assert_eq!(result.expires_at, None);
    }

    #[test]
    fn test_from_auth_file_empty_file() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("nonexistent_auth.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");

        let result = Token::from_auth_file(&file_path).expect("Should return empty AuthTokens");

        assert_eq!(result.token, None);
        assert_eq!(result.refresh_token, None);
        assert_eq!(result.expires_at, None);
    }

    #[test]
    fn test_write_to_auth_file() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("test_auth.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");

        let tokens = AuthTokens {
            token: Some("vca_test_token".to_string()),
            refresh_token: Some("test_refresh_token".to_string()),
            expires_at: Some(1234567890),
        };

        tokens
            .write_to_auth_file(&file_path)
            .expect("Failed to write auth file");

        // Read back and verify
        let content = file_path
            .read_to_string()
            .expect("Failed to read auth file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("Invalid JSON");

        assert_eq!(parsed["token"], "vca_test_token");
        assert_eq!(parsed["refreshToken"], "test_refresh_token");
        assert_eq!(parsed["expiresAt"], 1234567890);

        // Verify the JSON structure includes the expected comments
        assert!(content.contains("This is your Vercel credentials file"));
        assert!(content.contains(
            "https://vercel.com/docs/projects/project-configuration/global-configuration#auth.json"
        ));
    }

    #[tokio::test]
    async fn test_refresh_token_missing_refresh_token() {
        let tokens = AuthTokens {
            token: Some("vca_test_token".to_string()),
            refresh_token: None, // No refresh token
            expires_at: Some(current_unix_time_secs() - 3600),
        };

        let result = tokens.refresh_token().await;
        assert!(matches!(result, Err(Error::TokenNotFound)));
    }

    #[test]
    fn test_auth_tokens_roundtrip() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("roundtrip_auth.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");

        let original_tokens = AuthTokens {
            token: Some("vca_roundtrip_token".to_string()),
            refresh_token: Some("roundtrip_refresh_token".to_string()),
            expires_at: Some(1234567890),
        };

        // Write tokens to file
        original_tokens
            .write_to_auth_file(&file_path)
            .expect("Failed to write auth file");

        // Read tokens back from file
        let read_tokens = Token::from_auth_file(&file_path).expect("Failed to read auth file");

        // Verify they match
        assert_eq!(original_tokens.token, read_tokens.token);
        assert_eq!(original_tokens.refresh_token, read_tokens.refresh_token);
        assert_eq!(original_tokens.expires_at, read_tokens.expires_at);
    }

    enum MockErrorType {
        Error,
        Forbidden,
    }
    enum MockCachingResponse {
        CachingStatus(bool),
        Error(MockErrorType),
    }

    struct MockCacheClient {
        pub response: MockCachingResponse,
    }

    impl CacheClient for MockCacheClient {
        async fn get_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
            _method: Method,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!()
        }

        async fn fetch_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!()
        }

        async fn put_artifact(
            &self,
            _hash: &str,
            _artifact_body: impl turborepo_api_client::Stream<
                Item = Result<turborepo_api_client::Bytes, turborepo_api_client::Error>,
            > + Send
            + Sync
            + 'static,
            _body_len: usize,
            _duration: u64,
            _tag: Option<&str>,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<(), turborepo_api_client::Error> {
            unimplemented!()
        }

        async fn artifact_exists(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!()
        }

        async fn get_caching_status(
            &self,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<CachingStatusResponse, turborepo_api_client::Error> {
            match self.response {
                MockCachingResponse::CachingStatus(status) => {
                    let caching_status = if status {
                        CachingStatus::Enabled
                    } else {
                        CachingStatus::Disabled
                    };
                    Ok(CachingStatusResponse {
                        status: caching_status,
                    })
                }
                MockCachingResponse::Error(MockErrorType::Error) => {
                    Err(turborepo_api_client::Error::UnknownStatus {
                        code: "error".to_string(),
                        message: "Error fetching caching status".to_string(),
                        backtrace: Backtrace::capture(),
                    })
                }
                MockCachingResponse::Error(MockErrorType::Forbidden) => {
                    Err(turborepo_api_client::Error::UnknownStatus {
                        code: "forbidden".to_string(),
                        message: "Forbidden from accessing cache".to_string(),
                        backtrace: Backtrace::capture(),
                    })
                }
            }
        }
    }

    #[tokio::test]
    async fn test_has_cache_access_granted() {
        let mock = MockCacheClient {
            response: MockCachingResponse::CachingStatus(true),
        };

        let token = Token::Existing("existing_token".to_string());
        let team_info = TeamInfo {
            id: "team_id",
            slug: "team_slug",
        };

        let result = token
            .has_cache_access(&mock, Some(team_info.id), Some(team_info.slug))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_cache_access_denied() {
        let mock = MockCacheClient {
            response: MockCachingResponse::Error(MockErrorType::Forbidden),
        };

        let token = Token::Existing("existing_token".to_string());
        let team_info = TeamInfo {
            id: "team_id",
            slug: "team_slug",
        };

        let result = token
            .has_cache_access(&mock, Some(team_info.id), Some(team_info.slug))
            .await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_caching_status_errored() {
        let mock = MockCacheClient {
            response: MockCachingResponse::Error(MockErrorType::Error),
        };

        let token = Token::Existing("existing_token".to_string());
        let team_info = TeamInfo {
            id: "team_id",
            slug: "team_slug",
        };

        let result = token
            .has_cache_access(&mock, Some(team_info.id), Some(team_info.slug))
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::APIError(_)));
    }

    struct MockTokenClient {
        metadata_response: Option<ResponseTokenMetadata>,
        should_fail: bool,
    }

    impl TokenClient for MockTokenClient {
        async fn get_metadata(
            &self,
            _token: &str,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            if self.should_fail {
                return Err(turborepo_api_client::Error::UnknownStatus {
                    code: "error".to_string(),
                    message: "Failed to get metadata".to_string(),
                    backtrace: Backtrace::capture(),
                });
            }

            if let Some(metadata) = &self.metadata_response {
                Ok(metadata.clone())
            } else {
                Ok(ResponseTokenMetadata {
                    id: "test".to_string(),
                    name: "test".to_string(),
                    token_type: "test".to_string(),
                    scopes: vec![],
                    active_at: current_unix_time() - 100,
                    created_at: 0,
                })
            }
        }

        async fn delete_token(&self, _token: &str) -> turborepo_api_client::Result<()> {
            if self.should_fail {
                return Err(turborepo_api_client::Error::UnknownStatus {
                    code: "error".to_string(),
                    message: "Failed to delete token".to_string(),
                    backtrace: Backtrace::capture(),
                });
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_token_invalidate() {
        let token = Token::new("test-token".to_string());

        // Test successful invalidation
        let client = MockTokenClient {
            metadata_response: None,
            should_fail: false,
        };
        assert!(token.invalidate(&client).await.is_ok());

        // Test failed invalidation
        let client = MockTokenClient {
            metadata_response: None,
            should_fail: true,
        };
        assert!(token.invalidate(&client).await.is_err());
    }

    #[tokio::test]
    async fn test_token_is_active() {
        let token = Token::new("test-token".to_string());
        let current_time = current_unix_time();

        // Test active token
        let client = MockTokenClient {
            metadata_response: Some(ResponseTokenMetadata {
                id: "test".to_string(),
                name: "test".to_string(),
                token_type: "test".to_string(),
                scopes: vec![],
                active_at: current_time - 100,
                created_at: 0,
            }),
            should_fail: false,
        };
        assert!(token.is_active(&client).await.unwrap());

        // Test inactive token (future active_at)
        let client = MockTokenClient {
            metadata_response: Some(ResponseTokenMetadata {
                active_at: current_time + 1000,
                ..ResponseTokenMetadata {
                    id: "test".to_string(),
                    name: "test".to_string(),
                    token_type: "test".to_string(),
                    scopes: vec![],
                    created_at: 0,
                    active_at: 0,
                }
            }),
            should_fail: false,
        };
        assert!(!token.is_active(&client).await.unwrap());

        // Test failed metadata fetch
        let client = MockTokenClient {
            metadata_response: None,
            should_fail: true,
        };
        assert!(token.is_active(&client).await.is_err());
    }

    #[test]
    fn test_from_file_with_empty_token() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("empty_token.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");
        // TODO: This should probably be failing. An empty string is an empty token.
        file_path.create_with_contents(r#"{"token": ""}"#).unwrap();

        let result = Token::from_file(&file_path).expect("Failed to read token from file");
        assert!(matches!(result, Token::Existing(ref t) if t.is_empty()));
    }

    #[test]
    fn test_from_file_with_missing_token_field() {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        let tmp_path = tmp_dir.path().join("missing_token.json");
        let file_path = AbsoluteSystemPathBuf::try_from(tmp_path)
            .expect("Failed to create AbsoluteSystemPathBuf");
        file_path
            .create_with_contents(r#"{"other_field": "value"}"#)
            .unwrap();

        let result = Token::from_file(&file_path);
        assert!(matches!(result, Err(Error::TokenNotFound)));
    }

    struct MockSSOTokenClient {
        metadata_response: Option<ResponseTokenMetadata>,
    }

    impl TokenClient for MockSSOTokenClient {
        async fn get_metadata(
            &self,
            _token: &str,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            if let Some(metadata) = &self.metadata_response {
                Ok(metadata.clone())
            } else {
                Ok(ResponseTokenMetadata {
                    id: "test".to_string(),
                    name: "test".to_string(),
                    token_type: "".to_string(),
                    scopes: vec![],
                    active_at: current_unix_time() - 100,
                    created_at: 0,
                })
            }
        }

        async fn delete_token(&self, _token: &str) -> turborepo_api_client::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_handle_sso_token_error_forbidden_with_invalid_token_error() {
        let token = Token::new("test-token".to_string());
        let client = MockSSOTokenClient {
            metadata_response: Some(ResponseTokenMetadata {
                id: "test".to_string(),
                name: "test".to_string(),
                token_type: "sso".to_string(),
                scopes: vec![],
                active_at: current_unix_time() - 100,
                created_at: 0,
            }),
        };

        let errorful_response = reqwest::Response::from(
            http::Response::builder()
                .status(reqwest::StatusCode::FORBIDDEN)
                .body("")
                .unwrap(),
        );

        let result = token
            .handle_sso_token_error(&client, errorful_response.error_for_status().unwrap_err())
            .await;
        assert!(matches!(
            result,
            Err(Error::APIError(
                turborepo_api_client::Error::InvalidToken { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn test_handle_sso_token_error_forbidden_without_token_type() {
        let token = Token::new("test-token".to_string());
        let client = MockSSOTokenClient {
            metadata_response: Some(ResponseTokenMetadata {
                id: "test".to_string(),
                name: "test".to_string(),
                token_type: "".to_string(),
                scopes: vec![],
                active_at: current_unix_time() - 100,
                created_at: 0,
            }),
        };

        let errorful_response = reqwest::Response::from(
            http::Response::builder()
                .status(reqwest::StatusCode::FORBIDDEN)
                .body("")
                .unwrap(),
        );

        let result = token
            .handle_sso_token_error(&client, errorful_response.error_for_status().unwrap_err())
            .await;
        assert!(matches!(
            result,
            Err(Error::APIError(turborepo_api_client::Error::ReqwestError(
                _
            )))
        ));
    }

    #[tokio::test]
    async fn test_handle_sso_token_error_non_forbidden() {
        let token = Token::new("test-token".to_string());
        let client = MockSSOTokenClient {
            metadata_response: None,
        };

        let errorful_response = reqwest::Response::from(
            http::Response::builder()
                .status(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
                .body("")
                .unwrap(),
        );

        let result = token
            .handle_sso_token_error(&client, errorful_response.error_for_status().unwrap_err())
            .await;
        assert!(matches!(
            result,
            Err(Error::APIError(turborepo_api_client::Error::ReqwestError(
                _
            )))
        ));
    }

    #[tokio::test]
    async fn test_token_user_fetch() {
        let token = Token::new("valid-token".to_string());

        // Test successful user fetch
        let success_client = MockUserClient::new(true);
        let user_result = token.user(&success_client).await;
        assert!(user_result.is_ok());
        let user = user_result.unwrap();
        assert_eq!(user.id, "test_user_id");
        assert_eq!(user.username, "test_user");
        assert_eq!(user.email, "test@example.com");
        assert_eq!(user.name, Some("Test User".to_string()));
    }

    #[tokio::test]
    async fn test_token_user_fetch_with_empty_token() {
        // Test with empty token
        let empty_token = Token::new("".to_string());
        let user_result = empty_token.user(&MockUserClient::new(true)).await;
        assert!(user_result.is_err());
        assert!(matches!(user_result.unwrap_err(), Error::APIError(_)));
    }

    #[tokio::test]
    async fn test_empty_token_user_fetch_returns_api_error() {
        let empty_token = Token::new("".to_string());
        let user_result = empty_token.user(&MockUserClient::new(true)).await;
        assert!(user_result.is_err());
        assert!(matches!(user_result.unwrap_err(), Error::APIError(_)));
    }
}
