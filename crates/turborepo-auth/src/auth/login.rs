use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    time::Duration,
};

pub use error::Error;
use tracing::warn;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::{BOLD, ColorConfig, start_spinner};
use url::Url;

use crate::{
    LoginOptions, Token,
    auth::is_vercel,
    device_flow::{self, TokenSet},
    error, ui,
};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const LOGIN_REDIRECT_TIMEOUT: Duration = Duration::from_secs(300);

/// Login returns a `(Token, Option<TokenSet>)`. If a token is already present,
/// we do not overwrite it and instead log that we found an existing token.
///
/// First checks if an existing option has been passed in, then if the login is
/// to Vercel, checks if the user has a Vercel CLI token on disk.
///
/// For Vercel logins, uses the OAuth 2.0 Device Authorization Grant (RFC 8628).
/// For non-Vercel logins (self-hosted remote caches), opens a browser to the
/// server's token page and waits for a localhost redirect with the token.
///
/// The `TokenSet` is `Some` when login completed via the device authorization
/// flow (Vercel only). It's `None` for non-Vercel logins or when an existing
/// token was reused.
pub async fn login<T: Client + TokenClient + CacheClient>(
    options: &LoginOptions<'_, T>,
) -> Result<(Token, Option<TokenSet>), Error> {
    let LoginOptions {
        api_client,
        color_config,
        login_url: login_url_configuration,
        existing_token,
        force,
        sso_team: _,
        sso_login_callback_port,
    } = *options;

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
            let token = Token::existing(token.into());
            if token
                .is_valid(
                    api_client,
                    Some(valid_token_callback("Existing token found!", color_config)),
                )
                .await?
            {
                return Ok((token, None));
            }
        } else if is_vercel(login_url_configuration) {
            match crate::auth::get_token_with_refresh().await {
                Ok(Some(token_secret)) => {
                    let token = Token::existing_secret(token_secret);
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
                        return Ok((token, None));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("Failed to load existing Vercel token, proceeding with new login: {e}");
                }
            }
        }
    }

    if is_vercel(login_url_configuration) {
        login_vercel_device_flow(api_client, color_config, login_url_configuration).await
    } else {
        let port = sso_login_callback_port.unwrap_or(DEFAULT_PORT);
        login_redirect(api_client, color_config, login_url_configuration, port).await
    }
}

/// Vercel login via OAuth 2.0 Device Authorization Grant (RFC 8628).
async fn login_vercel_device_flow<T: Client>(
    api_client: &T,
    color_config: &ColorConfig,
    login_url: &str,
) -> Result<(Token, Option<TokenSet>), Error> {
    let http_client = reqwest::Client::new();

    let metadata = device_flow::discover(&http_client, login_url).await?;
    let device_auth = device_flow::device_authorization_request(&http_client, &metadata).await?;

    let expires_at = crate::current_unix_time_secs() + device_auth.expires_in;

    let verification_url = device_auth
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device_auth.verification_uri);

    // RFC 8628 §3.3: the user code MUST be displayed so users can verify
    // it matches what the authorization server shows (anti-phishing).
    println!(
        "\n  Visit {} and confirm code {}",
        color_config.apply(BOLD.apply_to(&device_auth.verification_uri)),
        color_config.apply(BOLD.apply_to(&device_auth.user_code))
    );

    if !cfg!(test) {
        let _ = webbrowser::open(verification_url);
    }

    let spinner = start_spinner("Waiting for authentication...");

    let token_set = device_flow::poll_for_token(
        &http_client,
        &metadata,
        &device_auth.device_code,
        device_auth.interval,
        expires_at,
    )
    .await?;

    spinner.finish_and_clear();

    let secret_token = turborepo_api_client::SecretString::new(token_set.access_token.clone());

    let user_response = api_client
        .get_user(&secret_token)
        .await
        .map_err(Error::FailedToFetchUser)?;

    ui::print_cli_authorized(&user_response.user.email, color_config);

    Ok((Token::new(token_set.access_token.clone()), Some(token_set)))
}

