use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::warn;
use turborepo_api_client::Client;
use turborepo_ui::{start_spinner, BOLD, UI};

use crate::{error, server, ui};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const DEFAULT_SSO_PROVIDER: &str = "SAML/OIDC Single Sign-On";

fn make_token_name() -> Result<String> {
    let host = hostname::get()?;

    Ok(format!(
        "Turbo CLI on {} via {DEFAULT_SSO_PROVIDER}",
        host.to_string_lossy()
    ))
}

/// present, and the token has access to the provided `sso_team`, we do not
/// overwrite it and instead log that we found an existing token.
pub async fn sso_login<F>(
    api_client: &impl Client,
    ui: &UI,
    existing_token: Option<&str>,
    mut set_token: F,
    login_url_configuration: &str,
    sso_team: &str,
    login_server: &impl server::SSOLoginServer,
) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    // Check if token exists first. Must be there for the user and contain the
    // sso_team passed into this function.
    if let Some(token) = existing_token {
        let (result_user, result_teams) =
            tokio::join!(api_client.get_user(token), api_client.get_teams(token));

        if let (Ok(response_user), Ok(response_teams)) = (result_user, result_teams) {
            if response_teams
                .teams
                .iter()
                .any(|team| team.slug == sso_team)
            {
                println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
                ui::print_cli_authorized(&response_user.user.email, ui);
                return Ok(());
            }
        }
    }

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
    let url = login_url.as_str();

    // Don't open the browser in tests.
    if !cfg!(test) && webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let token_cell = Arc::new(OnceCell::new());
    login_server.run(DEFAULT_PORT, token_cell.clone()).await?;
    spinner.finish_and_clear();

    let token = token_cell
        .get()
        .ok_or_else(|| anyhow!("no token auth token found"))?;

    let token_name = make_token_name().context("failed to make sso token name")?;

    let verified_user = api_client.verify_sso_token(token, &token_name).await?;
    let user_response = api_client.get_user(&verified_user.token).await?;

    set_token(&verified_user.token)?;

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
    use crate::SSOLoginServer;
    const EXPECTED_VERIFICATION_TOKEN: &str = "expected_verification_token";

    lazy_static::lazy_static! {
        static ref SSO_HITS: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
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

        fn set_base_url(&mut self, base_url: &str) {
            self.base_url = base_url.to_string();
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

    struct MockSSOLoginServer {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SSOLoginServer for MockSSOLoginServer {
        async fn run(
            &self,
            _port: u16,
            verification_token: Arc<OnceCell<String>>,
        ) -> anyhow::Result<()> {
            self.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            verification_token
                .set(EXPECTED_VERIFICATION_TOKEN.to_string())
                .unwrap();
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_sso_login() {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let url = format!("http://localhost:{port}");
        let ui = UI::new(false);
        let team = "something";

        let mut api_client = MockApiClient::new();
        api_client.set_base_url(&url);

        let mut got_token: Option<String> = None;

        // closure that will check that the token is sent correctly
        let set_token = |t: &str| -> anyhow::Result<(), anyhow::Error> {
            // Force the got token to be incorrect if this is called a second time.
            got_token = Some(t.to_owned());
            Ok(())
        };

        let login_server = MockSSOLoginServer {
            hits: Arc::new(0.into()),
        };

        sso_login(&api_client, &ui, None, set_token, &url, team, &login_server)
            .await
            .unwrap();

        assert_eq!(got_token, Some(EXPECTED_VERIFICATION_TOKEN.to_owned()));

        // Re-assign set_token due to ownership rules. This shouldn't be called.
        let mut second_token: Option<&str> = None;
        let set_token = |_: &str| -> anyhow::Result<(), anyhow::Error> {
            second_token = Some("not expected");
            Ok(())
        };

        // Call the login function twice to test that we check for existing tokens.
        // Total server hits should be 1.
        sso_login(
            &api_client,
            &ui,
            got_token.as_deref(),
            set_token,
            &url,
            team,
            &login_server,
        )
        .await
        .unwrap();

        handle.abort();

        // This makes sure we never make it to the login server.
        assert_eq!(
            login_server.hits.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        // If our set_token was called a second time, it'll set second_token as Some.
        assert_eq!(second_token, None);
    }
}
