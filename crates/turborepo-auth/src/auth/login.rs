use std::sync::Arc;

pub use error::Error;
use reqwest::Url;
use tokio::sync::OnceCell;
use tracing::{debug, warn};
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::{start_spinner, ColorConfig, BOLD};

use crate::{auth::extract_vercel_token, error, ui, LoginOptions, Token};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

/// Login returns a `Token` struct. If a token is already present,
/// we do not overwrite it and instead log that we found an existing token,
/// setting the `exists` field to `true`.
///
/// First checks if an existing option has been passed in, then if the login is
/// to Vercel, checks if the user has a Vercel CLI token on disk.
pub async fn login<T: Client + TokenClient + CacheClient>(
    options: &LoginOptions<'_, T>,
) -> Result<Token, Error> {
    let LoginOptions {
        api_client,
        color_config,
        login_url: login_url_configuration,
        login_server,
        existing_token,
        force,
        sso_team: _,
    } = *options; // Deref or we get double references for each of these

    // I created a closure that gives back a closure since the `is_valid` checks do
    // a call to get the user, so instead of doing that multiple times we have
    // `is_valid` give back the user email.
    //
    // In the future I want to make the Token have some non-skewable information and
    // be able to get rid of this, but it works for now.
    let valid_token_callback = |message: &str, color_config: &ColorConfig| {
        let message = message.to_string();
        let color_config = *color_config;
        move |user_email: &str| {
            println!("{}", color_config.apply(BOLD.apply_to(message)));
            ui::print_cli_authorized(user_email, &color_config);
        }
    };

    // Check if passed in token exists first.
    if !force {
        if let Some(token) = existing_token {
            debug!("found existing turbo token");
            let token = Token::existing(token.into());
            if token
                .is_valid(
                    api_client,
                    Some(valid_token_callback("Existing token found!", color_config)),
                )
                .await?
            {
                return Ok(token);
            }
        // If the user is logging into Vercel, check for an existing `vc` token.
        } else if login_url_configuration.contains("vercel.com") {
            // The extraction can return an error, but we don't want to fail the login if
            // the token is not found.
            if let Ok(Some(token)) = extract_vercel_token() {
                debug!("found existing Vercel token");
                let token = Token::existing(token);
                if token
                    .is_valid(
                        api_client,
                        Some(valid_token_callback(
                            "Existing Vercel token found!",
                            color_config,
                        )),
                    )
                    .await?
                {
                    return Ok(token);
                }
            }
        }
    }

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let mut login_url = Url::parse(login_url_configuration)?;
    let mut success_url = login_url.clone();
    success_url
        .path_segments_mut()
        .map_err(|_: ()| Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["turborepo", "success"]);

    // Create the full login URL.
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
            crate::LoginType::Basic {
                success_redirect: success_url.to_string(),
            },
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

    ui::print_cli_authorized(&user_response.user.email, color_config);

    Ok(Token::new(token.into()))
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, sync::atomic::AtomicUsize};

    use async_trait::async_trait;
    use reqwest::{Method, RequestBuilder, Response};
    use turborepo_vercel_api::{
        CachingStatus, CachingStatusResponse, Membership, Role, SpacesResponse, Team,
        TeamsResponse, User, UserResponse, VerifiedSsoUser,
    };
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::{login_server, LoginServer};

    struct MockLoginServer {
        hits: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LoginServer for MockLoginServer {
        async fn run(
            &self,
            _: u16,
            _: login_server::LoginType,
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
        async fn handle_403(_response: Response) -> turborepo_api_client::Error {
            unimplemented!("handle_403")
        }
        fn make_url(&self, endpoint: &str) -> turborepo_api_client::Result<Url> {
            let url = format!("{}{}", self.base_url, endpoint);
            Url::parse(&url).map_err(|err| turborepo_api_client::Error::InvalidUrl { url, err })
        }
    }

    impl TokenClient for MockApiClient {
        async fn get_metadata(
            &self,
            token: &str,
        ) -> turborepo_api_client::Result<turborepo_vercel_api::token::ResponseTokenMetadata>
        {
            if token.is_empty() {
                return Err(MockApiError::EmptyToken.into());
            }

            Ok(turborepo_vercel_api::token::ResponseTokenMetadata {
                id: "id".to_string(),
                name: "name".to_string(),
                token_type: "token".to_string(),
                origin: "github".to_string(),
                scopes: vec![turborepo_vercel_api::token::Scope {
                    scope_type: "user".to_string(),
                    origin: "github".to_string(),
                    team_id: None,
                    expires_at: None,
                    created_at: 1111111111111,
                }],
                active_at: 0,
                created_at: 123456,
            })
        }
        async fn delete_token(&self, _token: &str) -> turborepo_api_client::Result<()> {
            Ok(())
        }
    }

    impl CacheClient for MockApiClient {
        async fn get_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
            _method: Method,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!("get_artifact")
        }
        async fn put_artifact(
            &self,
            _hash: &str,
            _artifact_body: impl turborepo_api_client::Stream<
                    Item = Result<turborepo_api_client::Bytes, turborepo_api_client::Error>,
                > + Send
                + Sync
                + 'static,
            _duration: u64,
            _tag: Option<&str>,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<(), turborepo_api_client::Error> {
            unimplemented!("set_artifact")
        }
        async fn fetch_artifact(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!("fetch_artifact")
        }
        async fn artifact_exists(
            &self,
            _hash: &str,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!("artifact_exists")
        }
        async fn get_caching_status(
            &self,
            _token: &str,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<CachingStatusResponse, turborepo_api_client::Error> {
            Ok(CachingStatusResponse {
                status: CachingStatus::Enabled,
            })
        }
    }

    #[tokio::test]
    async fn test_login() {
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let color_config = ColorConfig::new(false);
        let url = format!("http://localhost:{port}");

        let api_client = MockApiClient::new();

        let login_server = MockLoginServer {
            hits: Arc::new(0.into()),
        };
        let mut options = LoginOptions::new(&color_config, &url, &api_client, &login_server);

        let token = login(&options).await.unwrap();
        assert_matches!(token, Token::New(..));

        let got_token = token.into_inner().to_string();
        assert_eq!(&got_token, turborepo_vercel_api_mock::EXPECTED_TOKEN);

        // Call the login function a second time to test that we check for existing
        // tokens. Total server hits should be 1.
        options.existing_token = Some(&got_token);
        let second_token = login(&options).await.unwrap();
        assert!(matches!(second_token, Token::Existing(..)));

        api_server.abort();
        assert_eq!(
            login_server.hits.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }
}
