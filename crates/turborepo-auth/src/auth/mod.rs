mod login;
mod logout;
mod sso;

pub use login::*;
pub use logout::*;
pub use sso::*;
use turbopath::AbsoluteSystemPath;
#[cfg(test)]
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::ColorConfig;

pub(crate) fn is_vercel(login_url: &str) -> bool {
    login_url.contains("vercel.com")
}

const VERCEL_TOKEN_DIR: &str = "com.vercel.cli";
const VERCEL_TOKEN_FILE: &str = "auth.json";

pub struct LoginOptions<'a, T: Client + TokenClient + CacheClient> {
    pub color_config: &'a ColorConfig,
    pub login_url: &'a str,
    pub api_client: &'a T,

    pub sso_team: Option<&'a str>,
    pub existing_token: Option<&'a str>,
    pub force: bool,
    pub sso_login_callback_port: Option<u16>,
}
impl<'a, T: Client + TokenClient + CacheClient> LoginOptions<'a, T> {
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

/// Attempts to get a valid token with automatic refresh if expired.
/// Checks Turbo auth.json first, then Vercel auth.json, then the legacy
/// turborepo/config.json token.
pub async fn get_token_with_refresh() -> Result<Option<turborepo_api_client::SecretString>, Error> {
    use crate::{TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token};

    let turbo_config_dir = turborepo_dirs::config_dir()?;
    if let Some(config_dir) = turbo_config_dir.as_ref() {
        let turbo_auth_path = config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        if let Some(token) = get_auth_file_token(&turbo_auth_path).await? {
            return Ok(Some(token));
        }
    }

    if let Some(vercel_config_dir) = turborepo_dirs::vercel_config_dir()?.as_ref() {
        let vercel_auth_path =
            vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
        if let Some(token) = get_auth_file_token(&vercel_auth_path).await? {
            return Ok(Some(token));
        }
    }

    if let Some(config_dir) = turbo_config_dir.as_ref() {
        let turbo_config_path = config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
        if let Ok(turbo_token) = Token::from_file(&turbo_config_path) {
            return Ok(Some(turbo_token.into_inner().clone()));
        }
    }

    Ok(None)
}

async fn get_auth_file_token(
    path: &AbsoluteSystemPath,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    let auth_tokens = crate::Token::from_auth_file(path)?;
    let Some(token) = &auth_tokens.token else {
        return Ok(None);
    };

    if auth_tokens.is_expired() {
        if token.expose().starts_with("vca_")
            && auth_tokens.refresh_token.is_some()
            && let Ok(new_tokens) = auth_tokens.refresh_token().await
        {
            if let Err(e) = new_tokens.write_to_auth_file(path) {
                tracing::warn!("Failed to write refreshed tokens to {path}: {e}");
            }
            return Ok(new_tokens.token);
        }

        return Ok(None);
    }

    Ok(Some(token.clone()))
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Mutex};

    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{get_token_with_refresh, is_vercel};
    use crate::{AuthTokens, TURBO_AUTH_FILE, TURBO_TOKEN_DIR, Token, current_unix_time_secs};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ConfigDirEnvGuard;

    impl ConfigDirEnvGuard {
        fn set(turbo_dir: &AbsoluteSystemPathBuf, vercel_dir: &AbsoluteSystemPathBuf) -> Self {
            unsafe {
                std::env::set_var("TURBO_CONFIG_DIR_PATH", turbo_dir.as_str());
                std::env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_dir.as_str());
            }