/// Non-Vercel login via browser redirect to a localhost server.
/// Preserves the original login flow for self-hosted remote cache servers:
/// opens `{login_url}/turborepo/token?redirect_uri=http://127.0.0.1:{port}`,
/// waits for the server to redirect back with a `?token=` query parameter.
async fn login_redirect<T: Client>(
    api_client: &T,
    color_config: &ColorConfig,
    login_url_configuration: &str,
    port: u16,
) -> Result<(Token, Option<TokenSet>), Error> {
    let listener = TcpListener::bind(format!("{DEFAULT_HOST_NAME}:{port}"))
        .map_err(error::Error::CallbackListenerFailed)?;
    let port = listener.local_addr().unwrap().port();
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{port}");

    let mut login_url =
        Url::parse(login_url_configuration).map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?;

    let mut success_url = login_url.clone();
    success_url
        .path_segments_mut()
        .map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["turborepo", "success"]);

    login_url
        .path_segments_mut()
        .map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["turborepo", "token"]);

    login_url
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect_url);

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");

    let url = login_url.as_str();
    if !cfg!(test) && webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let success_redirect = success_url.to_string();
    let token_string =
        tokio::task::spawn_blocking(move || wait_for_login_redirect(listener, &success_redirect))
            .await
            .map_err(|_| Error::CallbackTaskFailed)??;

    spinner.finish_and_clear();

    let secret_token = turborepo_api_client::SecretString::new(token_string.clone());

    let user_response = api_client
        .get_user(&secret_token)
        .await
        .map_err(Error::FailedToFetchUser)?;

    ui::print_cli_authorized(&user_response.user.email, color_config);

    Ok((Token::new(token_string), None))
}

