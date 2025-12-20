//! HTTP client for interacting with the Remote Cache API.
//! Provides authentication, caching, and telemetry endpoints for Remote Cache
//! operations. By default configured for Vercel API

#![feature(error_generic_member_access)]
#![feature(assert_matches)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]

use std::{backtrace::Backtrace, env, future::Future, time::Duration};

use lazy_static::lazy_static;
use regex::Regex;
pub use reqwest::Response;
use reqwest::{Body, Method, RequestBuilder, StatusCode};
use serde::Deserialize;
use turborepo_ci::{Vendor, is_ci};
use turborepo_vercel_api::{
    APIError, CachingStatus, CachingStatusResponse, PreflightResponse, Team, TeamsResponse,
    UserResponse, VerificationResponse, VerifiedSsoUser, token::ResponseTokenMetadata,
};
use url::Url;

pub use crate::error::{Error, Result};

pub mod analytics;
mod error;
mod retry;
pub mod telemetry;

pub use bytes::Bytes;
pub use tokio_stream::Stream;

lazy_static! {
    static ref AUTHORIZATION_REGEX: Regex =
        Regex::new(r"(?i)(?:^|,) *authorization *(?:,|$)").unwrap();
}

pub trait Client {
    fn get_user(&self, token: &str) -> impl Future<Output = Result<UserResponse>> + Send;
    fn get_teams(&self, token: &str) -> impl Future<Output = Result<TeamsResponse>> + Send;
    fn get_team(
        &self,
        token: &str,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<Team>>> + Send;
    fn add_ci_header(request_builder: RequestBuilder) -> RequestBuilder;
    fn verify_sso_token(
        &self,
        token: &str,
        token_name: &str,
    ) -> impl Future<Output = Result<VerifiedSsoUser>> + Send;
    fn handle_403(response: Response) -> impl Future<Output = Error> + Send;
    fn make_url(&self, endpoint: &str) -> Result<Url>;
}

pub trait CacheClient {
    fn get_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    fn fetch_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    #[allow(clippy::too_many_arguments)]
    fn put_artifact(
        &self,
        hash: &str,
        artifact_body: impl tokio_stream::Stream<Item = Result<bytes::Bytes>> + Send + Sync + 'static,
        body_len: usize,
        duration: u64,
        tag: Option<&str>,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send;
    fn artifact_exists(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    fn get_caching_status(
        &self,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<CachingStatusResponse>> + Send;
}

pub trait TokenClient {
    fn get_metadata(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<ResponseTokenMetadata>> + Send;
    fn delete_token(&self, token: &str) -> impl Future<Output = Result<()>> + Send;
}

#[derive(Clone)]
pub struct APIClient {
    client: reqwest::Client,
    cache_client: reqwest::Client,
    base_url: String,
    user_agent: String,
    use_preflight: bool,
}

#[derive(Clone)]
pub struct APIAuth {
    pub team_id: Option<String>,
    pub token: String,
    pub team_slug: Option<String>,
}

impl std::fmt::Debug for APIAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("APIAuth")
            .field("team_id", &self.team_id)
            .field("token", &"***")
            .field("team_slug", &self.team_slug)
            .finish()
    }
}

pub fn is_linked(api_auth: &Option<APIAuth>) -> bool {
    api_auth
        .as_ref()
        .is_some_and(|api_auth| api_auth.is_linked())
}

impl Client for APIClient {
    async fn get_user(&self, token: &str) -> Result<UserResponse> {
        let url = self.make_url("/v2/user")?;
        let request_builder = self
            .client
            .get(url)
            .header("User-Agent", self.user_agent.clone())
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json");
        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        Ok(response.json().await?)
    }

    async fn get_teams(&self, token: &str) -> Result<TeamsResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v2/teams?limit=100")?)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"));

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        Ok(response.json().await?)
    }

    async fn get_team(&self, token: &str, team_id: &str) -> Result<Option<Team>> {
        let endpoint = format!("/v2/teams/{team_id}");
        let response = self
            .client
            .get(self.make_url(&endpoint)?)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }
    fn add_ci_header(mut request_builder: RequestBuilder) -> RequestBuilder {
        if is_ci()
            && let Some(vendor_constant) = Vendor::get_constant()
        {
            request_builder = request_builder.header("x-artifact-client-ci", vendor_constant);
        }

        request_builder
    }

