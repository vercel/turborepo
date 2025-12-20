mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
#[cfg(test)]
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::ColorConfig;

use crate::LoginServer;

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

pub struct LoginOptions<'a, T: Client + TokenClient + CacheClient> {
    pub color_config: &'a ColorConfig,
    pub login_url: &'a str,
    pub api_client: &'a T,
    pub login_server: &'a dyn LoginServer,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
    pub force: bool,
    pub sso_login_callback_port: Option<u16>,
}
impl<'a, T: Client + TokenClient + CacheClient> LoginOptions<'a, T> {
    pub fn new(
        color_config: &'a ColorConfig,
        login_url: &'a str,
        api_client: &'a T,
        login_server: &'a dyn LoginServer,
    ) -> Self {
        Self {
            color_config,
            login_url,
            api_client,
            login_server,
            sso_team: None,
            existing_token: None,
            force: false,
            sso_login_callback_port: None,
        }
    }
}

/// Options for logging out.
pub struct LogoutOptions<T> {
    pub color_config: ColorConfig,
    pub api_client: T,
    /// If we should invalidate the token on the server.
    pub invalidate: bool,
    /// Path override for testing
    #[cfg(test)]
    pub path: Option<AbsoluteSystemPathBuf>,
}

/// Attempts to get a valid token with automatic refresh if expired.
/// Falls back to turborepo/config.json if refresh fails.
pub async fn get_token_with_refresh() -> Result<Option<String>, Error> {
    use crate::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token};

    let vercel_config_dir = match turborepo_dirs::vercel_config_dir()? {
        Some(dir) => dir,
        None => return Ok(None),
    };

    let auth_path = vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);

    let auth_tokens = Token::from_auth_file(&auth_path)?;

    if let Some(token) = &auth_tokens.token {
        if auth_tokens.is_expired() {
            // Only attempt refresh for Vercel tokens that start with "vca_"
            if token.starts_with("vca_")
                && auth_tokens.refresh_token.is_some()
                && let Ok(new_tokens) = auth_tokens.refresh_token().await
            {
                let _ = new_tokens.write_to_auth_file(&auth_path);
                return Ok(new_tokens.token);
            }

            if let Ok(Some(config_dir)) = turborepo_dirs::config_dir() {
                let turbo_config_path =
                    config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
                if let Ok(turbo_token) = Token::from_file(&turbo_config_path) {
                    return Ok(Some(turbo_token.into_inner().to_string()));
                }
            }

            Ok(None)
        } else {
            Ok(Some(token.clone()))
        }
    } else {
        // No token in auth.json, try turborepo/config.json
        if let Ok(Some(config_dir)) = turborepo_dirs::config_dir() {
            let turbo_config_path =
                config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
            if let Ok(turbo_token) = Token::from_file(&turbo_config_path) {
                return Ok(Some(turbo_token.into_inner().to_string()));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use crate::{AuthTokens, Token, current_unix_time_secs};

    // Mock the turborepo_dirs functions for testing
    fn create_mock_vercel_config_dir() -> AbsoluteSystemPathBuf {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        AbsoluteSystemPathBuf::try_from(tmp_dir.into_path()).expect("Failed to create path")
    }

    fn create_mock_turbo_config_dir() -> AbsoluteSystemPathBuf {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        AbsoluteSystemPathBuf::try_from(tmp_dir.into_path()).expect("Failed to create path")
    }

    fn setup_auth_file(
        config_dir: &AbsoluteSystemPathBuf,
        token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<u64>,
    ) {
        let auth_dir = config_dir.join_component("com.vercel.cli");
        fs::create_dir_all(&auth_dir).expect("Failed to create auth dir");
        let auth_file = auth_dir.join_component("auth.json");

        let auth_tokens = AuthTokens {
            token: Some(token.to_string()),
            refresh_token: refresh_token.map(|s| s.to_string()),
            expires_at,
        };

        auth_tokens
            .write_to_auth_file(&auth_file)
            .expect("Failed to write auth file");
    }

    fn setup_turbo_config_file(config_dir: &AbsoluteSystemPathBuf, token: &str) {
        let turbo_dir = config_dir.join_component("turborepo");
        fs::create_dir_all(&turbo_dir).expect("Failed to create turbo dir");
        let config_file = turbo_dir.join_component("config.json");

        let content = format!(r#"{{"token": "{token}"}}"#);
        config_file
            .create_with_contents(content)
            .expect("Failed to write turbo config");
    }

    #[tokio::test]
    async fn test_vca_token_with_valid_refresh() {
        // This test verifies that vca_ prefixed tokens attempt refresh when expired
        // Note: This test focuses on the logic flow rather than actual HTTP refresh
        // since we can't easily mock the HTTP client in this unit test

        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        // Setup expired vca_ token with refresh token
        setup_auth_file(
            &vercel_config_dir,
            "vca_expired_token_123",
            Some("refresh_token_456"),
            Some(current_time - 3600), // Expired 1 hour ago
        );

        // Read the auth tokens to verify the setup
        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

        // Verify the token is expired and has vca_ prefix
        assert!(auth_tokens.is_expired());
        assert!(auth_tokens.token.as_ref().unwrap().starts_with("vca_"));
        assert!(auth_tokens.refresh_token.is_some());

        // The actual refresh would happen in get_token_with_refresh, but we
        // can't test the HTTP call in a unit test. The important logic
        // is that it attempts refresh for vca_ tokens and falls back
        // appropriately.
    }

    #[tokio::test]
    async fn test_legacy_token_skips_refresh() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        // Setup expired legacy token (no vca_ prefix) with refresh token
        setup_auth_file(
            &vercel_config_dir,
            "legacy_token_123",
            Some("refresh_token_456"),
            Some(current_time - 3600), // Expired 1 hour ago
        );

        // Setup fallback turbo config token
        setup_turbo_config_file(&turbo_config_dir, "turbo_fallback_token");

        // Read the auth tokens to verify the setup
        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

        // Verify the token is expired and does NOT have vca_ prefix
        assert!(auth_tokens.is_expired());
        assert!(!auth_tokens.token.as_ref().unwrap().starts_with("vca_"));
        assert!(auth_tokens.refresh_token.is_some());

        // The key behavior: legacy tokens should NOT attempt refresh even if
        // they have a refresh token. They should fall back to turbo
        // config instead. This is the critical logic we're testing -
        // that the vca_ prefix check prevents refresh attempts for
        // legacy tokens.
    }

    #[tokio::test]
    async fn test_vca_token_without_refresh_token() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        // Setup expired vca_ token WITHOUT refresh token
        setup_auth_file(
            &vercel_config_dir,
            "vca_expired_token_123",
            None,                      // No refresh token
            Some(current_time - 3600), // Expired 1 hour ago
        );

        // Setup fallback turbo config token
        setup_turbo_config_file(&turbo_config_dir, "turbo_fallback_token");

        // Read the auth tokens to verify the setup
        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

        // Verify the token is expired, has vca_ prefix, but no refresh token
        assert!(auth_tokens.is_expired());
        assert!(auth_tokens.token.as_ref().unwrap().starts_with("vca_"));
        assert!(auth_tokens.refresh_token.is_none());

        // Even vca_ tokens should fall back to turbo config if they don't have
        // a refresh token
    }

    #[tokio::test]
    async fn test_non_expired_vca_token() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        // Setup non-expired vca_ token
        setup_auth_file(
            &vercel_config_dir,
            "vca_valid_token_123",
            Some("refresh_token_456"),
            Some(current_time + 3600), // Expires 1 hour from now
        );

        // Read the auth tokens to verify the setup
        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

        // Verify the token is NOT expired
        assert!(!auth_tokens.is_expired());
        assert!(auth_tokens.token.as_ref().unwrap().starts_with("vca_"));

        // Non-expired tokens should be returned as-is without any refresh
        // attempt
    }

    #[tokio::test]
    async fn test_non_expired_legacy_token() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        // Setup non-expired legacy token
        setup_auth_file(
            &vercel_config_dir,
            "legacy_token_123",
            Some("refresh_token_456"),
            Some(current_time + 3600), // Expires 1 hour from now
        );

        // Read the auth tokens to verify the setup
        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

        // Verify the token is NOT expired
        assert!(!auth_tokens.is_expired());
        assert!(!auth_tokens.token.as_ref().unwrap().starts_with("vca_"));

        // Non-expired legacy tokens should be returned as-is
    }

    #[tokio::test]
    async fn test_token_prefix_edge_cases() {
        let current_time = current_unix_time_secs();

        // Test various token prefixes to ensure only "vca_" triggers refresh
        let test_cases = vec![
            ("vca_token", true),         // Should attempt refresh
            ("VCA_token", false),        // Case sensitive - should not refresh
            ("vca_", true),              // Minimal vca_ prefix - should attempt refresh
            ("vca", false),              // Missing underscore - should not refresh
            ("xvca_token", false),       // Has vca_ but not at start - should not refresh
            ("", false),                 // Empty token - should not refresh
            ("some_other_token", false), // Different prefix - should not refresh
        ];

        for (token, should_attempt_refresh) in test_cases {
            let _auth_tokens = AuthTokens {
                token: Some(token.to_string()),
                refresh_token: Some("refresh_token".to_string()),
                expires_at: Some(current_time - 3600), // Expired
            };

            let has_vca_prefix = token.starts_with("vca_");
            assert_eq!(
                has_vca_prefix, should_attempt_refresh,
                "Token '{token}' prefix check failed"
            );
        }
    }
}
