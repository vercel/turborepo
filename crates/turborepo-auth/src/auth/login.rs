use std::sync::Arc;

pub use error::Error;
use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::warn;
use turborepo_api_client::Client;
use turborepo_ui::{start_spinner, BOLD};

use crate::{
    auth::{check_token, extract_vercel_token},
    error, ui, LoginOptions,
};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

/// Token is the result of a successful login. It contains the token string and
/// a boolean indicating whether the token already existed on the filesystem.
#[derive(Debug)]
pub struct Token {
    /// The actual token string.
    pub token: String,
    /// If this is `true`, it means this token already exists on the filesystem.
    /// If `false`, this is a new token.
    pub exists: bool,
}

/// Login returns a `Token` struct. If a token is already present,
/// we do not overwrite it and instead log that we found an existing token,
/// setting the `exists` field to `true`.
///
/// First checks if an existing option has been passed in, then if the login is
/// to Vercel, checks if the user has a Vercel CLI token on disk.
pub async fn login<T: Client>(options: &LoginOptions<'_, T>) -> Result<Token, Error> {
    let (api_client, ui, login_url_configuration, login_server) = (
        options.api_client,
        options.ui,
        options.login_url,
        options.login_server,
    );
    // Check if passed in token exists first.
    if let Some(token) = options.existing_token {
        return check_token(token, ui, api_client, "Existing token found!").await;
    }

    // If the user is logging into Vercel, check for an existing `vc` token.
    if login_url_configuration.contains("vercel.com") {
        match extract_vercel_token() {
            Ok(token) => {
                println!(
                    "{}",
                    ui.apply(BOLD.apply_to("Existing Vercel token found!"))
                );
                return Ok(Token {
                    token,
                    exists: true,
                });
            }
            Err(error) => {
                // Only send the warning if we're debugging.
                dbg!("Failed to extract Vercel token: ", error);
            }
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

    let token = token_cell.get().ok_or(Error::FailedToGetToken)?;

    // TODO: make this a request to /teams endpoint instead?
    let user_response = api_client
        .get_user(token.as_str())
        .await
        .map_err(Error::FailedToFetchUser)?;

    ui::print_cli_authorized(&user_response.user.email, ui);

    Ok(Token {
        token: token.into(),
        exists: false,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;
    use reqwest::{Method, RequestBuilder, Response};
    use turborepo_api_client::Client;
    use turborepo_ui::UI;
    use turborepo_vercel_api::{
        CachingStatusResponse, Membership, Role, SpacesResponse, Team, TeamsResponse, User,
        UserResponse, VerifiedSsoUser,
    };
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::LoginServer;

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
        ) -> Result<(), Error> {
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
        async fn get_user(&self, token: &str) -> turborepo_api_client::Result<UserResponse> {
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
        async fn get_teams(&self, token: &str) -> turborepo_api_client::Result<TeamsResponse> {
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
        async fn get_team(
            &self,
            _token: &str,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn get_caching_status(
            &self,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> turborepo_api_client::Result<CachingStatusResponse> {
            unimplemented!("get_caching_status")
        }
        async fn get_spaces(
            &self,
            _token: &str,
            _team_id: Option<&str>,
        ) -> turborepo_api_client::Result<SpacesResponse> {
            unimplemented!("get_spaces")
        }
        async fn verify_sso_token(
            &self,
            token: &str,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
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
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> turborepo_api_client::Result<()> {
            unimplemented!("put_artifact")
        }
        async fn handle_403(_response: Response) -> turborepo_api_client::Error {
            unimplemented!("handle_403")
        }
        async fn fetch_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> turborepo_api_client::Result<Option<Response>> {
            unimplemented!("fetch_artifact")
        }
        async fn artifact_exists(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> turborepo_api_client::Result<Option<Response>> {
            unimplemented!("artifact_exists")
        }
        async fn get_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
            _method: Method,
        ) -> turborepo_api_client::Result<Option<Response>> {
            unimplemented!("get_artifact")
        }
        fn make_url(&self, endpoint: &str) -> turborepo_api_client::Result<Url> {
            let url = format!("{}{}", self.base_url, endpoint);
            Url::parse(&url).map_err(|err| turborepo_api_client::Error::InvalidUrl { url, err })
        }
    }

    #[tokio::test]
    async fn test_login() {
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let ui = UI::new(false);
        let url = format!("http://localhost:{port}");

        let api_client = MockApiClient::new();

        let login_server = MockLoginServer {
            hits: Arc::new(0.into()),
        };
        let mut options = LoginOptions::new(&ui, &url, &api_client, &login_server);

        let token = login(&options).await.unwrap();
        assert!(!token.exists);

        let got_token = token.token.to_string();
        assert_eq!(&got_token, turborepo_vercel_api_mock::EXPECTED_TOKEN);

        // Call the login function a second time to test that we check for existing
        // tokens. Total server hits should be 1.
        options.existing_token = Some(&got_token);
        let second_token = login(&options).await.unwrap();
        assert!(second_token.exists);

        api_server.abort();
        assert_eq!(
            login_server.hits.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }
}