    async fn verify_sso_token(&self, token: &str, token_name: &str) -> Result<VerifiedSsoUser> {
        let request_builder = self
            .client
            .get(self.make_url("/registration/verify")?)
            .query(&[("token", token), ("tokenName", token_name)])
            .header("User-Agent", self.user_agent.clone());

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        let verification_response: VerificationResponse = response.json().await?;

        Ok(VerifiedSsoUser {
            token: verification_response.token,
            team_id: verification_response.team_id,
        })
    }

    async fn handle_403(response: Response) -> Error {
        #[derive(Deserialize)]
        struct WrappedAPIError {
            error: APIError,
        }
        let body = match response.text().await {
            Ok(body) => body,
            Err(e) => return Error::ReqwestError(e),
        };

        let WrappedAPIError { error: api_error } = match serde_json::from_str(&body) {
            Ok(api_error) => api_error,
            Err(err) => {
                return Error::InvalidJson {
                    err,
                    text: body.clone(),
                };
            }
        };

        if let Some(status_string) = api_error.code.strip_prefix("remote_caching_") {
            let status = match status_string {
                "disabled" => CachingStatus::Disabled,
                "enabled" => CachingStatus::Enabled,
                "over_limit" => CachingStatus::OverLimit,
                "paused" => CachingStatus::Paused,
                _ => {
                    return Error::UnknownCachingStatus(
                        status_string.to_string(),
                        Backtrace::capture(),
                    );
                }
            };

            Error::CacheDisabled {
                status,
                message: api_error.message,
            }
        } else {
            Error::UnknownStatus {
                code: api_error.code,
                message: api_error.message,
                backtrace: Backtrace::capture(),
            }
        }
    }

    fn make_url(&self, endpoint: &str) -> Result<Url> {
        let url = format!("{}{}", self.base_url, endpoint);
        Url::parse(&url).map_err(|err| Error::InvalidUrl { url, err })
    }
}

impl CacheClient for APIClient {
    async fn get_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> Result<Option<Response>> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{hash}"))?;
        let mut allow_auth = true;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(
                    token,
                    request_url.clone(),
                    "GET",
                    "Authorization, User-Agent",
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location;
        };

        let mut request_builder = self
            .client
            .request(method, request_url)
            .header("User-Agent", self.user_agent.clone());

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {token}"));
        }

        request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout).await?;
        let response = response.into_response();

        match response.status() {
            StatusCode::FORBIDDEN => Err(Self::handle_403(response).await),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Ok(Some(response.error_for_status()?)),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn artifact_exists(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<Option<Response>> {
        self.get_artifact(hash, token, team_id, team_slug, Method::HEAD)
            .await
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<Option<Response>> {
        self.get_artifact(hash, token, team_id, team_slug, Method::GET)
            .await
    }

    #[tracing::instrument(skip_all)]
    async fn put_artifact(
        &self,
        hash: &str,
        artifact_body: impl tokio_stream::Stream<Item = Result<bytes::Bytes>> + Send + Sync + 'static,
        body_length: usize,
        duration: u64,
        tag: Option<&str>,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<()> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{hash}"))?;
        let mut allow_auth = true;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(
                    token,
                    request_url.clone(),
                    "PUT",
                    "Authorization, Content-Type, User-Agent, x-artifact-duration, x-artifact-tag",
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location.clone();
        }

        let stream = Body::wrap_stream(artifact_body);

        let mut request_builder = self
            .cache_client
            .put(request_url)
            .header("Content-Type", "application/octet-stream")
            .header("x-artifact-duration", duration.to_string())
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Length", body_length)
            .body(stream);

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {token}"));
        }

        request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        request_builder = Self::add_ci_header(request_builder);

        if let Some(tag) = tag {
            request_builder = request_builder.header("x-artifact-tag", tag);
        }

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Connection)
                .await?
                .into_response();

        if response.status() == StatusCode::FORBIDDEN {
            return Err(Self::handle_403(response).await);
        }

        response.error_for_status()?;
        Ok(())
    }

    async fn get_caching_status(
        &self,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<CachingStatusResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v8/artifacts/status")?)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"));

        let request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        Ok(response.json().await?)
    }
}

