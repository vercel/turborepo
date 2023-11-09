use std::sync::Arc;

use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::warn;
use turborepo_api_client::Client;
use turborepo_ui::{start_spinner, UI};

use crate::{convert_to_auth_token, error, ui, AuthToken, Error, SSOLoginServer};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const DEFAULT_SSO_PROVIDER: &str = "SAML/OIDC Single Sign-On";

fn make_token_name() -> Result<String, Error> {
    let host = hostname::get().map_err(Error::FailedToMakeSSOTokenName)?;

    Ok(format!(
        "Turbo CLI on {} via {DEFAULT_SSO_PROVIDER}",
        host.to_string_lossy()
    ))
}

/// present, and the token has access to the provided `sso_team`, we do not
/// overwrite it and instead log that we found an existing token.
pub async fn sso_login<'a>(
    api_client: &impl Client,
    ui: &UI,
    login_url_configuration: &str,
    sso_team: &str,
    login_server: &impl SSOLoginServer,
) -> Result<AuthToken, Error> {
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let mut login_url = Url::parse(login_url_configuration)?;

    login_url
        .path_segments_mut()
        .map_err(|_: ()| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["api", "auth", "sso"]);

    login_url
        .query_pairs_mut()
        .append_pair("teamId", sso_team)
        .append_pair("mode", "login")
        .append_pair("next", &redirect_url);

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");

    // Try to open browser for auth confirmation.
    let url = login_url.as_str();
    if login_server.open_web_browser(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let token_cell = Arc::new(OnceCell::new());
    login_server.run(DEFAULT_PORT, token_cell.clone()).await?;
    spinner.finish_and_clear();

    let token = token_cell.get().ok_or(Error::FailedToGetToken)?;

    let token_name = make_token_name()?;

    let verified_user = api_client
        .verify_sso_token(token, &token_name)
        .await
        .map_err(Error::FailedToValidateSSOToken)?;

    let user_response = api_client
        .get_user(&verified_user.token)
        .await
        .map_err(Error::FailedToFetchUser)?;

    let auth_token = convert_to_auth_token(&verified_user.token, api_client).await?;

    ui::print_cli_authorized(&user_response.user.email, ui);

    Ok(auth_token)
}

#[cfg(test)]
mod tests {
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::mocks::*;

    #[tokio::test]
    async fn test_sso_login() {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let url = format!("http://localhost:{port}");
        let ui = UI::new(false);
        let team = "something";

        let api_client = MockApiClient::new();

        let login_server = MockSSOLoginServer {
            hits: Arc::new(0.into()),
        };

        let token = sso_login(&api_client, &ui, &url, team, &login_server)
            .await
            .unwrap();

        assert_eq!(token.token, EXPECTED_VERIFICATION_TOKEN.to_owned());

        handle.abort();
    }
}
