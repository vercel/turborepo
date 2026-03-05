use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    time::Duration,
};

use tracing::warn;
use turborepo_api_client::{CacheClient, Client, TokenClient};
use turborepo_ui::{BOLD, ColorConfig, start_spinner};
use url::Url;

use crate::{
    Error, LoginOptions, Token,
    device_flow::{self, TokenSet},
    error, ui,
};

const DEFAULT_SSO_PROVIDER: &str = "SAML/OIDC Single Sign-On";
const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const SSO_REDIRECT_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

fn is_vercel(login_url: &str) -> bool {
    login_url.contains("vercel.com")
}

fn make_token_name() -> Result<String, Error> {
    let host = hostname::get().map_err(Error::FailedToMakeSSOTokenName)?;

    Ok(format!(
        "Turbo CLI on {} via {DEFAULT_SSO_PROVIDER}",
        host.to_string_lossy()
    ))
}

/// Perform an SSO login flow.
///
/// For Vercel:
/// 1. Requires an existing device-flow login (needs refresh token).
/// 2. Introspects the current token to get `session_id` and `client_id`.
/// 3. Opens browser to SSO URL with session/client context and localhost
///    redirect.
/// 4. Receives verification token via localhost redirect.
/// 5. Verifies the SSO token with the API.
///
/// For non-Vercel (self-hosted):
/// 1. Opens browser to `{login_url}/api/auth/sso?teamId=...&next=localhost`.
/// 2. Receives token via localhost redirect.
/// 3. Verifies the SSO token with the API.
///
/// Returns `(Token, Option<TokenSet>)`. The `TokenSet` is always `None`
/// for SSO flows since the SSO verification returns a different token type.
pub async fn sso_login<T: Client + TokenClient + CacheClient>(
    options: &LoginOptions<'_, T>,
) -> Result<(Token, Option<TokenSet>), Error> {
    let LoginOptions {
        api_client,
        color_config,
        login_url: login_url_configuration,
        sso_team,
        existing_token,
        force,
        sso_login_callback_port,
    } = *options;

    let sso_team = sso_team.ok_or(Error::EmptySSOTeam)?;

    let valid_token_callback = |message: &str, color_config: &ColorConfig| {
        let message = message.to_string();
        let color_config = *color_config;
        move |user_email: &str| {
            println!("{}", color_config.apply(BOLD.apply_to(message)));
            ui::print_cli_authorized(user_email, &color_config);
        }
    };

    // Check if token exists first. Must be there for the user and contain the
    // sso_team passed into this function.
    if !force {
        if let Some(token) = existing_token {
            let token = Token::existing(token.into());
            if token
                .is_valid_sso(
                    api_client,
                    sso_team,
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
                        .is_valid_sso(
                            api_client,
                            sso_team,
                            Some(valid_token_callback(
                                &format!("Existing Vercel token for {sso_team} found!"),
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
                    warn!("Failed to load existing Vercel token for SSO, proceeding: {e}");
                }
            }
        }
    }

    let verification_token = if is_vercel(login_url_configuration) {
        sso_vercel(login_url_configuration, sso_team, existing_token).await?
    } else {
        let port = sso_login_callback_port.unwrap_or(DEFAULT_PORT);
        sso_redirect(login_url_configuration, sso_team, port).await?
    };

    // Verify the SSO token with the API
    let secret_verification_token =
        turborepo_api_client::SecretString::new(verification_token.clone());

    let token_name = make_token_name()?;

    let verified_user = api_client
        .verify_sso_token(&secret_verification_token, &token_name)
        .await
        .map_err(Error::FailedToValidateSSOToken)?;

    let user_response = api_client
        .get_user(&verified_user.token)
        .await
        .map_err(Error::FailedToFetchUser)?;

    ui::print_cli_authorized(&user_response.user.email, color_config);

    Ok((Token::New(verified_user.token), None))
}

/// Vercel SSO: introspect current token, open browser with session context.
async fn sso_vercel(
    login_url_configuration: &str,
    sso_team: &str,
    existing_token: Option<&str>,
) -> Result<String, Error> {
    // SSO on Vercel requires an existing device-flow login so we can
    // introspect the token for session_id and client_id.
    let current_token = match existing_token {
        Some(t) => t.to_string(),
        None => match crate::auth::get_token_with_refresh().await {
            Ok(Some(secret)) => secret.expose().to_string(),
            _ => return Err(Error::SSORequiresLogin),
        },
    };

    let http_client = reqwest::Client::new();
    let metadata = device_flow::discover(&http_client, login_url_configuration).await?;
    let introspection =
        device_flow::introspect_token(&http_client, &metadata, &current_token).await?;

    if !introspection.active {
        return Err(Error::IntrospectionFailed {
            message: "session is not active".to_string(),
        });
    }

    let session_id = introspection.session_id.ok_or(Error::IntrospectionFailed {
        message: "missing session_id".to_string(),
    })?;
    let client_id = introspection.client_id.ok_or(Error::IntrospectionFailed {
        message: "missing client_id".to_string(),
    })?;

    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| Error::DeviceAuthorizationFailed {
            message: format!("failed to bind localhost server for SSO redirect: {e}"),
        })?;
    let port = listener.local_addr().unwrap().port();

    let state = format!("{:x}{:x}", rand::random::<u64>(), rand::random::<u64>());

    let mut sso_url =
        Url::parse(login_url_configuration).map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?;
    sso_url
        .path_segments_mut()
        .map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["sso", sso_team]);

    sso_url
        .query_pairs_mut()
        .append_pair("session_id", &session_id)
        .append_pair("client_id", &client_id)
        .append_pair("state", &state)
        .append_pair("next", &format!("http://localhost:{port}"));

    println!(">>> Opening browser to {sso_url}");
    let spinner = start_spinner("Waiting for your authorization...");
    let url = sso_url.as_str();

    if !cfg!(test) && webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let expected_state = state.clone();
    let verification_token =
        tokio::task::spawn_blocking(move || wait_for_sso_redirect(listener, Some(&expected_state)))
            .await
            .map_err(|_| Error::FailedToGetToken)??;

    spinner.finish_and_clear();
    Ok(verification_token)
}