impl TokenClient for APIClient {
    async fn get_metadata(&self, token: &str) -> Result<ResponseTokenMetadata> {
        let endpoint = "/v5/user/tokens/current";
        let url = self.make_url(endpoint)?;
        let request_builder = self
            .client
            .get(url)
            .header("User-Agent", self.user_agent.clone())
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json");

        #[derive(Deserialize, Debug)]
        struct Response {
            #[serde(rename = "token")]
            metadata: ResponseTokenMetadata,
        }
        #[derive(Deserialize, Debug)]
        struct ErrorResponse {
            error: ErrorDetails,
        }
        #[derive(Deserialize, Debug)]
        struct ErrorDetails {
            message: String,
            #[serde(rename = "invalidToken", default)]
            invalid_token: bool,
        }

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout).await?;
        let response = response.into_response();
        let status = response.status();
        // Give a better error message for invalid tokens. This endpoint returns the
        // following statuses:
        // 200: OK
        // 400: Bad Request
        // 403: Forbidden
        // 404: Not Found
        match status {
            StatusCode::OK => Ok(response.json::<Response>().await?.metadata),
            // If we're forbidden, check to see if the token is invalid. If so, give back a nice
            // error message.
            StatusCode::FORBIDDEN => {
                let body = response.json::<ErrorResponse>().await?;
                if body.error.invalid_token {
                    return Err(Error::InvalidToken {
                        status: status.as_u16(),
                        // Call make_url again since url is moved.
                        url: self.make_url(endpoint)?.to_string(),
                        message: body.error.message,
                    });
                }
                Err(Error::ForbiddenToken {
                    url: self.make_url(endpoint)?.to_string(),
                })
            }
            _ => Err(response.error_for_status().unwrap_err().into()),
        }
    }

    /// Invalidates the given token on the server.
    async fn delete_token(&self, token: &str) -> Result<()> {
        let endpoint = "/v3/user/tokens/current";
        let url = self.make_url(endpoint)?;
        let request_builder = self
            .client
            .delete(url)
            .header("User-Agent", self.user_agent.clone())
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json");

        #[derive(Deserialize, Debug)]
        struct ErrorResponse {
            error: ErrorDetails,
        }
        #[derive(Deserialize, Debug)]
        struct ErrorDetails {
            message: String,
            #[serde(rename = "invalidToken", default)]
            invalid_token: bool,
        }

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response();
        let status = response.status();
        // Give a better error message for invalid tokens. This endpoint returns the
        // following statuses:
        // 200: OK
        // 400: Bad Request
        // 403: Forbidden
        // 404: Not Found
        match status {
            StatusCode::OK => Ok(()),
            // If we're forbidden, check to see if the token is invalid. If so, give back a nice
            // error message.
            StatusCode::FORBIDDEN => {
                let body = response.json::<ErrorResponse>().await?;
                if body.error.invalid_token {
                    return Err(Error::InvalidToken {
                        status: status.as_u16(),
                        // Call make_url again since url is moved.
                        url: self.make_url(endpoint)?.to_string(),
                        message: body.error.message,
                    });
                }
                Err(Error::ForbiddenToken {
                    url: self.make_url(endpoint)?.to_string(),
                })
            }
            _ => Err(response.error_for_status().unwrap_err().into()),
        }
    }
}

