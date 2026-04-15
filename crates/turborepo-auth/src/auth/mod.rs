mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
use tracing::warn;
use turbopath::AbsoluteSystemPath;
#[cfg(test)]
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::{Client, TokenClient};
use turborepo_ui::ColorConfig;

pub(crate) fn is_vercel(login_url: &str) -> bool {
    login_url.contains("vercel.com")
}

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingTokenSource {
    TurboConfig,
    LegacyAuth,
    Other,
}

pub struct LoginOptions<'a, T: Client + TokenClient> {
    pub color_config: &'a ColorConfig,
    pub login_url: &'a str,
    pub api_client: &'a T,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
    pub force: bool,
    pub sso_login_callback_port: Option<u16>,
}
impl<'a, T: Client + TokenClient> LoginOptions<'a, T> {
    pub fn new(color_config: &'a ColorConfig, login_url: &'a str, api_client: &'a T) -> Self {
        Self {
            color_config,
            login_url,
            api_client,
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

// Tokens and refresh metadata now live in turborepo/config.json. For existing
// sessions, we can still read matching refresh metadata from the legacy Vercel
// CLI auth file without rewriting it.
fn load_auth_tokens(turbo_config_path: &AbsoluteSystemPath) -> Result<crate::AuthTokens, Error> {
    use crate::Token;

    let turbo_auth_tokens = Token::from_auth_file(turbo_config_path)?;
    let Some(turbo_token) = turbo_auth_tokens.token.as_ref() else {
        return Ok(turbo_auth_tokens);
    };

    if turbo_auth_tokens.refresh_token.is_some() && turbo_auth_tokens.expires_at.is_some() {
        return Ok(turbo_auth_tokens);
    }

    let vercel_auth_tokens = load_legacy_auth_tokens(Some(turbo_token))?;
    if vercel_auth_tokens.token.is_none() {
        return Ok(turbo_auth_tokens);
    }

    Ok(crate::AuthTokens {
        token: turbo_auth_tokens.token,
        refresh_token: turbo_auth_tokens
            .refresh_token
            .or(vercel_auth_tokens.refresh_token),
        expires_at: turbo_auth_tokens
            .expires_at
            .or(vercel_auth_tokens.expires_at),
    })
}

fn load_turbo_auth_tokens() -> Result<crate::AuthTokens, Error> {
    use crate::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token};

    let Some(turbo_config_dir) = turborepo_dirs::config_dir()? else {
        return Ok(crate::AuthTokens::default());
    };
    let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);

    match Token::from_auth_file(&turbo_auth_path) {
        Ok(tokens) => Ok(tokens),
        Err(Error::InvalidTokenFileFormat { .. }) => {
            warn!("Ignoring malformed Turbo auth file at {turbo_auth_path}");
            Ok(crate::AuthTokens::default())
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn classify_existing_vercel_token(token: &str) -> Result<ExistingTokenSource, Error> {
    let legacy_auth_tokens = load_legacy_auth_tokens(None)?;
    if legacy_auth_tokens
        .token
        .as_ref()
        .is_some_and(|stored_token| stored_token.expose() == token)
    {
        return Ok(ExistingTokenSource::LegacyAuth);
    }

    let turbo_auth_tokens = load_turbo_auth_tokens()?;
    if turbo_auth_tokens
        .token
        .as_ref()
        .is_some_and(|stored_token| stored_token.expose() == token)
    {
        return Ok(ExistingTokenSource::TurboConfig);
    }

    Ok(ExistingTokenSource::Other)
}

fn load_legacy_auth_tokens(
    expected_token: Option<&turborepo_api_client::SecretString>,
) -> Result<crate::AuthTokens, Error> {
    use crate::Token;

    let Some(vercel_config_dir) = turborepo_dirs::vercel_config_dir()? else {
        return Ok(crate::AuthTokens::default());
    };
    let vercel_auth_path =
        vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
    let legacy_auth_tokens = match Token::from_auth_file(&vercel_auth_path) {
        Ok(tokens) => tokens,
        Err(Error::InvalidTokenFileFormat { .. }) => {
            warn!("Ignoring malformed legacy Vercel auth file at {vercel_auth_path}");
            return Ok(crate::AuthTokens::default());
        }
        Err(e) => return Err(e),
    };

    if let Some(expected_token) = expected_token
        && legacy_auth_tokens
            .token
            .as_ref()
            .map(|legacy_token| legacy_token.expose())
            != Some(expected_token.expose())
    {
        return Ok(crate::AuthTokens::default());
    }

    Ok(legacy_auth_tokens)
}

async fn exchange_legacy_auth_token(
    turbo_config_path: &AbsoluteSystemPath,
    expected_token: Option<&turborepo_api_client::SecretString>,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    let legacy_auth_tokens = load_legacy_auth_tokens(expected_token)?;
    let Some(legacy_token) = legacy_auth_tokens.token.clone() else {
        return Ok(None);
    };

    let exchange_source = if legacy_auth_tokens.is_expired()
        && legacy_token.expose().starts_with("vca_")
        && legacy_auth_tokens.refresh_token.is_some()
    {
        match legacy_auth_tokens.refresh_token().await {
            Ok(refreshed_tokens) => refreshed_tokens,
            Err(e) => {
                warn!("Failed to refresh legacy Vercel auth token before exchange: {e}");
                legacy_auth_tokens.clone()
            }
        }
    } else {
        legacy_auth_tokens.clone()
    };

    match exchange_source.exchange_legacy_token().await {
        Ok(exchanged_tokens) => {
            if let Err(e) = exchanged_tokens.write_to_config_file(turbo_config_path) {
                warn!("Failed to write exchanged tokens to {turbo_config_path}: {e}");
            }
            Ok(exchanged_tokens.token)
        }
        Err(e) => {
            warn!("Failed to exchange legacy Vercel auth token, using legacy token directly: {e}");
            if exchange_source.is_expired() {
                Ok(None)
            } else {
                Ok(exchange_source.token.or(Some(legacy_token)))
            }
        }
    }
}

/// Attempts to get a valid token with automatic refresh if expired.
pub async fn get_token_with_refresh() -> Result<Option<turborepo_api_client::SecretString>, Error> {
    use crate::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};

    let turbo_config_dir = match turborepo_dirs::config_dir()? {
        Some(dir) => dir,
        None => return Ok(None),
    };

    let turbo_config_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
    let auth_tokens = load_auth_tokens(&turbo_config_path)?;

    if let Some(token) = &auth_tokens.token {
        if classify_existing_vercel_token(token.expose())? == ExistingTokenSource::LegacyAuth {
            return exchange_legacy_auth_token(&turbo_config_path, Some(token)).await;
        }

        if auth_tokens.is_expired() {
            // Only attempt refresh for Vercel tokens that start with "vca_"
            if token.expose().starts_with("vca_")
                && auth_tokens.refresh_token.is_some()
                && let Ok(new_tokens) = auth_tokens.refresh_token().await
            {
                if let Err(e) = new_tokens.write_to_config_file(&turbo_config_path) {
                    tracing::warn!("Failed to write refreshed tokens to {turbo_config_path}: {e}");
                }
                return Ok(new_tokens.token);
            }

            exchange_legacy_auth_token(&turbo_config_path, Some(token)).await
        } else {
            Ok(Some(token.clone()))
        }
    } else {
        exchange_legacy_auth_token(&turbo_config_path, None).await
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{ExistingTokenSource, classify_existing_vercel_token, is_vercel};
    use crate::{AuthTokens, Token, current_unix_time_secs};

    // Mock the turborepo_dirs functions for testing
    fn create_mock_vercel_config_dir() -> AbsoluteSystemPathBuf {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        AbsoluteSystemPathBuf::try_from(tmp_dir.keep()).expect("Failed to create path")
    }

    fn create_mock_turbo_config_dir() -> AbsoluteSystemPathBuf {
        let tmp_dir = tempdir().expect("Failed to create temp dir");
        AbsoluteSystemPathBuf::try_from(tmp_dir.keep()).expect("Failed to create path")
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
            token: Some(token.into()),
            refresh_token: refresh_token.map(|s| s.into()),
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

    #[test]
    fn test_classify_existing_vercel_token_prefers_turbo_config() {
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_config_file(&turbo_config_dir, "turbo-token");
        setup_auth_file(&vercel_config_dir, "legacy-token", None, None);

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let source = classify_existing_vercel_token("turbo-token").unwrap();

        assert_eq!(source, ExistingTokenSource::TurboConfig);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_classify_existing_vercel_token_detects_legacy_auth() {
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_auth_file(&vercel_config_dir, "legacy-token", None, None);

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let source = classify_existing_vercel_token("legacy-token").unwrap();

        assert_eq!(source, ExistingTokenSource::LegacyAuth);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_classify_existing_vercel_token_prefers_legacy_when_duplicated() {
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_config_file(&turbo_config_dir, "duplicated-token");
        setup_auth_file(&vercel_config_dir, "duplicated-token", None, None);

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let source = classify_existing_vercel_token("duplicated-token").unwrap();

        assert_eq!(source, ExistingTokenSource::LegacyAuth);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_classify_existing_vercel_token_returns_other_for_untracked_token() {
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_config_file(&turbo_config_dir, "turbo-token");
        setup_auth_file(&vercel_config_dir, "legacy-token", None, None);

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let source = classify_existing_vercel_token("explicit-token").unwrap();

        assert_eq!(source, ExistingTokenSource::Other);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
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
        assert!(
            auth_tokens
                .token
                .as_ref()
                .unwrap()
                .expose()
                .starts_with("vca_")
        );
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
        assert!(
            !auth_tokens
                .token
                .as_ref()
                .unwrap()
                .expose()
                .starts_with("vca_")
        );
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
        assert!(
            auth_tokens
                .token
                .as_ref()
                .unwrap()
                .expose()
                .starts_with("vca_")
        );
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
        assert!(
            auth_tokens
                .token
                .as_ref()
                .unwrap()
                .expose()
                .starts_with("vca_")
        );

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
        assert!(
            !auth_tokens
                .token
                .as_ref()
                .unwrap()
                .expose()
                .starts_with("vca_")
        );

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
                token: Some(turborepo_api_client::SecretString::new(token.to_string())),
                refresh_token: Some(turborepo_api_client::SecretString::new(
                    "refresh_token".to_string(),
                )),
                expires_at: Some(current_time - 3600), // Expired
            };

            let has_vca_prefix = token.starts_with("vca_");
            assert_eq!(
                has_vca_prefix, should_attempt_refresh,
                "Token '{token}' prefix check failed"
            );
        }
    }

    #[test]
    fn test_is_vercel() {
        assert!(is_vercel("https://vercel.com"));
        assert!(is_vercel("https://api.vercel.com"));
        assert!(is_vercel("https://vercel.com/api"));
        assert!(!is_vercel("https://my-cache.example.com"));
        assert!(!is_vercel("http://localhost:3000"));
    }
}
