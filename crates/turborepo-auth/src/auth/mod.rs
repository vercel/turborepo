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

// Device-flow sessions live in Turbo's auth.json. We still fall back to the
// legacy config.json slot for non-OAuth tokens and older persisted sessions.
fn load_auth_tokens(
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
) -> Result<crate::AuthTokens, Error> {
    let turbo_auth_tokens = load_turbo_auth_tokens_from_paths(turbo_auth_path, turbo_config_path)?;
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

fn load_tokens_from_path(
    path: &AbsoluteSystemPath,
    label: &str,
) -> Result<Option<crate::AuthTokens>, Error> {
    use crate::Token;

    match Token::from_auth_file(path) {
        Ok(tokens) if tokens.token.is_some() => Ok(Some(tokens)),
        Ok(_) | Err(Error::TokenNotFound) => Ok(None),
        Err(Error::InvalidTokenFileFormat { .. }) => {
            warn!("Ignoring malformed {label} at {path}");
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

fn path_contains_token(
    path: &AbsoluteSystemPath,
    label: &str,
    expected_token: &str,
) -> Result<bool, Error> {
    Ok(load_tokens_from_path(path, label)?
        .and_then(|tokens| tokens.token)
        .is_some_and(|stored_token| stored_token.expose() == expected_token))
}

fn load_turbo_auth_tokens_from_paths(
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
) -> Result<crate::AuthTokens, Error> {
    if let Some(tokens) = load_tokens_from_path(turbo_auth_path, "Turbo auth file")? {
        return Ok(tokens);
    }

    Ok(load_tokens_from_path(turbo_config_path, "Turbo config file")?.unwrap_or_default())
}

fn load_turbo_auth_tokens() -> Result<crate::AuthTokens, Error> {
    use crate::{TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};

    let Some(turbo_config_dir) = turborepo_dirs::config_dir()? else {
        return Ok(crate::AuthTokens::default());
    };
    let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
    let turbo_config_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);

    load_turbo_auth_tokens_from_paths(&turbo_auth_path, &turbo_config_path)
}

pub(crate) fn classify_existing_vercel_token(token: &str) -> Result<ExistingTokenSource, Error> {
    if let Some(turbo_config_dir) = turborepo_dirs::config_dir()? {
        let turbo_auth_path =
            turbo_config_dir.join_components(&[crate::TURBO_TOKEN_DIR, crate::TURBO_AUTH_FILE]);
        if let Some(turbo_auth_tokens) = load_tokens_from_path(&turbo_auth_path, "Turbo auth file")?
            && turbo_auth_tokens
                .token
                .as_ref()
                .is_some_and(|stored_token| stored_token.expose() == token)
        {
            return Ok(ExistingTokenSource::TurboConfig);
        }
    }

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
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
    expected_token: Option<&turborepo_api_client::SecretString>,
    allow_legacy_token_fallback: bool,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    let legacy_auth_tokens = load_legacy_auth_tokens(expected_token)?;
    exchange_auth_tokens(
        &legacy_auth_tokens,
        turbo_auth_path,
        turbo_config_path,
        allow_legacy_token_fallback,
        "legacy Vercel auth token",
    )
    .await
}

fn can_refresh_token(auth_tokens: &crate::AuthTokens) -> bool {
    // Recovery may run after the access token has been corrupted or rejected,
    // so refreshability has to be derived from the stored refresh token rather
    // than the access-token prefix.
    auth_tokens.refresh_token.is_some()
}

fn should_exchange_turbo_config_token(
    auth_tokens: &crate::AuthTokens,
    turbo_auth_path: &AbsoluteSystemPath,
) -> Result<bool, Error> {
    let Some(token) = auth_tokens.token.as_ref() else {
        return Ok(false);
    };

    if token.expose().starts_with("vca_")
        || auth_tokens.refresh_token.is_some()
        || auth_tokens.expires_at.is_some()
    {
        return Ok(false);
    }

    Ok(!path_contains_token(
        turbo_auth_path,
        "Turbo auth file",
        token.expose(),
    )?)
}

async fn exchange_auth_tokens(
    auth_tokens: &crate::AuthTokens,
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
    allow_token_fallback: bool,
    source_label: &str,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    let Some(stored_token) = auth_tokens.token.clone() else {
        return Ok(None);
    };

    let should_refresh_before_exchange =
        can_refresh_token(auth_tokens) && (auth_tokens.is_expired() || !allow_token_fallback);
    let mut refresh_error = None;

    let exchange_source = if should_refresh_before_exchange {
        match auth_tokens.refresh_token().await {
            Ok(refreshed_tokens) => refreshed_tokens,
            Err(e) => {
                refresh_error = Some(e);
                auth_tokens.clone()
            }
        }
    } else {
        auth_tokens.clone()
    };

    match exchange_source.exchange_legacy_token().await {
        Ok(exchanged_tokens) => {
            if let Err(e) =
                persist_turbo_oauth_tokens(&exchanged_tokens, turbo_auth_path, turbo_config_path)
            {
                warn!(
                    "Failed to write exchanged tokens to {turbo_auth_path} and clear \
                     {turbo_config_path}: {e}"
                );
            }
            Ok(exchanged_tokens.token)
        }
        Err(e) => {
            if allow_token_fallback {
                if let Some(refresh_error) = refresh_error {
                    warn!(
                        "Failed to refresh or exchange {source_label}, using stored token \
                         directly: refresh error: {refresh_error}; exchange error: {e}"
                    );
                } else {
                    warn!("Failed to exchange {source_label}, using stored token directly: {e}");
                }
                if exchange_source.is_expired() {
                    Ok(None)
                } else {
                    Ok(exchange_source.token.or(Some(stored_token)))
                }
            } else {
                if let Some(refresh_error) = refresh_error {
                    warn!(
                        "Failed to refresh or exchange {source_label} after a forbidden response: \
                         refresh error: {refresh_error}; exchange error: {e}"
                    );
                } else {
                    warn!("Failed to exchange {source_label} after a forbidden response: {e}");
                }
                Ok(None)
            }
        }
    }
}

async fn refresh_and_persist_turbo_token(
    auth_tokens: &crate::AuthTokens,
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
) -> Option<turborepo_api_client::SecretString> {
    if !can_refresh_token(auth_tokens) {
        return None;
    }

    match auth_tokens.refresh_token().await {
        Ok(new_tokens) => {
            if let Err(e) =
                persist_turbo_oauth_tokens(&new_tokens, turbo_auth_path, turbo_config_path)
            {
                warn!(
                    "Failed to write refreshed tokens to {turbo_auth_path} and clear \
                     {turbo_config_path}: {e}"
                );
            }
            new_tokens.token
        }
        Err(e) => {
            warn!("Failed to refresh stored Vercel auth token: {e}");
            None
        }
    }
}

fn persist_turbo_oauth_tokens(
    auth_tokens: &crate::AuthTokens,
    turbo_auth_path: &AbsoluteSystemPath,
    turbo_config_path: &AbsoluteSystemPath,
) -> Result<(), Error> {
    auth_tokens.write_to_config_file(turbo_auth_path)?;
    crate::AuthTokens::clear_from_config_file(turbo_config_path)?;
    Ok(())
}

async fn get_token_with_refresh_inner(
    allow_legacy_token_fallback: bool,
    allow_turbo_config_token_fallback: bool,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    use crate::{TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};

    let turbo_config_dir = match turborepo_dirs::config_dir()? {
        Some(dir) => dir,
        None => return Ok(None),
    };

    let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
    let turbo_config_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
    let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path)?;

    if let Some(token) = &auth_tokens.token {
        if classify_existing_vercel_token(token.expose())? == ExistingTokenSource::LegacyAuth {
            return exchange_legacy_auth_token(
                &turbo_auth_path,
                &turbo_config_path,
                Some(token),
                allow_legacy_token_fallback,
            )
            .await;
        }

        if should_exchange_turbo_config_token(&auth_tokens, &turbo_auth_path)? {
            return exchange_auth_tokens(
                &auth_tokens,
                &turbo_auth_path,
                &turbo_config_path,
                allow_turbo_config_token_fallback,
                "Turbo config token",
            )
            .await;
        }

        if auth_tokens.is_expired() {
            if let Some(refreshed_token) =
                refresh_and_persist_turbo_token(&auth_tokens, &turbo_auth_path, &turbo_config_path)
                    .await
            {
                return Ok(Some(refreshed_token));
            }

            exchange_legacy_auth_token(
                &turbo_auth_path,
                &turbo_config_path,
                Some(token),
                allow_legacy_token_fallback,
            )
            .await
        } else {
            Ok(Some(token.clone()))
        }
    } else {
        exchange_legacy_auth_token(
            &turbo_auth_path,
            &turbo_config_path,
            None,
            allow_legacy_token_fallback,
        )
        .await
    }
}

/// Attempts to get a valid token with automatic refresh if expired.
pub async fn get_token_with_refresh() -> Result<Option<turborepo_api_client::SecretString>, Error> {
    get_token_with_refresh_inner(true, true).await
}

/// Login prefers upgrading legacy/config tokens into Turbo OAuth sessions. If
/// that upgrade fails, callers should fall through to a fresh login instead of
/// silently reusing the old token.
pub async fn get_token_with_refresh_for_login()
-> Result<Option<turborepo_api_client::SecretString>, Error> {
    get_token_with_refresh_inner(false, false).await
}

/// Attempts to recover a replacement token after the current token was rejected
/// by the server. Unlike `get_token_with_refresh`, this never falls back to the
/// same stored token.
pub async fn recover_token_after_forbidden(
    current_token: &turborepo_api_client::SecretString,
) -> Result<Option<turborepo_api_client::SecretString>, Error> {
    use crate::{TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE};

    let turbo_config_dir = match turborepo_dirs::config_dir()? {
        Some(dir) => dir,
        None => return Ok(None),
    };

    let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
    let turbo_config_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);

    match classify_existing_vercel_token(current_token.expose())? {
        ExistingTokenSource::LegacyAuth => {
            exchange_legacy_auth_token(
                &turbo_auth_path,
                &turbo_config_path,
                Some(current_token),
                false,
            )
            .await
        }
        ExistingTokenSource::TurboConfig => {
            let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path)?;
            if auth_tokens.token.as_ref().map(|token| token.expose())
                != Some(current_token.expose())
            {
                return Ok(None);
            }

            if should_exchange_turbo_config_token(&auth_tokens, &turbo_auth_path)? {
                return exchange_auth_tokens(
                    &auth_tokens,
                    &turbo_auth_path,
                    &turbo_config_path,
                    false,
                    "Turbo config token",
                )
                .await;
            }

            if let Some(refreshed_token) =
                refresh_and_persist_turbo_token(&auth_tokens, &turbo_auth_path, &turbo_config_path)
                    .await
                && refreshed_token.expose() != current_token.expose()
            {
                return Ok(Some(refreshed_token));
            }

            exchange_legacy_auth_token(
                &turbo_auth_path,
                &turbo_config_path,
                Some(current_token),
                false,
            )
            .await
        }
        ExistingTokenSource::Other => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use tempfile::tempdir;
    use tokio::sync::Mutex;
    use turbopath::AbsoluteSystemPathBuf;

    use super::{
        ExistingTokenSource, can_refresh_token, classify_existing_vercel_token,
        get_token_with_refresh, is_vercel, load_auth_tokens, recover_token_after_forbidden,
        should_exchange_turbo_config_token,
    };
    use crate::{
        AuthTokens, TURBO_AUTH_FILE, TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token,
        current_unix_time_secs,
    };

    static ENV_LOCK: Mutex<()> = Mutex::const_new(());

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
        let turbo_dir = config_dir.join_component(TURBO_TOKEN_DIR);
        fs::create_dir_all(&turbo_dir).expect("Failed to create turbo dir");
        let auth_file = turbo_dir.join_component(TURBO_AUTH_FILE);

        let auth_tokens = AuthTokens {
            token: Some(token.into()),
            refresh_token: refresh_token.map(|s| s.into()),
            expires_at,
        };

        auth_tokens
            .write_to_config_file(&auth_file)
            .expect("Failed to write turbo auth file");
    }

    #[test]
    fn test_classify_existing_vercel_token_prefers_turbo_config() {
        let _lock = ENV_LOCK.blocking_lock();
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
        let _lock = ENV_LOCK.blocking_lock();
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
    fn test_classify_existing_vercel_token_prefers_turbo_auth_file() {
        let _lock = ENV_LOCK.blocking_lock();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_auth_file(&turbo_config_dir, "duplicated-token", None, None);
        setup_auth_file(&vercel_config_dir, "duplicated-token", None, None);

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let source = classify_existing_vercel_token("duplicated-token").unwrap();

        assert_eq!(source, ExistingTokenSource::TurboConfig);

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_classify_existing_vercel_token_prefers_legacy_when_duplicated() {
        let _lock = ENV_LOCK.blocking_lock();
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
        let _lock = ENV_LOCK.blocking_lock();
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

    #[test]
    fn test_should_exchange_turbo_config_legacy_token() {
        let _lock = ENV_LOCK.blocking_lock();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_config_file(&turbo_config_dir, "vcp_legacy_token");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        let turbo_config_path =
            turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
        let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path).unwrap();

        assert!(should_exchange_turbo_config_token(&auth_tokens, &turbo_auth_path).unwrap());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_should_not_exchange_turbo_auth_oauth_token() {
        let _lock = ENV_LOCK.blocking_lock();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();

        setup_turbo_auth_file(
            &turbo_config_dir,
            "vca_oauth_token",
            Some("refresh_token_123"),
            Some(current_unix_time_secs() + 3600),
        );

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        let turbo_config_path =
            turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
        let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path).unwrap();

        assert!(!should_exchange_turbo_config_token(&auth_tokens, &turbo_auth_path).unwrap());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[tokio::test]
    async fn test_get_token_with_refresh_prefers_turbo_auth_file() {
        let _lock = ENV_LOCK.lock().await;
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        setup_turbo_auth_file(
            &turbo_config_dir,
            "vca_turbo_auth_token",
            Some("refresh_token_123"),
            Some(current_time + 3600),
        );
        setup_turbo_config_file(&turbo_config_dir, "legacy_config_token");
        setup_auth_file(
            &vercel_config_dir,
            "legacy_auth_token",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let token = get_token_with_refresh().await.unwrap().unwrap();

        assert_eq!(token.expose(), "vca_turbo_auth_token");

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[tokio::test]
    async fn test_get_token_with_refresh_falls_back_to_legacy_config() {
        let _lock = ENV_LOCK.lock().await;
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();

        setup_turbo_config_file(&turbo_config_dir, "legacy_config_token");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let token = get_token_with_refresh().await.unwrap().unwrap();

        assert_eq!(token.expose(), "legacy_config_token");

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_load_auth_tokens_backfills_matching_legacy_metadata() {
        let _lock = ENV_LOCK.blocking_lock();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        setup_turbo_auth_file(&turbo_config_dir, "shared-token", None, None);
        setup_auth_file(
            &vercel_config_dir,
            "shared-token",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        let turbo_config_path =
            turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
        let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path).unwrap();

        assert_eq!(
            auth_tokens.token.as_ref().map(|token| token.expose()),
            Some("shared-token")
        );
        assert_eq!(
            auth_tokens
                .refresh_token
                .as_ref()
                .map(|token| token.expose()),
            Some("refresh_token_456")
        );
        assert_eq!(auth_tokens.expires_at, Some(current_time + 3600));

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[test]
    fn test_load_auth_tokens_ignores_mismatched_legacy_metadata() {
        let _lock = ENV_LOCK.blocking_lock();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        setup_turbo_auth_file(&turbo_config_dir, "turbo-token", None, None);
        setup_auth_file(
            &vercel_config_dir,
            "other-token",
            Some("refresh_token_456"),
            Some(current_time + 3600),
        );

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let turbo_auth_path = turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        let turbo_config_path =
            turbo_config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
        let auth_tokens = load_auth_tokens(&turbo_auth_path, &turbo_config_path).unwrap();

        assert_eq!(
            auth_tokens.token.as_ref().map(|token| token.expose()),
            Some("turbo-token")
        );
        assert!(auth_tokens.refresh_token.is_none());
        assert!(auth_tokens.expires_at.is_none());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[tokio::test]
    async fn test_recover_token_after_forbidden_ignores_untracked_token() {
        let _lock = ENV_LOCK.lock().await;
        let turbo_config_dir = create_mock_turbo_config_dir();
        let vercel_config_dir = create_mock_vercel_config_dir();
        let current_time = current_unix_time_secs();

        setup_turbo_auth_file(
            &turbo_config_dir,
            "stored-token",
            Some("refresh_token_123"),
            Some(current_time + 3600),
        );

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", turbo_config_dir.as_path());
            env::set_var("VERCEL_CONFIG_DIR_PATH", vercel_config_dir.as_path());
        }

        let recovered = recover_token_after_forbidden(&turborepo_api_client::SecretString::new(
            "other-token".to_string(),
        ))
        .await
        .unwrap();

        assert!(recovered.is_none());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
            env::remove_var("VERCEL_CONFIG_DIR_PATH");
        }
    }

    #[tokio::test]
    async fn test_expired_token_with_refresh_token_is_refreshable() {
        // This focuses on the local gating logic. The HTTP refresh itself is
        // covered elsewhere.

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

        // Any expired token with a refresh token should be refreshable, even if
        // the access token itself is later corrupted.
        assert!(auth_tokens.is_expired());
        assert!(auth_tokens.refresh_token.is_some());
        assert!(can_refresh_token(&auth_tokens));
    }

    #[tokio::test]
    async fn test_non_vca_token_with_refresh_token_is_still_refreshable() {
        let vercel_config_dir = create_mock_vercel_config_dir();
        let turbo_config_dir = create_mock_turbo_config_dir();
        let current_time = current_unix_time_secs();

        // Setup expired non-vca token with refresh token
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

        // Recovery should use the refresh token rather than relying on the
        // current access-token prefix.
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
        assert!(can_refresh_token(&auth_tokens));
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
    async fn test_refreshability_depends_on_refresh_token_not_access_token_prefix() {
        for token in ["vca_token", "legacy_token", "corrupted!!!", ""] {
            let auth_tokens = AuthTokens {
                token: Some(turborepo_api_client::SecretString::new(token.to_string())),
                refresh_token: Some(turborepo_api_client::SecretString::new(
                    "refresh_token".to_string(),
                )),
                expires_at: Some(current_unix_time_secs() - 3600),
            };

            assert!(
                can_refresh_token(&auth_tokens),
                "Token '{token}' should be refreshable"
            );
        }

        let auth_tokens = AuthTokens {
            token: Some(turborepo_api_client::SecretString::new(
                "vca_token".to_string(),
            )),
            refresh_token: None,
            expires_at: Some(current_unix_time_secs() - 3600),
        };

        assert!(!can_refresh_token(&auth_tokens));
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