impl APIClient {
    /// Create a new APIClient.
    ///
    /// # Arguments
    /// `base_url` - The base URL for the API.
    /// `timeout` - The timeout for requests.
    /// `upload_timeout` - If specified, uploading files will use `timeout` for
    ///                    the connection, and `upload_timeout` for the total.
    ///                    Otherwise, `timeout` will be used for the total.
    /// `version` - The version of the client.
    /// `use_preflight` - If true, use the preflight API for all requests.
    pub fn new(
        base_url: impl AsRef<str>,
        timeout: Option<Duration>,
        upload_timeout: Option<Duration>,
        version: &str,
        use_preflight: bool,
    ) -> Result<Self> {
        // for the api client, the timeout applies for the entire duration
        // of the request, including the connection phase
        let client = reqwest::Client::builder();
        let client = if let Some(dur) = timeout {
            client.timeout(dur)
        } else {
            client
        }
        .build()
        .map_err(Error::TlsError)?;

        // for the cache client, the timeout applies only to the request
        // connection time, while the upload timeout applies to the entire
        // request
        let cache_client = reqwest::Client::builder();
        let cache_client = match (timeout, upload_timeout) {
            (Some(dur), Some(upload_dur)) => cache_client.connect_timeout(dur).timeout(upload_dur),
            (Some(dur), None) | (None, Some(dur)) => cache_client.timeout(dur),
            (None, None) => cache_client,
        }
        .build()
        .map_err(Error::TlsError)?;

        let user_agent = build_user_agent(version);
        Ok(APIClient {
            client,
            cache_client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
            use_preflight,
        })
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub fn with_base_url(&mut self, base_url: String) {
        self.base_url = base_url;
    }

    async fn do_preflight(
        &self,
        token: &str,
        request_url: Url,
        request_method: &str,
        request_headers: &str,
    ) -> Result<PreflightResponse> {
        let request_builder = self
            .client
            .request(Method::OPTIONS, request_url)
            .header("User-Agent", self.user_agent.clone())
            .header("Access-Control-Request-Method", request_method)
            .header("Access-Control-Request-Headers", request_headers)
            .header("Authorization", format!("Bearer {token}"));

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response();

        let headers = response.headers();
        let location = if let Some(location) = headers.get("Location") {
            let location = location.to_str()?;

            match Url::parse(location) {
                Ok(location_url) => location_url,
                Err(url::ParseError::RelativeUrlWithoutBase) => Url::parse(&self.base_url)
                    .map_err(|err| Error::InvalidUrl {
                        url: self.base_url.clone(),
                        err,
                    })?
                    .join(location)
                    .map_err(|err| Error::InvalidUrl {
                        url: location.to_string(),
                        err,
                    })?,
                Err(e) => {
                    return Err(Error::InvalidUrl {
                        url: location.to_string(),
                        err: e,
                    });
                }
            }
        } else {
            response.url().clone()
        };

        let allowed_headers = headers
            .get("Access-Control-Allow-Headers")
            .map_or("", |h| h.to_str().unwrap_or(""));

        let allow_auth = AUTHORIZATION_REGEX.is_match(allowed_headers);

        Ok(PreflightResponse {
            location,
            allow_authorization_header: allow_auth,
        })
    }
    /// Create a new request builder with the preflight check done,
    /// team parameters added, CI header, and a content type of json.
    pub(crate) async fn create_request_builder(
        &self,
        url: &str,
        api_auth: &APIAuth,
        method: Method,
    ) -> Result<RequestBuilder> {
        let mut url = self.make_url(url)?;
        let mut allow_auth = true;

        let APIAuth {
            token,
            team_id,
            team_slug,
        } = api_auth;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(
                    token,
                    url.clone(),
                    method.as_str(),
                    "Authorization, User-Agent",
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            url = preflight_response.location;
        }

        let mut request_builder = self
            .client
            .request(method, url)
            .header("Content-Type", "application/json");

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {token}"));
        }

        request_builder =
            Self::add_team_params(request_builder, team_id.as_deref(), team_slug.as_deref());

        if let Some(constant) = turborepo_ci::Vendor::get_constant() {
            request_builder = request_builder.header("x-artifact-client-ci", constant);
        }

        Ok(request_builder)
    }

    fn add_team_params(
        mut request_builder: RequestBuilder,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> RequestBuilder {
        match team_id {
            Some(team_id) if team_id.starts_with("team_") => {
                request_builder = request_builder.query(&[("teamId", team_id)]);
            }
            _ => (),
        }
        if let Some(slug) = team_slug {
            request_builder = request_builder.query(&[("slug", slug)]);
        }
        request_builder
    }
}

impl APIAuth {
    pub fn is_linked(&self) -> bool {
        self.team_id.is_some() || self.team_slug.is_some()
    }
}

// Anon Client
#[derive(Clone)]
pub struct AnonAPIClient {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
}

impl AnonAPIClient {
    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }

    pub fn new(base_url: impl AsRef<str>, timeout: u64, version: &str) -> Result<Self> {
        let client_build = if timeout != 0 {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout))
                .build()
        } else {
            reqwest::Client::builder().build()
        };

        let client = client_build.map_err(Error::TlsError)?;

        let user_agent = build_user_agent(version);
        Ok(AnonAPIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
        })
    }
}

