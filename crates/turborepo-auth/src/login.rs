use std::sync::Arc;

pub use error::Error;
use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::warn;
use turborepo_api_client::Client;
use turborepo_ui::{start_spinner, UI};

use crate::{convert_to_auth_token, error, ui, AuthToken, LoginServer};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

/// Fetches a raw token from the login server and converts it to an
/// AuthToken.
pub async fn login(
    api_client: &impl Client,
    ui: &UI,
    login_url_configuration: &str,
    login_server: &impl LoginServer,
) -> Result<AuthToken, Error> {
    let login_url = build_login_url(login_url_configuration)?;

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");

    // Try to open browser for auth confirmation.
    let url = login_url.as_str();
    if login_server.open_web_browser(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let token_cell = Arc::new(OnceCell::new());
    login_server
        .run(
            DEFAULT_PORT,
            login_url_configuration.to_string(),
            token_cell.clone(),
        )
        .await?;

    spinner.finish_and_clear();

    let token = token_cell.get().ok_or(Error::FailedToGetToken)?;
    let auth_token = convert_to_auth_token(token, api_client);
    let response_user = api_client.get_user(&auth_token.token).await?;

    ui::print_cli_authorized(&response_user.user.email, ui);

    Ok(auth_token)
}

fn build_login_url(config: &str) -> Result<Url, Error> {
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let mut login_url = Url::parse(config).map_err(Error::UrlParseError)?;

    login_url
        .path_segments_mut()
        .map_err(|_: ()| Error::LoginUrlCannotBeABase {
            value: config.to_string(),
        })?
        .extend(["turborepo", "token"]);

    login_url
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect_url);

    Ok(login_url)
}

#[cfg(test)]
mod tests {
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::mocks::*;

    #[tokio::test]
    async fn test_login() {
        // Setup: Start login server on separate thread
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let ui = UI::new(false);
        let url = format!("http://localhost:{port}");

        let api_client = MockApiClient::new();

        let login_server = MockLoginServer {
            hits: Arc::new(0.into()),
        };

        // Test: Call the login function and check the result
        let auth_token = login(&api_client, &ui, &url, &login_server).await.unwrap();

        let got_token = Some(auth_token.token);

        // Token should be set now
        assert_eq!(
            got_token.as_deref(),
            Some(turborepo_vercel_api_mock::EXPECTED_TOKEN)
        );

        api_server.abort();
    }
}
