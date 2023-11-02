/*
 * Much of the login contained in `config_token` and things related to
 * conversion of tokens should be removed in the future. This is a stopgap
 * until we are confident or comfortable with making users re-login.
 */
mod auth_file;
mod config_token;

pub use auth_file::*;
pub use config_token::*;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_api_client::Client;

use crate::error;

/// If `--api` is passed in and it's not the default Vercel api, it means we're
/// looking for a non-Vercel remote cache token.
///
/// This function only loads tokens
/// from auth.json and if it doesn't exist, creates the `auth.json` from a
/// `config.json`.
pub async fn load_turbo_tokens(
    client: &impl Client,
    // tokens_file_path is mostly here for testing purposes.
    auth_file_path: &AbsoluteSystemPathBuf,
) -> Result<AuthFile, crate::Error> {
    match read_auth_file(auth_file_path) {
        // We found our `auth.json`, so use that.
        Ok(auth_file) => Ok(auth_file),
        // We did not find the auth file, it means we need to check the
        // `config.json` for a token.
        Err(e) => {
            async {
                match read_config_auth(auth_file_path) {
                    // Convert the found config token to an auth file.
                    Ok(config_token) => {
                        convert_to_auth_file(&config_token.token, client, auth_file_path)
                            .await
                            .map_err(error::Error::from)
                    }
                    // No config either, bomb out.
                    Err(_) => Err(e),
                }
            }
            .await
        }
    }
}

// If --api is passed in and it's the default Vercel api or if --api isn't
// passed in at all.
pub async fn load_vercel_tokens(
    client: &impl Client,
    auth_dir_path: &AbsoluteSystemPathBuf,
) -> Result<AuthFile, crate::Error> {
    // TODO: Until we get the vercel auth token in com.vercel.cli to look like our
    // format, make this a passthrough for load_turbo_tokens.
    load_turbo_tokens(client, auth_dir_path).await
}

pub async fn read_or_create_auth_file(
    auth_file_path: &AbsoluteSystemPathBuf,
    config_file_path: &AbsoluteSystemPathBuf,
    client: &impl Client,
) -> Result<AuthFile, crate::Error> {
    if auth_file_path.exists() {
        let content = auth_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(crate::Error::FailedToReadAuthFile)?;
        let auth_file: AuthFile = serde_json::from_str(&content)
            .map_err(|e| crate::Error::FailedToSerializeAuthFile { error: e })?;
        return Ok(auth_file);
    } else if config_file_path.exists() {
        let content = config_file_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(crate::Error::FailedToReadConfigFile)?;
        let config_token: ConfigToken = serde_json::from_str(&content)
            .map_err(|e| crate::Error::FailedToSerializeAuthFile { error: e })?;

        return convert_to_auth_file(&config_token.token, client, auth_file_path).await;
    }
    // If neither file exists, return an empty auth file.
    Ok(AuthFile { tokens: Vec::new() })
}
