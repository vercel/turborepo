use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    time::Duration,
};

use tracing::warn;
use turborepo_api_client::{Client, TokenClient};
use turborepo_ui::{ColorConfig, start_spinner};
use url::Url;

use super::login::{
    is_auth_rejection_error, is_inactive_token_error, login_vercel_device_flow,
    valid_token_callback,
};
use crate::{
    Error, LoginOptions, Token,
    auth::{
        ExistingTokenSource, classify_existing_vercel_token, ensure_trusted_vercel_api,
        generate_csrf_state, is_vercel, should_attempt_vercel_token_refresh,
        should_skip_existing_token_for_login,
    },
    device_flow::TokenSet,
    error, ui,
};

const DEFAULT_SSO_PROVIDER: &str = "SAML/OIDC Single Sign-On";
const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const SSO_REDIRECT_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

enum ExistingSSOTokenAction {
    Reuse(Token),
    Discard,
}

fn make_token_name() -> Result<String, Error> {
    let host = hostname::get().map_err(Error::FailedToMakeSSOTokenName)?;

    Ok(format!(
        "Turbo CLI on {} via {DEFAULT_SSO_PROVIDER}",
        host.to_string_lossy()
    ))
}

async fn classify_existing_sso_token<T: Client + TokenClient>(
    token: Token,
    api_client: &T,
    sso_team: &str,
    valid_message_fn: Option<impl FnOnce(&str)>,
) -> Result<ExistingSSOTokenAction, Error> {
    match token
        .is_valid_sso(api_client, sso_team, valid_message_fn)
        .await
    {
        Ok(true) => Ok(ExistingSSOTokenAction::Reuse(token)),
        Ok(false) => Ok(ExistingSSOTokenAction::Discard),
        Err(err) if is_inactive_token_error(&err) => {
            warn!("Stored token is no longer active, proceeding with a fresh SSO login");
            Ok(ExistingSSOTokenAction::Discard)
        }
        Err(Error::SSOTeamNotFound(_)) => {
            warn!(
                "Stored token does not have access to {sso_team}, proceeding with a fresh SSO \
                 login"
            );
            Ok(ExistingSSOTokenAction::Discard)
        }
        Err(err) => Err(err),
    }
}