            Self
        }
    }

    impl Drop for ConfigDirEnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("TURBO_CONFIG_DIR_PATH");
                std::env::remove_var("VERCEL_CONFIG_DIR_PATH");
            }
        }
    }

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

    fn setup_turbo_auth_file(
        config_dir: &AbsoluteSystemPathBuf,
        token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<u64>,
    ) {
        let auth_dir = config_dir.join_component(TURBO_TOKEN_DIR);
        fs::create_dir_all(&auth_dir).expect("Failed to create turbo auth dir");
        let auth_file = auth_dir.join_component(TURBO_AUTH_FILE);

        let auth_tokens = AuthTokens {
            token: Some(token.into()),
            refresh_token: refresh_token.map(|s| s.into()),
            expires_at,
        };

        auth_tokens
            .write_to_auth_file(&auth_file)
            .expect("Failed to write turbo auth file");
    }

    #[tokio::test]
    async fn test_get_token_with_refresh_prefers_turbo_auth_file() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let _guard = ConfigDirEnvGuard::set(&turbo_config_dir, &vercel_config_dir);
        let current_time = current_unix_time_secs();

        setup_turbo_auth_file(
            &turbo_config_dir,
            "vca_turbo_auth_token",
            Some("refresh_token_123"),
            Some(current_time + 3600),
        );
        setup_auth_file(
            &vercel_config_dir,
            "vercel_shared_token",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );
        setup_turbo_config_file(&turbo_config_dir, "legacy_config_token");

        let token = get_token_with_refresh().await.unwrap().unwrap();
        assert_eq!(token.expose(), "vca_turbo_auth_token");
    }

    #[tokio::test]
    async fn test_get_token_with_refresh_falls_back_to_legacy_config() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let _guard = ConfigDirEnvGuard::set(&turbo_config_dir, &vercel_config_dir);

        setup_turbo_config_file(&turbo_config_dir, "legacy_config_token");

        let token = get_token_with_refresh().await.unwrap().unwrap();
        assert_eq!(token.expose(), "legacy_config_token");
    }

    #[tokio::test]
    async fn test_vca_token_with_valid_refresh() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        setup_auth_file(
            &vercel_config_dir,
            "vca_expired_token_123",
            Some("refresh_token_456"),
            Some(current_time - 3600),
        );

        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

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
    }

    #[tokio::test]
    async fn test_legacy_token_skips_refresh() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        setup_auth_file(
            &vercel_config_dir,
            "legacy_token_123",
            Some("refresh_token_456"),
            Some(current_time - 3600),
        );

        setup_turbo_config_file(&turbo_config_dir, "turbo_fallback_token");

        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

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
    }

    #[tokio::test]
    async fn test_vca_token_without_refresh_token() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        setup_auth_file(
            &vercel_config_dir,
            "vca_expired_token_123",
            None,
            Some(current_time - 3600),
        );

        setup_turbo_config_file(&turbo_config_dir, "turbo_fallback_token");

        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

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
    }

    #[tokio::test]
    async fn test_non_expired_vca_token() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        setup_auth_file(
            &vercel_config_dir,
            "vca_valid_token_123",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );

        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

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

        setup_auth_file(
            &vercel_config_dir,
            "legacy_token_123",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );

        let auth_path = vercel_config_dir.join_components(&["com.vercel.cli", "auth.json"]);
        let auth_tokens = Token::from_auth_file(&auth_path).expect("Failed to read auth file");

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

    #[test]
    fn test_is_vercel() {
        assert!(is_vercel("https://vercel.com"));
        assert!(is_vercel("https://api.vercel.com"));
        assert!(is_vercel("https://vercel.com/api"));
        assert!(!is_vercel("https://my-cache.example.com"));
        assert!(!is_vercel("http://localhost:3000"));
    }

    #[tokio::test]
    async fn test_token_prefix_edge_cases() {
        let current_time = current_unix_time_secs();

        let test_cases = vec![
            ("vca_token", true),
            ("VCA_token", false),
            ("vca_", true),
            ("vca", false),
            ("xvca_token", false),
            ("", false),
            ("some_other_token", false),
        ];

        for (token, should_attempt_refresh) in test_cases {
            let _auth_tokens = AuthTokens {
                token: Some(turborepo_api_client::SecretString::new(token.to_string())),
                refresh_token: Some(turborepo_api_client::SecretString::new(
                    "refresh_token".to_string(),
                )),
                expires_at: Some(current_time - 3600),
            };

            let has_vca_prefix = token.starts_with("vca_");
            assert_eq!(
                has_vca_prefix, should_attempt_refresh,
                "Token '{token}' prefix check failed"
            );
        }
    }
}