/// Accept HTTP requests on the listener until one carries a `token` query
/// parameter. Browsers may send preflight, favicon, or other auxiliary
/// requests before the real redirect arrives — looping prevents those from
/// consuming the single-shot listener.
fn wait_for_login_redirect(listener: TcpListener, success_redirect: &str) -> Result<String, Error> {
    listener
        .set_nonblocking(false)
        .map_err(Error::CallbackListenerFailed)?;

    let deadline = std::time::Instant::now() + LOGIN_REDIRECT_TIMEOUT;

    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or(Error::CallbackTimeout)?;

        listener
            .set_nonblocking(false)
            .map_err(Error::CallbackListenerFailed)?;

        let (stream, _) = listener.accept().map_err(Error::CallbackListenerFailed)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(Error::CallbackListenerFailed)?;

        let mut reader =
            std::io::BufReader::new(stream.try_clone().map_err(Error::CallbackListenerFailed)?);
        let mut request_line = String::new();
        if reader
            .by_ref()
            .take(8192)
            .read_line(&mut request_line)
            .is_err()
        {
            continue;
        }

        let path = match request_line.split_whitespace().nth(1) {
            Some(p) => p.to_string(),
            None => continue,
        };

        let url = match Url::parse(&format!("http://localhost{path}")) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let params: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        if let Some(token) = params.get("token").cloned() {
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {success_redirect}\r\nConnection: close\r\n\r\n"
            );
            let mut write_stream = stream;
            if let Err(e) = write_stream.write_all(response.as_bytes()) {
                warn!("Failed to send redirect to browser: {e}");
            }
            return Ok(token);
        }

        // Not the request we're looking for — send a minimal response and
        // keep listening.
        let mut write_stream = stream;
        let _ = write_stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, time::Duration};

    use reqwest::{Method, RequestBuilder, Response};
    use turborepo_vercel_api::{
        CachingStatus, CachingStatusResponse, Membership, Role, Team, TeamsResponse, User,
        UserResponse, VerifiedSsoUser,
    };
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;

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
        async fn get_user(
            &self,
            token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<UserResponse> {
            if token.expose().is_empty() {
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
        async fn get_teams(
            &self,
            token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<TeamsResponse> {
            if token.expose().is_empty() {
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
            _token: &turborepo_api_client::SecretString,
            _team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            unimplemented!("get_team")
        }
        fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
            unimplemented!("add_ci_header")
        }
        async fn verify_sso_token(
            &self,
            token: &turborepo_api_client::SecretString,
            _: &str,
        ) -> turborepo_api_client::Result<VerifiedSsoUser> {
            Ok(VerifiedSsoUser {
                token: token.clone(),
                team_id: Some("team_id".to_string()),
            })
        }
        async fn handle_403(_response: Response) -> turborepo_api_client::Error {
            unimplemented!("handle_403")
        }
        fn make_url(&self, endpoint: &str) -> turborepo_api_client::Result<url::Url> {
            let url = format!("{}{}", self.base_url, endpoint);
            url::Url::parse(&url)
                .map_err(|err| turborepo_api_client::Error::InvalidUrl { url, err })
        }
    }

    impl TokenClient for MockApiClient {
        async fn get_metadata(
            &self,
            token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<turborepo_vercel_api::token::ResponseTokenMetadata>
        {
            if token.expose().is_empty() {
                return Err(MockApiError::EmptyToken.into());
            }

            Ok(turborepo_vercel_api::token::ResponseTokenMetadata {
                id: "id".to_string(),
                name: "name".to_string(),
                token_type: "token".to_string(),
                scopes: vec![turborepo_vercel_api::token::Scope {
                    scope_type: "user".to_string(),
                    team_id: None,
                    expires_at: None,
                    created_at: 1111111111111,
                }],
                active_at: 0,
                created_at: 123456,
            })
        }
        async fn delete_token(
            &self,
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<()> {
            Ok(())
        }
    }

    impl CacheClient for MockApiClient {
        async fn get_artifact(
            &self,
            _hash: &str,
            _token: &turborepo_api_client::SecretString,
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
            _body_len: usize,
            _duration: u64,
            _tag: Option<&str>,
            _token: &turborepo_api_client::SecretString,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<(), turborepo_api_client::Error> {
            unimplemented!("set_artifact")
        }
        async fn fetch_artifact(
            &self,
            _hash: &str,
            _token: &turborepo_api_client::SecretString,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!("fetch_artifact")
        }
        async fn artifact_exists(
            &self,
            _hash: &str,
            _token: &turborepo_api_client::SecretString,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<Option<Response>, turborepo_api_client::Error> {
            unimplemented!("artifact_exists")
        }
        async fn get_caching_status(
            &self,
            _token: &turborepo_api_client::SecretString,
            _team_id: Option<&str>,
            _team_slug: Option<&str>,
        ) -> Result<CachingStatusResponse, turborepo_api_client::Error> {
            Ok(CachingStatusResponse {
                status: CachingStatus::Enabled,
            })
        }
    }

    #[tokio::test]
    async fn test_login_existing_token() {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let api_server = tokio::spawn(start_test_server(port, Some(ready_tx)));

        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("Test server failed to start")
            .expect("Server setup failed");

        let color_config = turborepo_ui::ColorConfig::new(false);
        let url = format!("http://localhost:{port}");

        let api_client = MockApiClient::new();

        let mut options = LoginOptions::new(&color_config, &url, &api_client);
        options.existing_token = Some(turborepo_vercel_api_mock::EXPECTED_TOKEN);

        let (token, token_set) = login(&options).await.unwrap();
        assert_matches!(token, Token::Existing(..));
        assert!(token_set.is_none());

        api_server.abort();
    }

    #[test]
    fn test_wait_for_login_redirect_happy_path() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request = "GET /?token=my-login-token HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
        });

        let result = wait_for_login_redirect(listener, "https://example.com/turborepo/success");
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-login-token");
    }

    #[test]
    fn test_wait_for_login_redirect_skips_non_token_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            // First: a request without a token (e.g. favicon). Listener should
            // respond with 204 and keep waiting.
            let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request = "GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
            let mut buf = [0u8; 256];
            let _ = std::io::Read::read(&mut stream, &mut buf);

            // Second: the real callback with a token.
            let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request = "GET /?token=my-login-token HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
        });

        let result = wait_for_login_redirect(listener, "https://example.com/turborepo/success");
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-login-token");
    }
}
