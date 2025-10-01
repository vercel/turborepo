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

fn extract_vercel_token() -> Result<Option<String>, Error> {
    let vercel_config_dir =
        turborepo_dirs::vercel_config_dir()?.ok_or_else(|| Error::ConfigDirNotFound)?;

    let vercel_token_path =
        vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
    let contents = std::fs::read_to_string(vercel_token_path)?;

    #[derive(serde::Deserialize)]
    struct VercelToken {
        // This isn't actually dead code, it's used by serde to deserialize the JSON.
        #[allow(dead_code)]
        token: Option<String>,
    }

    Ok(serde_json::from_str::<VercelToken>(&contents)?.token)
}

/// Attempts to get a valid token with automatic refresh if expired.
/// Falls back to turborepo/config.json if refresh fails.
pub async fn get_token_with_refresh() -> Result<Option<String>, Error> {
    use crate::{TURBO_TOKEN_DIR, TURBO_TOKEN_FILE, Token};

    let vercel_config_dir = match turborepo_dirs::vercel_config_dir()? {
        Some(dir) => dir,
        None => {
            tracing::debug!("No Vercel config directory found");
            return Ok(None);
        }
    };

    let auth_path = vercel_config_dir.join_components(&[VERCEL_TOKEN_DIR, VERCEL_TOKEN_FILE]);
    tracing::debug!("Reading auth tokens from: {}", auth_path);

    // Try to read auth.json with token and refresh token
    let auth_tokens = Token::from_auth_file(&auth_path)?;
    tracing::debug!(
        "Auth tokens loaded - has token: {}, has refresh: {}, expires_at: {:?}",
        auth_tokens.token.is_some(),
        auth_tokens.refresh_token.is_some(),
        auth_tokens.expires_at
    );

    println!("hey hiiiii howdy");
    // If we have a token
    if let Some(token) = &auth_tokens.token {
        // Check if token is expired
        if auth_tokens.is_expired() {
            println!("hey hi howdy");
            // Try to refresh the token
            if auth_tokens.refresh_token.is_some() {
                match auth_tokens.refresh_token().await {
                    Ok(new_tokens) => {
                        tracing::info!("Successfully refreshed token");
                        // Write the new tokens back to auth.json
                        if let Err(e) = new_tokens.write_to_auth_file(&auth_path) {
                            tracing::warn!("Failed to write refreshed token to auth.json: {}", e);
                        } else {
                            tracing::info!("Updated auth.json with new tokens");
                        }
                        return Ok(new_tokens.token);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to refresh token: {}", e);
                        // Fall through to try turborepo/config.json
                    }
                }
            }

            // Token expired and refresh failed, try turborepo/config.json
            tracing::info!("Attempting to fall back to turborepo/config.json");
            if let Ok(Some(config_dir)) = turborepo_dirs::config_dir() {
                let turbo_config_path =
                    config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_TOKEN_FILE]);
                tracing::debug!("Checking for fallback token at: {}", turbo_config_path);
                if let Ok(turbo_token) = Token::from_file(&turbo_config_path) {
                    tracing::info!("Found valid fallback token in turborepo/config.json");
                    return Ok(Some(turbo_token.into_inner().to_string()));
                } else {
                    tracing::debug!("No valid token found in turborepo/config.json");
                }
            } else {
                tracing::debug!("Could not locate config directory for fallback");
            }

            // No valid fallback token found
            tracing::warn!("No valid token found after refresh attempt and fallback");
            Ok(None)
        } else {
            // Token is not expired, use it
            tracing::debug!("Token is still valid, using existing token");
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
