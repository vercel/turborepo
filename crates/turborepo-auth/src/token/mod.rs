/*
 * Much of the login contained in `config_token` and things related to
 * conversion of tokens should be removed in the future. This is a stopgap
 * until we are confident or comfortable with making users re-login.
 */
mod auth_file;
mod config_token;

pub use auth_file::*;
pub use config_token::*;
use turborepo_api_client::Client;

use crate::error;

const DEFAULT_LOGIN_URL: &str = "https://vercel.com";
const DEFAULT_API_URL: &str = "https://vercel.com/api";

/// Fetches the token for a specific api from the auth file.
///
/// If the api is `vercel.com`, we attempt to use the Vercel CLI token.
///
/// The client is used to convert the legacy token to our new format.
pub async fn get_token_for(api: &str, client: &impl Client) -> Result<AuthToken, crate::Error> {
    // NOTE: In the future this will look at the Vercel CLI to see if there's a
    // valid token in there instead of our auth.
    //
    // For now, we'll just use our auth file. But the logic should be very similar
    // going forward.
    let tokens: Vec<AuthToken> = if api == DEFAULT_LOGIN_URL || api == DEFAULT_API_URL {
        load_vercel_tokens(client).await?
    } else {
        load_turbo_tokens(client).await?
    };

    // Return found token.
    if let Some(token) = tokens.iter().find(|t| t.api == api) {
        println!("Found token for api: {}", api);
        // TODO: I don't like that we're cloning the whole token here. Might be
        //       beneficial to rethink or force an AuthToken to be passed in and
        //       overwritten.
        Ok(token.clone())
    } else {
        // Couldn't find a token, bomb out.
        Err(crate::Error::FailedToFindTokenForAPI {
            api: api.to_string(),
        })
    }
}

/// If --api is passed in and it's not the default Vercel api, it means we're
/// looking for a non-Vercel remote cache token.
///
/// This function only loads tokens
/// from auth.json and if it doesn't exist, creates the `auth.json` from a
/// `config.json`.
pub async fn load_turbo_tokens(client: &impl Client) -> Result<Vec<AuthToken>, crate::Error> {
    match read_auth_file() {
        // We found our `auth.json`, so use that.
        Ok(auth_file) => Ok(auth_file.tokens),
        // TODO(voz): Surely there's a nicer way to accomplish this?
        Err(crate::Error::FailedToFindConfigDir) => Err(crate::Error::FailedToFindConfigDir),
        // If we found the config directory but not the auth file, it means we need to check the
        // `config.json` for a token.
        Err(e) => {
            let result = async {
                match read_config_auth() {
                    Ok(config_token) => convert_to_auth_file(&config_token.token, client)
                        .await
                        .map_err(error::Error::from),
                    Err(_) => Err(e),
                }
            }
            .await;

            match result {
                Ok(auth_file) => Ok(auth_file.tokens),
                Err(e) => Err(e),
            }
        }
    }
}

// If --api is passed in and it's the default Vercel api or if --api isn't
// passed in at all.
pub async fn load_vercel_tokens(client: &impl Client) -> Result<Vec<AuthToken>, crate::Error> {
    // TODO: Until we get the vercel auth token in com.vercel.cli to look like our
    // format, make this a passthrough for load_turbo_tokens.
    load_turbo_tokens(client).await
}
