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
            // Try to refresh the token
            if auth_tokens.refresh_token.is_some()
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