/// Non-Vercel SSO: open browser to `{login_url}/api/auth/sso` with
/// `teamId`, `mode`, and `next` params, then wait for localhost redirect.
/// This preserves the original SSO flow for self-hosted remote cache servers.
async fn sso_redirect(
    login_url_configuration: &str,
    sso_team: &str,
    port: u16,
) -> Result<String, Error> {
    let listener = TcpListener::bind(format!("{DEFAULT_HOST_NAME}:{port}"))
        .map_err(|_| Error::FailedToGetToken)?;
    let port = listener.local_addr().unwrap().port();
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{port}");

    let mut login_url =
        Url::parse(login_url_configuration).map_err(|_| error::Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?;

    login_url
        .path_segments_mut()
        .map_err(|_| error::Error::LoginUrlCannotBeABase {
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

    if !cfg!(test) && webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let verification_token =
        tokio::task::spawn_blocking(move || wait_for_sso_redirect(listener, None))
            .await
            .map_err(|_| Error::FailedToGetToken)??;

    spinner.finish_and_clear();
    Ok(verification_token)
}

/// Accept a single HTTP request on the listener, extract the `token` query
/// parameter, send a redirect response, and return the token.
///
/// If `expected_state` is provided (Vercel flow), validates the CSRF state
/// param. For non-Vercel flows, state validation is skipped since the remote
/// server may not support it.
fn wait_for_sso_redirect(
    listener: TcpListener,
    expected_state: Option<&str>,
) -> Result<String, Error> {
    listener
        .set_nonblocking(false)
        .map_err(|_| Error::FailedToGetToken)?;

    let (stream, _) = listener.accept().map_err(|_| Error::FailedToGetToken)?;
    stream
        .set_read_timeout(Some(SSO_REDIRECT_TIMEOUT))
        .map_err(|_| Error::FailedToGetToken)?;

    let mut reader =
        std::io::BufReader::new(stream.try_clone().map_err(|_| Error::FailedToGetToken)?);
    let mut request_line = String::new();
    reader
        .by_ref()
        .take(8192)
        .read_line(&mut request_line)
        .map_err(|_| Error::FailedToGetToken)?;

    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or(Error::FailedToGetToken)?;

    let url =
        Url::parse(&format!("http://localhost{path}")).map_err(|_| Error::FailedToGetToken)?;

    let params: std::collections::HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Validate CSRF state parameter if expected (Vercel flow only)
    if let Some(expected) = expected_state {
        match params.get("state") {
            Some(returned_state) if returned_state == expected => {}
            _ => {
                warn!("SSO redirect state parameter mismatch or missing");
                return Err(Error::FailedToGetToken);
            }
        }
    }

    // Determine redirect location (matching vc's getNotificationUrl behavior)
    let redirect_location = if params.contains_key("loginError") {
        let mut redirect_url =
            Url::parse("https://vercel.com/notifications/cli-login-failed").expect("valid URL");
        for (k, v) in &params {
            redirect_url.query_pairs_mut().append_pair(k, v);
        }
        redirect_url.to_string()
    } else if params.contains_key("ssoEmail") {
        let mut redirect_url =
            Url::parse("https://vercel.com/notifications/cli-login-incomplete").expect("valid URL");
        for (k, v) in &params {
            redirect_url.query_pairs_mut().append_pair(k, v);
        }
        redirect_url.to_string()
    } else {
        let mut redirect_url =
            Url::parse("https://vercel.com/notifications/cli-login-success").expect("valid URL");
        if let Some(email) = params.get("email") {
            redirect_url.query_pairs_mut().append_pair("email", email);
        }
        redirect_url.to_string()
    };

    let response =
        format!("HTTP/1.1 302 Found\r\nLocation: {redirect_location}\r\nConnection: close\r\n\r\n");
    let mut write_stream = stream;
    if let Err(e) = write_stream.write_all(response.as_bytes()) {
        warn!("Failed to send redirect to browser: {e}");
    }

    if params.contains_key("loginError") {
        return Err(Error::FailedToGetToken);
    }

    params.get("token").cloned().ok_or(Error::FailedToGetToken)
}

#[cfg(test)]
mod tests {
    use std::{assert_matches::assert_matches, io::Write, net::TcpStream};

    use reqwest::{Method, RequestBuilder, Response};
    use turborepo_vercel_api::{
        CachingStatus, CachingStatusResponse, Membership, Role, Team, TeamsResponse, User,
        UserResponse, VerifiedSsoUser,
        token::{ResponseTokenMetadata, Scope},
    };

    use super::*;
    use crate::current_unix_time;

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
            team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            Ok(Some(Team {
                id: team_id.to_string(),
                slug: team_id.to_string(),
                name: "Test Team".to_string(),
                created_at: 0,
                created: chrono::Utc::now(),
                membership: Membership::new(Role::Member),
            }))
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
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            Ok(ResponseTokenMetadata {
                id: "test".to_string(),
                name: "test".to_string(),
                token_type: "test".to_string(),
                scopes: vec![Scope {
                    scope_type: "team".to_string(),
                    team_id: Some("my-team".to_string()),
                    created_at: 0,
                    expires_at: None,
                }],
                active_at: current_unix_time() - 100,
                created_at: 0,
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
    async fn test_sso_login_missing_team() {
        let color_config = turborepo_ui::ColorConfig::new(false);
        let api_client = MockApiClient::new();

        let options = LoginOptions {
            color_config: &color_config,
            login_url: "https://api.vercel.com",
            api_client: &api_client,
            existing_token: None,
            sso_team: None,
            force: false,
            sso_login_callback_port: None,
        };

        let result = sso_login(&options).await;
        assert_matches!(result, Err(Error::EmptySSOTeam));
    }

    #[tokio::test]
    async fn test_sso_login_with_existing_token() {
        let color_config = turborepo_ui::ColorConfig::new(false);
        let api_client = MockApiClient::new();

        let options = LoginOptions {
            color_config: &color_config,
            login_url: "https://api.vercel.com",
            api_client: &api_client,
            existing_token: Some("existing-token"),
            sso_team: Some("my-team"),
            force: false,
            sso_login_callback_port: None,
        };

        let (result, token_set) = sso_login(&options).await.unwrap();
        assert_matches!(result, Token::Existing(ref token) if token.expose() == "existing-token");
        assert!(token_set.is_none());
    }

    #[test]
    fn test_make_token_name() {
        let result = make_token_name();
        assert!(result.is_ok());

        let token_name = result.unwrap();
        assert!(token_name.contains("Turbo CLI on"));
        assert!(token_name.contains("via SAML/OIDC Single Sign-On"));
    }

    #[test]
    fn test_wait_for_sso_redirect_happy_path_with_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = "test-state-123";

        let handle = std::thread::spawn({
            let state = state.to_string();
            move || {
                let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                let request = format!(
                    "GET /?token=my-sso-token&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).unwrap();
            }
        });

        let result = wait_for_sso_redirect(listener, Some(state));
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-sso-token");
    }

    #[test]
    fn test_wait_for_sso_redirect_happy_path_without_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request = "GET /?token=my-sso-token HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
        });

        let result = wait_for_sso_redirect(listener, None);
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-sso-token");
    }

    #[test]
    fn test_wait_for_sso_redirect_missing_token() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = "test-state";

        let handle = std::thread::spawn({
            let state = state.to_string();
            move || {
                let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                let request = format!("GET /?state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n");
                stream.write_all(request.as_bytes()).unwrap();
            }
        });

        let result = wait_for_sso_redirect(listener, Some(state));
        handle.join().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_wait_for_sso_redirect_state_mismatch() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request =
                "GET /?token=stolen&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
        });

        let result = wait_for_sso_redirect(listener, Some("correct-state"));
        handle.join().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_wait_for_sso_redirect_login_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = "s";

        let handle = std::thread::spawn({
            let state = state.to_string();
            move || {
                let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                let request = format!(
                    "GET /?loginError=failed&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).unwrap();
            }
        });

        let result = wait_for_sso_redirect(listener, Some(state));
        handle.join().unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_is_vercel() {
        assert!(is_vercel("https://vercel.com"));
        assert!(is_vercel("https://api.vercel.com"));
        assert!(!is_vercel("https://my-cache.example.com"));
    }
}