async fn classify_existing_sso_token_with_recovery<T: Client + TokenClient>(
    token: Token,
    api_client: &T,
    sso_team: &str,
    valid_message: &str,
    color_config: &ColorConfig,
) -> Result<ExistingSSOTokenAction, Error> {
    match classify_existing_sso_token(
        token.clone(),
        api_client,
        sso_team,
        Some(valid_token_callback(valid_message, color_config)),
    )
    .await
    {
        Ok(ExistingSSOTokenAction::Reuse(token)) => Ok(ExistingSSOTokenAction::Reuse(token)),
        Ok(ExistingSSOTokenAction::Discard) => {
            let Some(recovered_token) =
                crate::auth::recover_token_after_forbidden(token.into_inner()).await?
            else {
                return Ok(ExistingSSOTokenAction::Discard);
            };

            match classify_existing_sso_token(
                Token::existing_secret(recovered_token),
                api_client,
                sso_team,
                Some(valid_token_callback(valid_message, color_config)),
            )
            .await
            {
                Ok(result) => Ok(result),
                Err(err) if is_auth_rejection_error(&err) => Ok(ExistingSSOTokenAction::Discard),
                Err(err) => Err(err),
            }
        }
        Err(err) if is_auth_rejection_error(&err) => {
            let Some(recovered_token) =
                crate::auth::recover_token_after_forbidden(token.into_inner()).await?
            else {
                return Ok(ExistingSSOTokenAction::Discard);
            };

            match classify_existing_sso_token(
                Token::existing_secret(recovered_token),
                api_client,
                sso_team,
                Some(valid_token_callback(valid_message, color_config)),
            )
            .await
            {
                Ok(result) => Ok(result),
                Err(err) if is_auth_rejection_error(&err) => Ok(ExistingSSOTokenAction::Discard),
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

/// Perform an SSO login flow.
///
/// For Vercel:
/// 1. Reuses an existing token when it already has access to the requested
///    team.
/// 2. Otherwise runs the OAuth device flow.
/// 3. Vercel's device authorization page handles any additional SSO challenge.
/// 4. Validates that the resulting token has access to the requested team.
///
/// For non-Vercel (self-hosted):
/// 1. Opens browser to
///    `{login_url}/api/auth/sso?teamId=...&next=localhost&state=...`.
/// 2. Receives token via localhost redirect.
/// 3. Verifies the SSO token with the API.
///
/// Returns `(Token, Option<TokenSet>)`. The `TokenSet` is `Some` when Vercel
/// SSO completes via device flow and `None` when an existing token is reused or
/// when a self-hosted SSO flow returns a verified token directly.
pub async fn sso_login<T: Client + TokenClient>(
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
    let is_vercel_login = is_vercel(login_url_configuration);
    if is_vercel_login {
        ensure_trusted_vercel_api(api_client)?;
    }

    // Check if token exists first. Must be there for the user and contain the
    // sso_team passed into this function.
    // For Vercel logins, --force is silently ignored.
    if !force || is_vercel_login {
        if let Some(token) = existing_token {
            let token_source = classify_existing_vercel_token(token)?;
            let skip_existing_token = should_skip_existing_token_for_login(
                token_source,
                is_vercel_login,
                login_url_configuration,
            );

            if is_vercel_login && should_attempt_vercel_token_refresh(token_source) {
                match crate::auth::get_token_with_refresh_for_login().await {
                    Ok(Some(token_secret)) => {
                        if classify_existing_vercel_token(token_secret.expose())?
                            != ExistingTokenSource::LegacyAuth
                        {
                            match classify_existing_sso_token_with_recovery(
                                Token::existing_secret(token_secret),
                                api_client,
                                sso_team,
                                &format!("Using existing Vercel login for team: {sso_team}"),
                                color_config,
                            )
                            .await?
                            {
                                ExistingSSOTokenAction::Reuse(token) => {
                                    return Ok((token, None));
                                }
                                ExistingSSOTokenAction::Discard => {}
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        warn!("Failed to load existing token for SSO, proceeding: {e}");
                    }
                }
            } else if !skip_existing_token && token_source != ExistingTokenSource::LegacyAuth {
                let token_action = if is_vercel_login {
                    classify_existing_sso_token_with_recovery(
                        Token::existing(token.to_string()),
                        api_client,
                        sso_team,
                        "Using existing Vercel login.",
                        color_config,
                    )
                    .await?
                } else {
                    classify_existing_sso_token(
                        Token::existing(token.to_string()),
                        api_client,
                        sso_team,
                        Some(valid_token_callback("Using existing login.", color_config)),
                    )
                    .await?
                };

                match token_action {
                    ExistingSSOTokenAction::Reuse(token) => return Ok((token, None)),
                    ExistingSSOTokenAction::Discard => {}
                }
            }
        } else if is_vercel_login {
            match crate::auth::get_token_with_refresh_for_login().await {
                Ok(Some(token_secret)) => {
                    if classify_existing_vercel_token(token_secret.expose())?
                        != ExistingTokenSource::LegacyAuth
                    {
                        match classify_existing_sso_token_with_recovery(
                            Token::existing_secret(token_secret),
                            api_client,
                            sso_team,
                            &format!("Using existing Vercel login for team: {sso_team}"),
                            color_config,
                        )
                        .await?
                        {
                            ExistingSSOTokenAction::Reuse(token) => return Ok((token, None)),
                            ExistingSSOTokenAction::Discard => {}
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("Failed to load existing token for SSO, proceeding: {e}");
                }
            }
        }
    }

    if is_vercel_login {
        let (token, token_set) =
            login_vercel_device_flow(api_client, color_config, login_url_configuration).await?;

        return match token
            .is_valid_sso(api_client, sso_team, Option::<fn(&str)>::None)
            .await
        {
            Ok(true) => Ok((token, token_set)),
            Ok(false) => Err(Error::SSOTokenExpired(sso_team.to_owned())),
            Err(err) => Err(err),
        };
    }

    let port = sso_login_callback_port.unwrap_or(DEFAULT_PORT);
    let verification_token = sso_redirect(login_url_configuration, sso_team, port).await?;

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

/// Non-Vercel SSO: open browser to `{login_url}/api/auth/sso` with
/// `teamId`, `mode`, and `next` params, then wait for localhost redirect.
/// This preserves the original SSO flow for self-hosted remote cache servers.
async fn sso_redirect(
    login_url_configuration: &str,
    sso_team: &str,
    port: u16,
) -> Result<String, Error> {
    let listener = TcpListener::bind(format!("{DEFAULT_HOST_NAME}:{port}"))
        .map_err(Error::CallbackListenerFailed)?;
    let port = listener.local_addr().unwrap().port();
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{port}");
    let state = generate_csrf_state();

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
        .append_pair("next", &redirect_url)
        .append_pair("state", &state);

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");
    let url = login_url.as_str();

    if !cfg!(test) && webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }

    let verification_token =
        tokio::task::spawn_blocking(move || wait_for_sso_redirect(listener, &state))
            .await
            .map_err(|_| Error::CallbackTaskFailed)??;

    spinner.finish_and_clear();
    Ok(verification_token)
}

/// Accept HTTP requests on the listener until one carries a `token` (or
/// `loginError`) query parameter. Browsers may send preflight, favicon, or
/// other auxiliary requests before the real redirect arrives — looping
/// prevents those from consuming the single-shot listener.
///
/// Validates the CSRF state param before accepting a callback token.
fn wait_for_sso_redirect(listener: TcpListener, expected_state: &str) -> Result<String, Error> {
    listener
        .set_nonblocking(false)
        .map_err(Error::CallbackListenerFailed)?;

    let deadline = std::time::Instant::now() + SSO_REDIRECT_TIMEOUT;

    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or(Error::CallbackTimeout)?;

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

        // Only process requests that look like the real callback
        let is_callback = params.contains_key("token") || params.contains_key("loginError");
        if !is_callback {
            let mut write_stream = stream;
            let _ = write_stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
            continue;
        }

        match params.get("state") {
            Some(returned_state) if returned_state == expected_state => {}
            _ => {
                warn!("SSO redirect state parameter mismatch or missing");
                return Err(Error::CsrfStateMismatch);
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
                Url::parse("https://vercel.com/notifications/cli-login-incomplete")
                    .expect("valid URL");
            for (k, v) in &params {
                redirect_url.query_pairs_mut().append_pair(k, v);
            }
            redirect_url.to_string()
        } else {
            let mut redirect_url = Url::parse("https://vercel.com/notifications/cli-login-success")
                .expect("valid URL");
            if let Some(email) = params.get("email") {
                redirect_url.query_pairs_mut().append_pair("email", email);
            }
            redirect_url.to_string()
        };

        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {redirect_location}\r\nConnection: close\r\n\r\n"
        );
        let mut write_stream = stream;
        if let Err(e) = write_stream.write_all(response.as_bytes()) {
            warn!("Failed to send redirect to browser: {e}");
        }

        if params.contains_key("loginError") {
            return Err(Error::LoginCallbackError);
        }

        return params
            .get("token")
            .cloned()
            .ok_or(Error::TokenMissingFromCallback);
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches, io::Write, net::TcpStream};

    use reqwest::{RequestBuilder, Response};
    use turborepo_vercel_api::{
        Membership, Role, Team, TeamsResponse, User, UserResponse, VerifiedSsoUser,
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
            Self::with_base_url("https://vercel.com/api")
        }

        fn with_base_url(base_url: &str) -> Self {
            Self {
                base_url: base_url.to_string(),
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
            token: &turborepo_api_client::SecretString,
            team_id: &str,
        ) -> turborepo_api_client::Result<Option<Team>> {
            if token.expose() == "needs-sso-token" || team_id == "needs-sso-team" {
                return Ok(None);
            }

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
            token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<ResponseTokenMetadata> {
            if token.expose() == "stale-token" {
                return Err(turborepo_api_client::Error::InvalidToken {
                    status: 200,
                    url: "https://vercel.com/api/login/oauth/token/introspect".to_string(),
                    message: "token is not active".to_string(),
                });
            }

            Ok(ResponseTokenMetadata {
                scopes: vec![Scope {
                    scope_type: "team".to_string(),
                    team_id: Some("my-team".to_string()),
                    expires_at: None,
                }],
                active_at: current_unix_time() - 100,
                client_id: None,
            })
        }
        async fn delete_token(
            &self,
            _token: &turborepo_api_client::SecretString,
        ) -> turborepo_api_client::Result<()> {
            Ok(())
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

    #[tokio::test]
    async fn test_vercel_sso_rejects_untrusted_api_url() {
        let color_config = turborepo_ui::ColorConfig::new(false);
        let api_client = MockApiClient::with_base_url("https://attacker.test/api");

        let options = LoginOptions {
            color_config: &color_config,
            login_url: "https://vercel.com",
            api_client: &api_client,
            existing_token: Some("existing-token"),
            sso_team: Some("my-team"),
            force: false,
            sso_login_callback_port: None,
        };

        let result = sso_login(&options).await;

        assert_matches!(
            result,
            Err(Error::UntrustedVercelApiUrl { ref api_url })
                if api_url == "https://attacker.test/api"
        );
    }

    #[tokio::test]
    async fn test_classify_existing_sso_token_discards_inactive_token() {
        let api_client = MockApiClient::new();

        let result = classify_existing_sso_token(
            Token::existing("stale-token".to_string()),
            &api_client,
            "my-team",
            Option::<fn(&str)>::None,
        )
        .await
        .unwrap();

        assert!(matches!(result, ExistingSSOTokenAction::Discard));
    }

    #[tokio::test]
    async fn test_classify_existing_sso_token_discards_token_missing_team_access() {
        let api_client = MockApiClient::new();

        let result = classify_existing_sso_token(
            Token::existing("needs-sso-token".to_string()),
            &api_client,
            "needs-sso-team",
            Option::<fn(&str)>::None,
        )
        .await
        .unwrap();

        assert!(matches!(result, ExistingSSOTokenAction::Discard));
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

        let result = wait_for_sso_redirect(listener, state);
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-sso-token");
    }

    #[test]
    fn test_wait_for_sso_redirect_rejects_missing_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
            let request = "GET /?token=my-sso-token HTTP/1.1\r\nHost: localhost\r\n\r\n";
            stream.write_all(request.as_bytes()).unwrap();
        });

        let result = wait_for_sso_redirect(listener, "test-state");
        handle.join().unwrap();
        assert_matches!(result, Err(Error::CsrfStateMismatch));
    }

    #[test]
    fn test_wait_for_sso_redirect_skips_non_token_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = "test-state";

        let handle = std::thread::spawn({
            let state = state.to_string();
            move || {
                // First: a request without token/loginError (e.g. favicon).
                // Listener responds with 204 and keeps waiting.
                let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                let request = format!("GET /?state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n");
                stream.write_all(request.as_bytes()).unwrap();
                let mut buf = [0u8; 256];
                let _ = std::io::Read::read(&mut stream, &mut buf);

                // Second: the real callback with a token.
                let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
                let request = format!(
                    "GET /?token=my-sso-token&state={state} HTTP/1.1\r\nHost: localhost\r\n\r\n"
                );
                stream.write_all(request.as_bytes()).unwrap();
            }
        });

        let result = wait_for_sso_redirect(listener, state);
        handle.join().unwrap();
        assert_eq!(result.unwrap(), "my-sso-token");
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

        let result = wait_for_sso_redirect(listener, "correct-state");
        handle.join().unwrap();
        assert_matches!(result, Err(Error::CsrfStateMismatch));
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

        let result = wait_for_sso_redirect(listener, state);
        handle.join().unwrap();
        assert!(result.is_err());
    }
}