fn build_user_agent(version: &str) -> String {
    format!(
        "turbo {} {} {} {}",
        version,
        rustc_version_runtime::version(),
        env::consts::OS,
        env::consts::ARCH
    )
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use anyhow::Result;
    use bytes::Bytes;
    use insta::assert_snapshot;
    use turborepo_vercel_api::telemetry::{TelemetryEvent, TelemetryGenericEvent};
    use turborepo_vercel_api_mock::start_test_server;
    use url::Url;

    use crate::{APIClient, AnonAPIClient, CacheClient, Client, telemetry::TelemetryClient};

    #[tokio::test]
    async fn test_do_preflight() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

        let base_url = format!("http://localhost:{port}");

        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{base_url}/preflight/absolute-location")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await;

        assert!(response.is_ok());

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{base_url}/preflight/relative-location")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await;

        // Since PreflightResponse returns a Url,
        // do_preflight would error if the Url is relative
        assert!(response.is_ok());

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{base_url}/preflight/allow-auth")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(response.allow_authorization_header);

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{base_url}/preflight/no-allow-auth")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(!response.allow_authorization_header);

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_403_includes_text_on_invalid_json() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .body("this isn't valid JSON")
                .unwrap(),
        );
        let err = APIClient::handle_403(response).await;
        assert_snapshot!(
            err.to_string(),
            @"unable to parse 'this isn't valid JSON' as JSON: expected ident at line 1 column 2"
        );
    }

    #[tokio::test]
    async fn test_handle_403_parses_error_if_present() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .body(r#"{"error": {"code": "forbidden", "message": "Not authorized"}}"#)
                .unwrap(),
        );
        let err = APIClient::handle_403(response).await;
        assert_snapshot!(err.to_string(), @"Unknown status forbidden: Not authorized");
    }

    #[tokio::test]
    async fn test_content_length() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

        let base_url = format!("http://localhost:{port}");

        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(200)),
            None,
            "2.0.0",
            true,
        )?;
        let body = b"hello world!";
        let artifact_body = tokio_stream::once(Ok(Bytes::copy_from_slice(body)));

        client
            .put_artifact(
                "eggs",
                artifact_body,
                body.len(),
                123,
                None,
                "token",
                None,
                None,
            )
            .await?;

        handle.abort();
        let _ = handle.await;

        Ok(())
    }

    #[tokio::test]
    async fn test_record_telemetry_success() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

        let base_url = format!("http://localhost:{port}");

        let client = AnonAPIClient::new(&base_url, 10, "2.0.0")?;

        let events = vec![
            TelemetryEvent::Generic(TelemetryGenericEvent {
                id: "test-id-1".to_string(),
                key: "test_key".to_string(),
                value: "test_value".to_string(),
                parent_id: None,
            }),
            TelemetryEvent::Generic(TelemetryGenericEvent {
                id: "test-id-2".to_string(),
                key: "test_key_2".to_string(),
                value: "test_value_2".to_string(),
                parent_id: Some("test-id-1".to_string()),
            }),
        ];

        let result = client
            .record_telemetry(events, "test-telemetry-id", "test-session-id")
            .await;

        assert!(result.is_ok());

        handle.abort();
        let _ = handle.await;

        Ok(())
    }

    #[tokio::test]
    async fn test_record_telemetry_empty_events() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

        let base_url = format!("http://localhost:{port}");

        let client = AnonAPIClient::new(&base_url, 10, "2.0.0")?;

        let events = vec![];

        let result = client
            .record_telemetry(events, "test-telemetry-id", "test-session-id")
            .await;

        assert!(result.is_ok());

        handle.abort();
        let _ = handle.await;

        Ok(())
    }

    #[tokio::test]
    async fn test_record_telemetry_with_different_event_types() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));

        // Wait for server to be ready
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .map_err(|_| anyhow::anyhow!("Test server failed to start"))??;

        let base_url = format!("http://localhost:{port}");

        let client = AnonAPIClient::new(&base_url, 10, "2.0.0")?;

        let events = vec![TelemetryEvent::Generic(TelemetryGenericEvent {
            id: "generic-id".to_string(),
            key: "generic_key".to_string(),
            value: "generic_value".to_string(),
            parent_id: None,
        })];

        let result = client
            .record_telemetry(events, "test-telemetry-id", "test-session-id")
            .await;

        assert!(result.is_ok());

        handle.abort();
        let _ = handle.await;

        Ok(())
    }
}
