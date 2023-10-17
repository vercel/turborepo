use std::sync::Arc;

use anyhow::{anyhow, Result};
pub use error::Error;
use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::warn;
use turborepo_api_client::Client;
use turborepo_ui::{start_spinner, BOLD, UI};

use crate::{error, server::LoginServer, ui};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

/// Login writes a token to disk at token_path. If a token is already present,
/// we do not overwrite it and instead log that we found an existing token.
pub async fn login<F>(
    api_client: &impl Client,
    ui: &UI,
    existing_token: Option<&str>,
    mut set_token: F,
    login_url_configuration: &str,
    login_server: &impl LoginServer,
) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    // Check if token exists first.
    if let Some(token) = existing_token {
        if let Ok(response) = api_client.get_user(token).await {
            println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
            ui::print_cli_authorized(&response.user.email, ui);
            return Ok(());
        }
    }

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let mut login_url = Url::parse(login_url_configuration)?;

    login_url
        .path_segments_mut()
        .map_err(|_: ()| Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["turborepo", "token"]);

    login_url
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect_url);

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");

    // Try to open browser for auth confirmation.
    let url = login_url.as_str();
    if !cfg!(test) && webbrowser::open(url).is_err() {
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

    let token = token_cell
        .get()
        .ok_or_else(|| anyhow!("Failed to get token"))?;

    // This function is passed in from turborepo-lib
    // TODO: inline this here and only pass in the location to write the token as an
    // optional arg.
    set_token(token)?;

    // TODO: make this a request to /teams endpoint instead?
    let user_response = api_client.get_user(token.as_str()).await?;

    ui::print_cli_authorized(&user_response.user.email, ui);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;
    use reqwest::{Method, RequestBuilder, Response};
    use turborepo_api_client::{Client, Error, Result};
    use turborepo_vercel_api::{
        CachingStatusResponse, Membership, PreflightResponse, Role, SpacesResponse, Team,
        TeamsResponse, User, UserResponse, VerifiedSsoUser,
    };
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;

    struct MockLoginServer {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LoginServer for MockLoginServer {
        async fn run(
            &self,
            _: u16,
            _: String,
            login_token: Arc<OnceCell<String>>,
        ) -> anyhow::Result<()> {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            login_token
                .set(turborepo_vercel_api_mock::EXPECTED_TOKEN.to_string())
                .unwrap();
            Ok(())
        }
    }

    #[derive(Debug, thiserror::Error)]
    enum MockApiError {
        #[error("Empty token")]
        EmptyToken,
    }

    impl From<MockApiError> for turborepo_api_client::Error {
        fn from(error: MockApiError) -> Self {
            match error {
                MockApiError::EmptyToken => turborepo_api_client::Error::UnknownStatus {
                    code: "empty token".to_string(),
                    message: "token is empty".to_string(),
                    backtrace: std::backtrace::Backtrace::capture(),
                },
            }
        }
    }

    struct MockApiClient {
        pub base_url: String,
    }

    impl MockApiClient {
        fn new() -> Self {
            Self {
                base_url: String::new(),
            }
        }
    }

    #[async_trait]
    impl Client for MockApiClient {
        async fn get_user(&self, token: &str) -> Result<UserResponse> {
            if token.is_empty() {
                return Err(MockApiError::EmptyToken.into());
            }

            Ok(UserResponse {
                user: User {
                    id: "id".to_string(),
                    username: "username".to_string(),
                    email: "email".to_string(),
                    name: None,
                    created_at: None,
                },
            })
        }
        async fn get_teams(&self, token: &str) -> Result<TeamsResponse> {
            if token.is_empty() {
                return Err(MockApiError::EmptyToken.into());
            }

            Ok(TeamsResponse {
                teams: vec![Team {
                    id: "id".to_string(),
                    slug: "something".to_string(),
                    name: "name".to_string(),
                    created_at: 0,
                    created: chrono::Utc::now(),
                    membership: Membership::new(Role::Member),
                }],
            })
        }
        async fn get_team(&self, _token: &str, _team_id: &str) -> Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        fn add_team_params(
            _request_builder: RequestBuilder,
            _team_id: &str,
            _team_slug: Option<&str>,
        ) -> RequestBuilder {
            unimplemented!("add_team_params")
        }
        async fn get_caching_status(
            &self,
            _token: &str,
            _team_id: &str,
            _team_slug: Option<&str>,
        ) -> Result<CachingStatusResponse> {
            unimplemented!("get_caching_status")
        }
        async fn get_spaces(&self, _token: &str, _team_id: Option<&str>) -> Result<SpacesResponse> {
            unimplemented!("get_spaces")
        }
        async fn verify_sso_token(&self, token: &str, _: &str) -> Result<VerifiedSsoUser> {
            Ok(VerifiedSsoUser {
                token: token.to_string(),
                team_id: Some("team_id".to_string()),
            })
        }
        async fn put_artifact(
            &self,
            _hash: &str,
            _artifact_body: &[u8],
            _duration: u64,
            _tag: Option<&str>,
            _token: &str,
        ) -> Result<()> {
            unimplemented!("put_artifact")
        }
        async fn handle_403(_response: Response) -> Error {
            unimplemented!("handle_403")
        }
        async fn fetch_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: &str,
            _team_slug: Option<&str>,
        ) -> Result<Response> {
            unimplemented!("fetch_artifact")
        }
        async fn artifact_exists(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: &str,
            _team_slug: Option<&str>,
        ) -> Result<Response> {
            unimplemented!("artifact_exists")
        }
        async fn get_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: &str,
            _team_slug: Option<&str>,
            _method: Method,
        ) -> Result<Response> {
            unimplemented!("get_artifact")
        }
        async fn do_preflight(
            &self,
            _token: &str,
            _request_url: &str,
            _request_method: &str,
            _request_headers: &str,
        ) -> Result<PreflightResponse> {
            unimplemented!("do_preflight")
        }
        fn make_url(&self, endpoint: &str) -> String {
            format!("{}{}", self.base_url, endpoint)
        }
    }

    #[tokio::test]
    async fn test_login() {
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let ui = UI::new(false);
        let url = format!("http://localhost:{port}");

        let api_client = MockApiClient::new();

        // Because of the borrow checker, we wrap the option in a ref cell since we have
        // multiple types of borrows happening in this test.
        let mut got_token: Option<String> = None;

        // set_token is called by login. Ignore the string given since that escapes
        let set_token = |t: &str| -> anyhow::Result<(), anyhow::Error> {
            // Make the fetched token the expectation
            got_token = Some(t.to_owned());
            Ok(())
        };

        let login_server = MockLoginServer {
            hits: Arc::new(0.into()),
        };

        login(&api_client, &ui, None, set_token, &url, &login_server)
            .await
            .unwrap();

        // Token should be set now
        assert_eq!(
            got_token.as_deref(),
            Some(turborepo_vercel_api_mock::EXPECTED_TOKEN)
        );

        // Re-assign set_token due to ownership rules. This shouldn't be called.
        let mut second_token: Option<&str> = None;
        let set_token = |_: &str| -> anyhow::Result<(), anyhow::Error> {
            // Set it to literally anything but the expected token.
            second_token = Some("not expected");
            Ok(())
        };

        // Call the login function a second time to test that we check for existing
        // tokens. Total server hits should be 1.
        login(
            &api_client,
            &ui,
            got_token.as_deref(),
            set_token,
            &url,
            &login_server,
        )
        .await
        .unwrap();

        api_server.abort();
        assert_eq!(
            login_server.hits.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(second_token, None);
    }
}
