//! HTTP client for interacting with the Remote Cache API.
//! Provides authentication, caching, and telemetry endpoints for Remote Cache
//! operations. By default configured for Vercel API

#![feature(error_generic_member_access)]
// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]
#![deny(clippy::all)]

use std::{
    backtrace::Backtrace,
    collections::HashMap,
    env,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
#[cfg(feature = "rustls-tls")]
use std::{io::Cursor, path::Path};

pub use reqwest::Response;
use reqwest::{Body, Method, RequestBuilder, StatusCode};
#[cfg(feature = "rustls-tls")]
use rustls_pemfile::{self, Item};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use turborepo_ci::{Vendor, is_ci};
use turborepo_types::Credential;
use turborepo_vercel_api::{
    APIError, CachingStatus, CachingStatusResponse, PreflightResponse, Team, TeamsResponse, User,
    UserResponse, VerificationResponse, VerifiedSsoUser,
    token::{ResponseTokenMetadata, Scope},
};
use url::Url;

pub use crate::error::{Error, Result};

pub mod analytics;
mod error;
mod retry;
mod shared_http_client;
pub mod telemetry;
pub use bytes::Bytes;
pub use shared_http_client::SharedHttpClient;
pub use tokio_stream::Stream;
#[cfg(feature = "rustls-tls")]
use turborepo_tls as tls;

/// Reusable transports for generic requests and destination-restricted
/// credentials.
#[derive(Clone, Debug)]
pub struct HttpClient {
    normal: reqwest::Client,
    no_redirect: reqwest::Client,
}

impl HttpClient {
    fn transport(&self, credential: Option<&Credential>) -> &reqwest::Client {
        if credential.is_some_and(Credential::is_saved_vercel) {
            &self.no_redirect
        } else {
            &self.normal
        }
    }

    pub fn request_with_credential(
        &self,
        method: Method,
        url: Url,
        credential: Option<&Credential>,
    ) -> Result<RequestBuilder> {
        if let Some(credential) = credential {
            validate_credential_destination(credential, &url)?;
        }
        Ok(self.transport(credential).request(method, url))
    }

    pub async fn execute_with_credential(
        &self,
        request: reqwest::Request,
        credential: Option<&Credential>,
    ) -> Result<Response> {
        if let Some(credential) = credential {
            validate_credential_destination(credential, request.url())?;
        }
        Ok(self.transport(credential).execute(request).await?)
    }
}

impl std::ops::Deref for HttpClient {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.normal
    }
}

/// Only these exact HTTPS origins may receive automatically loaded Vercel
/// credentials.
pub fn is_approved_vercel_destination(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(url.host_str(), Some("vercel.com" | "api.vercel.com"))
}

pub fn validate_credential_destination(credential: &Credential, url: &Url) -> Result<()> {
    if credential.is_saved_vercel() && !is_approved_vercel_destination(url) {
        return Err(Error::SavedVercelCredentialDestination);
    }
    Ok(())
}

fn allows_authorization_header(allowed_headers: &str) -> bool {
    allowed_headers == "*"
        || allowed_headers
            .split(',')
            .any(|header| header.trim().eq_ignore_ascii_case("authorization"))
}

fn response_error(response: Response, status: StatusCode) -> Error {
    match response.error_for_status() {
        Ok(response) => Error::UnknownStatus {
            code: status.to_string(),
            message: format!("request to {} returned unexpected status", response.url()),
            backtrace: Backtrace::capture(),
        },
        Err(err) => err.into(),
    }
}

pub trait Client {
    fn get_user(&self, token: &Credential) -> impl Future<Output = Result<UserResponse>> + Send;
    fn get_teams(&self, token: &Credential) -> impl Future<Output = Result<TeamsResponse>> + Send;
    fn get_team(
        &self,
        token: &Credential,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<Team>>> + Send;
    fn add_ci_header(request_builder: RequestBuilder) -> RequestBuilder;
    fn verify_sso_token(
        &self,
        token: &Credential,
        token_name: &str,
    ) -> impl Future<Output = Result<VerifiedSsoUser>> + Send;
    fn handle_403(response: Response) -> impl Future<Output = Error> + Send;
    fn make_url(&self, endpoint: &str) -> Result<Url>;
}

/// Metadata returned for a hit by POST /v8/artifacts.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactQueryHit {
    pub task_duration_ms: u64,
    pub sha: Option<String>,
    pub dirty_hash: Option<String>,
}

pub trait CacheClient {
    fn get_artifact(
        &self,
        hash: &str,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    fn fetch_artifact(
        &self,
        hash: &str,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    #[expect(clippy::too_many_arguments)]
    fn put_artifact(
        &self,
        hash: &str,
        artifact_body: impl tokio_stream::Stream<Item = Result<bytes::Bytes>> + Send + Sync + 'static,
        body_len: usize,
        duration: u64,
        tag: Option<&str>,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        sha: Option<&str>,
        dirty_hash: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send;
    fn artifact_exists(
        &self,
        hash: &str,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<Option<Response>>> + Send;
    fn get_caching_status(
        &self,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> impl Future<Output = Result<CachingStatusResponse>> + Send;
}

pub trait TokenClient {
    fn get_metadata(
        &self,
        token: &Credential,
    ) -> impl Future<Output = Result<ResponseTokenMetadata>> + Send;
    fn delete_token(&self, token: &Credential) -> impl Future<Output = Result<()>> + Send;
}

type ArtifactRateLimitKey = (Option<String>, Option<String>, [u8; 32]);

#[derive(Clone)]
pub struct APIClient {
    client: HttpClient,
    base_url: String,
    user_agent: String,
    use_preflight: bool,
    timeout: Option<Duration>,
    upload_timeout: Option<Duration>,
    // Kept on APIClient, not the shared reqwest client: different API clients
    // (and different teams/tokens on the same client) must not block each other.
    artifact_rate_limits: Arc<Mutex<HashMap<ArtifactRateLimitKey, retry::RateLimit>>>,
}

#[derive(Clone)]
pub struct APIAuth {
    pub team_id: Option<String>,
    pub token: Credential,
    pub team_slug: Option<String>,
}

impl std::fmt::Debug for APIAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("APIAuth")
            .field("team_id", &self.team_id)
            .field("token", &self.token)
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
    #[tracing::instrument(skip_all)]
    async fn get_user(&self, token: &Credential) -> Result<UserResponse> {
        if token.expose().starts_with("vca_") {
            return self.get_vercel_app_user(token).await;
        }

        self.get_legacy_user(token).await
    }

    #[tracing::instrument(skip_all)]
    async fn get_teams(&self, token: &Credential) -> Result<TeamsResponse> {
        let request_builder = self
            .credential_request(Method::GET, self.make_url("/v2/teams?limit=100")?, token)?
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .bearer_auth(token.expose());

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        Ok(response.json().await?)
    }

    #[tracing::instrument(skip_all)]
    async fn get_team(&self, token: &Credential, team_id: &str) -> Result<Option<Team>> {
        let endpoint = format!("/v2/teams/{team_id}");
        let response = self
            .credential_request(Method::GET, self.make_url(&endpoint)?, token)?
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .bearer_auth(token.expose())
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

    #[tracing::instrument(skip_all)]
    async fn verify_sso_token(
        &self,
        token: &Credential,
        token_name: &str,
    ) -> Result<VerifiedSsoUser> {
        let request_builder = self
            .credential_request(Method::GET, self.make_url("/registration/verify")?, token)?
            .query(&[("token", token.expose()), ("tokenName", token_name)])
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

impl APIClient {
    /// Query multiple artifacts without downloading their bodies. An invalid or
    /// unsupported response is an error so callers can fall back to HEAD.
    pub async fn query_artifacts(
        &self,
        hashes: &[String],
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<HashMap<String, Option<ArtifactQueryHit>>> {
        let request = self
            .credential_request(Method::POST, self.make_url("/v8/artifacts")?, token)?
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .bearer_auth(token.expose())
            .json(&serde_json::json!({ "hashes": hashes }));
        let request = Self::add_team_params(request, team_id, team_slug);
        let response = retry::make_rate_limited_request(
            request,
            retry::RetryStrategy::Timeout,
            self.artifact_rate_limit(token, team_id, team_slug),
        )
        .await?
        .into_response();
        if response.status() == StatusCode::FORBIDDEN {
            return Err(Self::handle_403(response).await);
        }
        Ok(response.error_for_status()?.json().await?)
    }
}

impl CacheClient for APIClient {
    #[tracing::instrument(skip_all)]
    async fn get_artifact(
        &self,
        hash: &str,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> Result<Option<Response>> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{hash}"))?;
        let mut allow_auth = true;

        Self::add_team_params_to_url(&mut request_url, team_id, team_slug);

        if self.use_preflight {
            let preflight_response = self
                .do_preflight_with_rate_limit(
                    token,
                    request_url.clone(),
                    "GET",
                    "Authorization, User-Agent",
                    Some(self.artifact_rate_limit(token, team_id, team_slug)),
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location;
        };

        let mut request_builder = self
            .api_request(method, request_url, allow_auth.then_some(token))?
            .header("User-Agent", self.user_agent.clone());

        if allow_auth {
            request_builder = request_builder.bearer_auth(token.expose());
        }

        let response = retry::make_rate_limited_request(
            request_builder,
            retry::RetryStrategy::Timeout,
            self.artifact_rate_limit(token, team_id, team_slug),
        )
        .await?;
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
        token: &Credential,
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
        token: &Credential,
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
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        sha: Option<&str>,
        dirty_hash: Option<&str>,
    ) -> Result<()> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{hash}"))?;
        let mut allow_auth = true;

        Self::add_team_params_to_url(&mut request_url, team_id, team_slug);

        if self.use_preflight {
            let preflight_response = self
                .do_preflight_with_rate_limit(
                    token,
                    request_url.clone(),
                    "PUT",
                    "Authorization, Content-Type, User-Agent, x-artifact-duration, \
                     x-artifact-tag, x-artifact-sha, x-artifact-dirty-hash",
                    Some(self.artifact_rate_limit(token, team_id, team_slug)),
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location.clone();
        }

        let stream = Body::wrap_stream(artifact_body);

        let mut request_builder = self
            .upload_request(Method::PUT, request_url, allow_auth.then_some(token))?
            .header("Content-Type", "application/octet-stream")
            .header("x-artifact-duration", duration.to_string())
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Length", body_length)
            .body(stream);

        if allow_auth {
            request_builder = request_builder.bearer_auth(token.expose());
        }

        request_builder = Self::add_ci_header(request_builder);

        if let Some(tag) = tag {
            request_builder = request_builder.header("x-artifact-tag", tag);
        }

        if let Some(sha) = sha {
            request_builder = request_builder.header("x-artifact-sha", sha);
        }

        if let Some(dirty_hash) = dirty_hash {
            request_builder = request_builder.header("x-artifact-dirty-hash", dirty_hash);
        }

        let response = retry::make_rate_limited_request(
            request_builder,
            retry::RetryStrategy::Connection,
            self.artifact_rate_limit(token, team_id, team_slug),
        )
        .await?
        .into_response();

        if response.status() == StatusCode::FORBIDDEN {
            return Err(Self::handle_403(response).await);
        }

        if !response.status().is_success() {
            let status = response.status();
            return Err(response_error(response, status));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn get_caching_status(
        &self,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<CachingStatusResponse> {
        let request_builder = self
            .credential_request(Method::GET, self.make_url("/v8/artifacts/status")?, token)?
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .bearer_auth(token.expose());

        let request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response = retry::make_rate_limited_request(
            request_builder,
            retry::RetryStrategy::Timeout,
            self.artifact_rate_limit(token, team_id, team_slug),
        )
        .await?
        .into_response()
        .error_for_status()?;

        Ok(response.json().await?)
    }
}

#[derive(Deserialize, Debug)]
struct VercelAppTokenIntrospectionResponse {
    active: bool,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    iat: Option<u64>,
    #[serde(default)]
    client_id: Option<String>,
}

#[derive(Deserialize)]
struct UserinfoResponse {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

impl TokenClient for APIClient {
    #[tracing::instrument(skip_all)]
    async fn get_metadata(&self, token: &Credential) -> Result<ResponseTokenMetadata> {
        if token.expose().starts_with("vca_") {
            return self.get_vercel_app_token_metadata(token).await;
        }

        let endpoint = "/v5/user/tokens/current";
        let url = self.make_url(endpoint)?;
        let request_builder = self
            .credential_request(Method::GET, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .bearer_auth(token.expose())
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
            _ => Err(response_error(response, status)),
        }
    }

    /// Invalidates the given token on the server.
    #[tracing::instrument(skip_all)]
    async fn delete_token(&self, token: &Credential) -> Result<()> {
        if token.expose().starts_with("vca_") {
            return self.delete_vercel_app_token(token).await;
        }

        let endpoint = "/v3/user/tokens/current";
        let url = self.make_url(endpoint)?;
        let request_builder = self
            .credential_request(Method::DELETE, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .bearer_auth(token.expose())
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
            _ => Err(response_error(response, status)),
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
        let client = Self::build_http_client(timeout)?;
        let user_agent = build_user_agent(version);
        Ok(APIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
            use_preflight,
            timeout,
            upload_timeout,
            artifact_rate_limits: Arc::default(),
        })
    }

    /// Creates an APIClient using a pre-built [`HttpClient`], avoiding
    /// redundant TLS initialization when a client already exists.
    pub fn new_with_client(
        client: HttpClient,
        base_url: impl AsRef<str>,
        timeout: Option<Duration>,
        upload_timeout: Option<Duration>,
        version: &str,
        use_preflight: bool,
    ) -> Self {
        let user_agent = build_user_agent(version);
        APIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
            use_preflight,
            timeout,
            upload_timeout,
            artifact_rate_limits: Arc::default(),
        }
    }

    /// Builds a shared HTTP client with optional connect timeout. This is
    /// the single TLS initialization point — all consumers should share the
    /// resulting client.
    #[tracing::instrument(skip_all)]
    pub fn build_http_client(connect_timeout: Option<Duration>) -> Result<HttpClient> {
        Self::build_http_client_with_native_roots(connect_timeout, true)
    }

    /// Builds an HTTP client using only the bundled Mozilla CA bundle
    /// (webpki-roots). This is instant (~0ms) because no system Keychain
    /// access is needed. Sufficient for all standard HTTPS connections.
    #[tracing::instrument(skip_all)]
    pub fn build_http_client_webpki_only(connect_timeout: Option<Duration>) -> Result<HttpClient> {
        Self::build_http_client_with_native_roots(connect_timeout, false)
    }

    fn build_http_client_with_native_roots(
        connect_timeout: Option<Duration>,
        #[allow(unused_variables)] native_roots: bool,
    ) -> Result<HttpClient> {
        // Make sure rustls can verify P-521 certificate chains before we build
        // any client. ring (rustls' default provider) cannot, which breaks
        // remote caches sitting behind P-521 issuers. See `turborepo-tls`.
        #[cfg(feature = "rustls-tls")]
        tls::ensure_crypto_provider();

        let build = |#[allow(unused_variables)] use_native: bool, redirects: bool| {
            let mut builder = reqwest::Client::builder();
            if !redirects {
                builder = builder.redirect(reqwest::redirect::Policy::none());
            }
            #[cfg(feature = "rustls-tls")]
            {
                builder = builder
                    .tls_built_in_webpki_certs(true)
                    .tls_built_in_native_certs(use_native);
                // When native certs are disabled (webpki-only fast path),
                // manually load certificates from SSL_CERT_FILE / SSL_CERT_DIR
                // so that custom CAs (e.g. corporate proxies, self-hosted
                // caches) work even before the full native client is ready.
                // This is pure file I/O — no macOS Keychain — so it stays fast.
                if !use_native {
                    builder = Self::add_env_certificates(builder);
                }
            }
            if let Some(dur) = connect_timeout {
                builder = builder.connect_timeout(dur);
            }
            builder.build()
        };

        let build_pair = |use_native| {
            Ok(HttpClient {
                normal: build(use_native, true)?,
                no_redirect: build(use_native, false)?,
            })
        };
        match build_pair(native_roots) {
            Ok(client) => Ok(client),
            Err(e) if native_roots => {
                // The system certificate store may be inaccessible (e.g.
                // "Access is denied" on locked-down Windows machines).
                // Fall back to the bundled Mozilla CA bundle which doesn't
                // require any OS-level access.
                tracing::warn!(
                    "System certificate store is unavailable ({e}), using bundled certificates"
                );
                build_pair(false).map_err(Error::TlsError)
            }
            Err(e) => Err(Error::TlsError(e)),
        }
    }

    /// Loads root certificates from `SSL_CERT_FILE` and `SSL_CERT_DIR`
    /// environment variables into the given client builder.
    #[cfg(feature = "rustls-tls")]
    fn add_env_certificates(mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        if let Ok(cert_file) = env::var("SSL_CERT_FILE") {
            match std::fs::read(&cert_file) {
                Ok(pem) if !pem.is_empty() => {
                    builder =
                        Self::append_certificates_from_bytes(builder, &pem, Path::new(&cert_file));
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("Failed to read SSL_CERT_FILE ({cert_file}): {e}");
                }
            }
        }

        if let Ok(cert_dir) = env::var("SSL_CERT_DIR")
            && let Ok(entries) = std::fs::read_dir(&cert_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && let Ok(pem) = std::fs::read(&path)
                    && !pem.is_empty()
                {
                    builder = Self::append_certificates_from_bytes(builder, &pem, path.as_path());
                }
            }
        }

        builder
    }

    #[cfg(feature = "rustls-tls")]
    fn append_certificates_from_bytes(
        mut builder: reqwest::ClientBuilder,
        bytes: &[u8],
        source: &Path,
    ) -> reqwest::ClientBuilder {
        let mut cursor = Cursor::new(bytes);
        let mut added_any = false;
        let mut saw_item = false;
        for item in rustls_pemfile::read_all(&mut cursor) {
            saw_item = true;
            match item {
                Ok(Item::X509Certificate(cert_der)) => {
                    match reqwest::Certificate::from_der(&cert_der) {
                        Ok(cert) => {
                            builder = builder.add_root_certificate(cert);
                            added_any = true;
                        }
                        Err(e) => {
                            tracing::debug!(
                                "Failed to parse DER certificate from {}: {e}",
                                source.display()
                            );
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(
                        "Failed to parse certificates from {}: {e}",
                        source.display()
                    );
                }
            }
        }

        if added_any {
            return builder;
        }

        if let Ok(cert) = reqwest::Certificate::from_pem(bytes) {
            return builder.add_root_certificate(cert);
        }

        if let Ok(cert) = reqwest::Certificate::from_der(bytes) {
            return builder.add_root_certificate(cert);
        }

        if saw_item {
            tracing::debug!("No X509 certificates found in {}", source.display());
        } else {
            tracing::debug!("No certificates found in {}", source.display());
        }
        builder
    }

    fn artifact_rate_limit(
        &self,
        token: &Credential,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> retry::RateLimit {
        // Keep credentials out of the map, which lives for the lifetime of this client.
        let key = (
            team_id.map(str::to_owned),
            team_slug.map(str::to_owned),
            Sha256::digest(token.expose().as_bytes()).into(),
        );
        self.artifact_rate_limits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key)
            .or_default()
            .clone()
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub fn with_base_url(&mut self, base_url: String) {
        self.base_url = base_url;
    }

    fn credential_request(
        &self,
        method: Method,
        url: Url,
        credential: &Credential,
    ) -> Result<RequestBuilder> {
        self.api_request(method, url, Some(credential))
    }

    /// Creates a request builder with the standard API timeout applied.
    fn api_request(
        &self,
        method: Method,
        url: Url,
        credential: Option<&Credential>,
    ) -> Result<RequestBuilder> {
        let mut builder = self
            .client
            .request_with_credential(method, url, credential)?;
        if let Some(dur) = self.timeout {
            builder = builder.timeout(dur);
        }
        Ok(add_ai_agent_header(builder))
    }

    /// Creates a request builder with upload timeout semantics:
    /// connect_timeout is set on the shared client, and the total
    /// request timeout uses upload_timeout (falling back to api timeout).
    fn upload_request(
        &self,
        method: Method,
        url: Url,
        credential: Option<&Credential>,
    ) -> Result<RequestBuilder> {
        let mut builder = self
            .client
            .request_with_credential(method, url, credential)?;
        if let Some(dur) = self.upload_timeout.or(self.timeout) {
            builder = builder.timeout(dur);
        }
        Ok(add_ai_agent_header(builder))
    }

    #[tracing::instrument(skip_all)]
    async fn do_preflight(
        &self,
        token: &Credential,
        request_url: Url,
        request_method: &str,
        request_headers: &str,
    ) -> Result<PreflightResponse> {
        self.do_preflight_with_rate_limit(token, request_url, request_method, request_headers, None)
            .await
    }

    async fn do_preflight_with_rate_limit(
        &self,
        token: &Credential,
        request_url: Url,
        request_method: &str,
        request_headers: &str,
        rate_limit: Option<retry::RateLimit>,
    ) -> Result<PreflightResponse> {
        let request_builder = self
            .credential_request(Method::OPTIONS, request_url, token)?
            .header("User-Agent", self.user_agent.clone())
            .header("Access-Control-Request-Method", request_method)
            .header("Access-Control-Request-Headers", request_headers)
            .bearer_auth(token.expose());

        let response = match rate_limit {
            Some(rate_limit) => {
                retry::make_rate_limited_request(
                    request_builder,
                    retry::RetryStrategy::Timeout,
                    rate_limit,
                )
                .await?
            }
            None => {
                retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                    .await?
            }
        }
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

        let allow_auth = allows_authorization_header(allowed_headers);

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

        Self::add_team_params_to_url(&mut url, team_id.as_deref(), team_slug.as_deref());

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
            .api_request(method, url, allow_auth.then_some(token))?
            .header("Content-Type", "application/json");

        if allow_auth {
            request_builder = request_builder.bearer_auth(token.expose());
        }

        if let Some(constant) = turborepo_ci::Vendor::get_constant() {
            request_builder = request_builder.header("x-artifact-client-ci", constant);
        }

        Ok(request_builder)
    }

    /// Adds the team parameters to a URL before a preflight request is made.
    ///
    /// Remote Caches need the team to resolve a request, so the parameters have
    /// to be part of the preflight request itself. They must not be appended
    /// afterwards either: a preflight response may point at signed storage, and
    /// such URLs sign their own query string, so appending a parameter to the
    /// returned location invalidates the signature.
    fn add_team_params_to_url(url: &mut Url, team_id: Option<&str>, team_slug: Option<&str>) {
        let team_id = team_id.filter(|team_id| team_id.starts_with("team_"));

        if team_id.is_none() && team_slug.is_none() {
            return;
        }

        let mut query = url.query_pairs_mut();

        if let Some(team_id) = team_id {
            query.append_pair("teamId", team_id);
        }

        if let Some(team_slug) = team_slug {
            query.append_pair("slug", team_slug);
        }
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

    /// Introspects a Vercel App token (prefixed "vca_") using the RFC 7662
    /// endpoint.
    async fn get_vercel_app_token_metadata(
        &self,
        token: &Credential,
    ) -> Result<ResponseTokenMetadata> {
        let url = self.make_url("/login/oauth/token/introspect")?;
        let request_builder = self
            .credential_request(Method::POST, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .form(&[("token", token.expose())]);

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        let resp: VercelAppTokenIntrospectionResponse = response.json().await?;

        if !resp.active {
            return Err(Error::InvalidToken {
                status: 200,
                url: self.make_url("/login/oauth/token/introspect")?.to_string(),
                message: "token is not active".to_string(),
            });
        }

        Ok(ResponseTokenMetadata {
            scopes: resp
                .scope
                .map(|s| {
                    s.split_whitespace()
                        .map(|scope_str| Scope {
                            scope_type: scope_str.to_string(),
                            expires_at: resp.exp.map(|e| e as u128 * 1000),
                            team_id: None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            active_at: resp.iat.unwrap_or(0) as u128 * 1000,
            client_id: resp.client_id,
        })
    }

    /// Fetches user info for a Vercel App token (prefixed `vca_`) using the
    /// OpenID Connect userinfo endpoint.
    async fn get_vercel_app_user(&self, token: &Credential) -> Result<UserResponse> {
        let url = self.make_url("/login/oauth/userinfo")?;
        let request_builder = self
            .credential_request(Method::GET, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .bearer_auth(token.expose());

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        let UserinfoResponse {
            sub,
            email,
            preferred_username,
            name,
        } = response.json().await?;
        let username = preferred_username.unwrap_or_default();
        let name = name.or((!username.is_empty()).then(|| username.clone()));
        // Some OIDC userinfo responses omit email. Keep login working by
        // falling back to another stable identifier we can display to the user.
        let email = email
            .or((!username.is_empty()).then(|| username.clone()))
            .or_else(|| name.clone())
            .unwrap_or_else(|| sub.clone());

        Ok(UserResponse {
            user: User {
                id: sub,
                name,
                username,
                email,
            },
        })
    }

    async fn get_legacy_user(&self, token: &Credential) -> Result<UserResponse> {
        let url = self.make_url("/v2/user")?;
        let request_builder = self
            .credential_request(Method::GET, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .bearer_auth(token.expose())
            .header("Content-Type", "application/json");
        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        Ok(response.json().await?)
    }

    /// Revokes a Vercel App token (prefixed "vca_") using the RFC 7009
    /// endpoint. Introspects the token first to discover the correct client_id.
    async fn delete_vercel_app_token(&self, token: &Credential) -> Result<()> {
        let client_id = self.introspect_client_id(token).await?;

        let url = self.make_url("/login/oauth/token/revoke")?;
        let request_builder = self
            .credential_request(Method::POST, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .form(&[("token", token.expose()), ("client_id", &client_id)]);

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response();
        let status = response.status();
        if !status.is_success() {
            return Err(response_error(response, status));
        }

        Ok(())
    }

    /// Introspects a token to discover its client_id.
    async fn introspect_client_id(&self, token: &Credential) -> Result<String> {
        let url = self.make_url("/login/oauth/token/introspect")?;
        let request_builder = self
            .credential_request(Method::POST, url, token)?
            .header("User-Agent", self.user_agent.clone())
            .form(&[("token", token.expose())]);

        let response =
            retry::make_retryable_request(request_builder, retry::RetryStrategy::Timeout)
                .await?
                .into_response()
                .error_for_status()?;

        let resp: VercelAppTokenIntrospectionResponse = response.json().await?;
        resp.client_id.ok_or_else(|| Error::InvalidToken {
            status: 200,
            url: self
                .make_url("/login/oauth/token/introspect")
                .map(|u| u.to_string())
                .unwrap_or_default(),
            message: "token is not active".to_string(),
        })
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

    /// Creates an AnonAPIClient using a pre-built `reqwest::Client`, avoiding
    /// redundant TLS initialization.
    pub fn new_with_client(
        client: reqwest::Client,
        base_url: impl AsRef<str>,
        version: &str,
    ) -> Self {
        let user_agent = build_user_agent(version);
        AnonAPIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
        }
    }
}

pub(crate) fn add_ai_agent_header(builder: RequestBuilder) -> RequestBuilder {
    match turborepo_ai_agents::get_agent() {
        Some(agent) => builder.header("x-ai-agent", agent),
        None => builder,
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
    use reqwest::{Method, StatusCode};
    use turborepo_vercel_api::telemetry::{TelemetryEvent, TelemetryGenericEvent};
    use turborepo_vercel_api_mock::start_test_server;
    use url::Url;

    use crate::{
        APIAuth, APIClient, AnonAPIClient, CacheClient, CachingStatus, Client, Credential, Error,
        TokenClient, telemetry::TelemetryClient,
    };

    #[tokio::test]
    async fn saved_vercel_credential_never_reaches_custom_cache() -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let received = server
            .mock_async(|when, then| {
                when.header("authorization", "Bearer synthetic-saved-vercel-sentinel");
                then.status(200).body("artifact");
            })
            .await;
        let client = APIClient::new(server.base_url(), None, None, "test", false)?;
        let saved = Credential::saved_vercel("synthetic-saved-vercel-sentinel".into());
        let result = client.fetch_artifact("sentinel", &saved, None, None).await;
        received.assert_calls_async(0).await;
        assert!(matches!(
            result,
            Err(Error::SavedVercelCredentialDestination)
        ));
        let generic = Credential::generic("synthetic-saved-vercel-sentinel".into());
        assert!(
            client
                .fetch_artifact("sentinel", &generic, None, None)
                .await?
                .is_some()
        );
        received.assert_calls_async(1).await;
        Ok(())
    }

    #[test]
    fn saved_vercel_destinations_are_exact_https_origins() {
        for approved in [
            "https://vercel.com/api",
            "https://api.vercel.com/login",
            "https://vercel.com:443/path",
        ] {
            assert!(
                super::is_approved_vercel_destination(&Url::parse(approved).unwrap()),
                "{approved}"
            );
        }
        for rejected in [
            "http://vercel.com/api",
            "https://vercel.com:8443",
            "https://foo.vercel.com",
            "https://vercel.com.evil.test",
            "https://vercel.com@evil.test",
            "https://user@vercel.com",
            "https://:password@api.vercel.com",
            "https://vercel.com.",
            "https://127.0.0.1",
            "ftp://vercel.com",
        ] {
            assert!(
                !super::is_approved_vercel_destination(&Url::parse(rejected).unwrap()),
                "{rejected}"
            );
        }
    }

    #[tokio::test]
    async fn every_credential_transport_blocks_saved_vercel_before_sending() -> anyhow::Result<()> {
        use crate::analytics::AnalyticsClient;
        let server = httpmock::MockServer::start_async().await;
        let requests = server
            .mock_async(|when, then| {
                when.any_request();
                then.status(200).json_body(serde_json::json!({}));
            })
            .await;
        for preflight in [false, true] {
            let client = APIClient::new_with_client(
                APIClient::build_http_client(None)?,
                server.base_url(),
                None,
                None,
                "test",
                preflight,
            );
            for secret in ["synthetic-legacy-sentinel", "vca_synthetic-oauth-sentinel"] {
                let token = Credential::saved_vercel(secret.into());
                assert!(matches!(
                    client.get_user(&token).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.get_teams(&token).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.get_team(&token, "team_test").await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.get_metadata(&token).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.delete_token(&token).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.verify_sso_token(&token, "test").await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.get_caching_status(&token, None, None).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.fetch_artifact("sentinel", &token, None, None).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client.artifact_exists("sentinel", &token, None, None).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client
                        .put_artifact(
                            "sentinel",
                            tokio_stream::iter(vec![Ok(Bytes::from_static(b"sentinel"))]),
                            8,
                            0,
                            None,
                            &token,
                            None,
                            None,
                            None,
                            None
                        )
                        .await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                assert!(matches!(
                    client
                        .query_artifacts(&["sentinel".into()], &token, None, None)
                        .await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
                let auth = APIAuth {
                    token,
                    team_id: None,
                    team_slug: None,
                };
                assert!(matches!(
                    client.record_analytics(&auth, vec![]).await,
                    Err(Error::SavedVercelCredentialDestination)
                ));
            }
        }
        requests.assert_calls_async(0).await;
        Ok(())
    }

    #[tokio::test]
    async fn generic_requests_follow_same_origin_redirects() -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let destination = server
            .mock_async(|when, then| {
                when.path("/redirected")
                    .header("authorization", "Bearer synthetic-sentinel");
                then.status(200)
                    .header("access-control-allow-headers", "authorization");
            })
            .await;
        server
            .mock_async(|when, then| {
                when.path("/v8/artifacts/sentinel");
                then.status(307)
                    .header("location", server.url("/redirected"));
            })
            .await;
        server
            .mock_async(|when, then| {
                when.path("/v3/user/tokens/current");
                then.status(307)
                    .header("location", server.url("/redirected"));
            })
            .await;
        let client = APIClient::new(server.base_url(), None, None, "test", false)?;
        let token = Credential::generic("synthetic-sentinel".into());
        for method in [Method::GET, Method::HEAD] {
            let response = client
                .get_artifact("sentinel", &token, None, None, method)
                .await?
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.url().path(), "/redirected");
        }
        client.delete_token(&token).await?;
        let preflight = client
            .do_preflight(
                &token,
                client.make_url("/v8/artifacts/sentinel")?,
                "GET",
                "Authorization",
            )
            .await?;
        assert!(preflight.allow_authorization_header);
        assert_eq!(preflight.location.path(), "/redirected");
        destination.assert_calls_async(4).await;
        Ok(())
    }

    #[tokio::test]
    async fn unhandled_upload_redirect_is_an_error() -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let redirect = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::PUT)
                    .path("/v8/artifacts/sentinel");
                then.status(307)
                    .header("location", server.url("/redirected"));
            })
            .await;
        let client = APIClient::new(server.base_url(), None, None, "test", false)?;
        let result = client
            .put_artifact(
                "sentinel",
                tokio_stream::iter(vec![Ok(Bytes::from_static(b"sentinel"))]),
                8,
                0,
                None,
                &Credential::generic("synthetic-sentinel".into()),
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(
            result.is_err(),
            "an unhandled upload redirect must not count as a cache write"
        );
        redirect.assert_calls_async(1).await;
        Ok(())
    }

    #[tokio::test]
    async fn oauth_revocation_requires_success_status() -> anyhow::Result<()> {
        for status in [200, 204, 302, 307, 308, 400, 501] {
            let server = httpmock::MockServer::start_async().await;
            let introspection = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::POST)
                        .path("/login/oauth/token/introspect");
                    then.status(200).json_body(
                        serde_json::json!({ "active": true, "client_id": "test-client" }),
                    );
                })
                .await;
            let revocation = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::POST)
                        .path("/login/oauth/token/revoke");
                    then.status(status)
                        .header("location", server.url("/redirected"));
                })
                .await;
            let destination = server
                .mock_async(|when, then| {
                    when.path("/redirected");
                    then.status(200);
                })
                .await;
            let mut transport = APIClient::build_http_client(None)?;
            // Exercise saved credentials' redirect policy with a generic test token
            // so the loopback server does not need an exception to the origin allowlist.
            transport.normal = transport.no_redirect.clone();
            let client =
                APIClient::new_with_client(transport, server.base_url(), None, None, "test", false);
            let result = client
                .delete_token(&Credential::generic("vca_synthetic-sentinel".into()))
                .await;
            assert_eq!(
                result.is_ok(),
                (200..300).contains(&status),
                "HTTP {status}"
            );
            if let Err(error) = result {
                assert!(!error.to_string().contains("vca_synthetic-sentinel"));
            }
            introspection.assert_calls_async(1).await;
            revocation.assert_calls_async(1).await;
            destination.assert_calls_async(0).await;
        }
        Ok(())
    }

    #[tokio::test]
    async fn saved_transport_does_not_follow_redirects() -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let destination = server
            .mock_async(|when, then| {
                when.path("/redirected");
                then.status(200);
            })
            .await;
        let redirect = server
            .mock_async(|when, then| {
                when.path("/source");
                then.status(307)
                    .header("location", server.url("/redirected"));
            })
            .await;
        let client = APIClient::build_http_client(None)?;
        let saved = Credential::saved_vercel("synthetic-sentinel".into());
        for method in [
            Method::GET,
            Method::HEAD,
            Method::OPTIONS,
            Method::POST,
            Method::PUT,
        ] {
            let (transport, request) = client
                .request_with_credential(
                    method,
                    Url::parse("https://api.vercel.com/source")?,
                    Some(&saved),
                )?
                .bearer_auth(saved.expose())
                .build_split();
            let mut request = request?;
            *request.url_mut() = Url::parse(&server.url("/source"))?;
            let response = transport.execute(request).await?;
            assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        }
        destination.assert_calls_async(0).await;
        let response = client
            .transport(None)
            .get(server.url("/source"))
            .send()
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        destination.assert_calls_async(1).await;
        redirect.assert_calls_async(6).await;
        Ok(())
    }

    #[test]
    fn api_auth_debug_redacts_token() {
        let auth = APIAuth {
            team_id: Some("team-123".to_string()),
            token: Credential::generic("real-secret-token".to_string().into()),
            team_slug: Some("my-team".to_string()),
        };
        let debug = format!("{:?}", auth);
        assert!(
            !debug.contains("real-secret-token"),
            "Debug output should not contain the raw token"
        );
        assert!(debug.contains("***"));
        assert!(debug.contains("team-123"));
        assert!(debug.contains("my-team"));
    }

    #[test]
    fn add_team_params_to_url_appends_slug() {
        let mut url = Url::parse("https://cache.example/v8/artifacts/abc123").unwrap();

        APIClient::add_team_params_to_url(&mut url, None, Some("my-team"));

        assert_eq!(
            url.as_str(),
            "https://cache.example/v8/artifacts/abc123?slug=my-team"
        );
    }

    #[test]
    fn add_team_params_to_url_appends_team_id_and_slug() {
        let mut url = Url::parse("https://cache.example/v8/artifacts/abc123").unwrap();

        APIClient::add_team_params_to_url(&mut url, Some("team_123"), Some("my-team"));

        assert_eq!(
            url.as_str(),
            "https://cache.example/v8/artifacts/abc123?teamId=team_123&slug=my-team"
        );
    }

    #[test]
    fn add_team_params_to_url_ignores_team_id_without_prefix() {
        let mut url = Url::parse("https://cache.example/v8/artifacts/abc123").unwrap();

        APIClient::add_team_params_to_url(&mut url, Some("123"), Some("my-team"));

        assert_eq!(
            url.as_str(),
            "https://cache.example/v8/artifacts/abc123?slug=my-team"
        );
    }

    #[test]
    fn add_team_params_to_url_leaves_url_untouched_when_unlinked() {
        let mut url = Url::parse("https://cache.example/v8/artifacts/abc123").unwrap();

        APIClient::add_team_params_to_url(&mut url, None, None);

        assert_eq!(url.as_str(), "https://cache.example/v8/artifacts/abc123");
    }

    #[tokio::test]
    async fn query_artifacts_sends_one_authenticated_batch_and_parses_metadata()
    -> anyhow::Result<()> {
        let server = httpmock::MockServer::start_async().await;
        let query = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/v8/artifacts")
                    .query_param("teamId", "team_123")
                    .query_param("slug", "my-team")
                    .header("authorization", "Bearer secret-token")
                    .json_body(serde_json::json!({"hashes": ["hit", "miss"]}));
                then.status(200).json_body(serde_json::json!({
                    "hit": {"size": 12, "taskDurationMs": 456, "sha": "abc", "dirtyHash": "def"},
                    "miss": null
                }));
            })
            .await;
        let client = APIClient::new(server.base_url(), None, None, "2.0.0", false)?;
        let results = client
            .query_artifacts(
                &["hit".into(), "miss".into()],
                &Credential::generic("secret-token".into()),
                Some("team_123"),
                Some("my-team"),
            )
            .await?;
        query.assert_calls_async(1).await;
        let hit = results.get("hit").unwrap().as_ref().unwrap();
        assert_eq!(hit.task_duration_ms, 456);
        assert_eq!(hit.sha.as_deref(), Some("abc"));
        assert_eq!(hit.dirty_hash.as_deref(), Some("def"));
        assert!(results.get("miss").unwrap().is_none());
        Ok(())
    }

    #[tokio::test]
    async fn fetch_artifact_does_not_leak_credentials_to_preflight_location() -> anyhow::Result<()>
    {
        let storage = httpmock::MockServer::start_async().await;
        let get = storage
            .mock_async(|w, t| {
                w.method(httpmock::Method::GET)
                    .path("/signed/artifact")
                    .query_param("sig", "abc")
                    .query_param_missing("teamId")
                    .query_param_missing("slug")
                    .header_missing("authorization");
                t.status(200).body("artifact");
            })
            .await;
        let cache = httpmock::MockServer::start_async().await;
        let signed_redirect = storage
            .mock_async(|w, t| {
                w.method(httpmock::Method::GET)
                    .path("/signed/redirect")
                    .query_param("sig", "abc")
                    .header_missing("authorization");
                t.status(307).header("Location", "/signed/artifact?sig=abc");
            })
            .await;
        let location = format!("{}/signed/redirect?sig=abc", storage.base_url());
        let options = cache
            .mock_async(|w, t| {
                w.method(httpmock::Method::OPTIONS)
                    .path("/v8/artifacts/security-test")
                    .query_param("slug", "my-team");
                t.status(200).header("Location", &location);
            })
            .await;
        let client = APIClient::new(
            cache.base_url(),
            Some(Duration::from_secs(10)),
            None,
            "2.0.0",
            true,
        )?;
        let token = Credential::generic("secret-token".into());
        let response = client
            .fetch_artifact("security-test", &token, None, Some("my-team"))
            .await?
            .unwrap();
        assert_eq!(response.text().await?, "artifact");
        options.assert_calls_async(1).await;
        signed_redirect.assert_calls_async(1).await;
        get.assert_calls_async(1).await;
        Ok(())
    }

    #[tokio::test]
    async fn test_do_preflight() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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

        let empty_token = Credential::generic(String::new().into());

        let response = client
            .do_preflight(
                &empty_token,
                Url::parse(&format!("{base_url}/preflight/absolute-location")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await;

        assert!(response.is_ok());

        let response = client
            .do_preflight(
                &empty_token,
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
                &empty_token,
                Url::parse(&format!("{base_url}/preflight/allow-auth")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(response.allow_authorization_header);

        let response = client
            .do_preflight(
                &empty_token,
                Url::parse(&format!("{base_url}/preflight/no-allow-auth")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(!response.allow_authorization_header);

        let response = client
            .do_preflight(
                &empty_token,
                Url::parse(&format!("{base_url}/preflight/wildcard-allow-auth")).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(response.allow_authorization_header);

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
    async fn test_handle_403_maps_remote_caching_status() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .body(r#"{"error": {"code": "remote_caching_paused", "message": "Paused"}}"#)
                .unwrap(),
        );
        let err = APIClient::handle_403(response).await;
        assert!(
            matches!(
                err,
                Error::CacheDisabled {
                    status: CachingStatus::Paused,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn test_content_length() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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

        let token = Credential::generic("token".to_string().into());
        client
            .put_artifact(
                "eggs",
                artifact_body,
                body.len(),
                123,
                None,
                &token,
                None,
                None,
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
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
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

    #[tokio::test]
    async fn test_get_user() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            None,
            "2.0.0",
            false,
        )?;
        let token = Credential::generic("test-token".to_string().into());

        let response = client.get_user(&token).await?;
        assert_eq!(
            response.user.id,
            turborepo_vercel_api_mock::EXPECTED_USER_ID
        );
        assert_eq!(
            response.user.username,
            turborepo_vercel_api_mock::EXPECTED_USERNAME
        );
        assert_eq!(
            response.user.email,
            turborepo_vercel_api_mock::EXPECTED_EMAIL
        );

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_get_teams() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            None,
            "2.0.0",
            false,
        )?;
        let token = Credential::generic("test-token".to_string().into());

        let response = client.get_teams(&token).await?;
        assert_eq!(response.teams.len(), 1);
        assert_eq!(
            response.teams[0].id,
            turborepo_vercel_api_mock::EXPECTED_TEAM_ID
        );
        assert_eq!(
            response.teams[0].slug,
            turborepo_vercel_api_mock::EXPECTED_TEAM_SLUG
        );

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_get_caching_status() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            None,
            "2.0.0",
            false,
        )?;
        let token = Credential::generic("test-token".to_string().into());

        let response = client.get_caching_status(&token, None, None).await?;
        assert!(matches!(
            response.status,
            turborepo_vercel_api::CachingStatus::Enabled
        ));

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_put_and_fetch_artifact() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            Some(Duration::from_secs(10)),
            "2.0.0",
            false,
        )?;
        let token = Credential::generic("test-token".to_string().into());

        let body = b"cached artifact data";
        let artifact_body = tokio_stream::once(Ok(Bytes::copy_from_slice(body)));
        client
            .put_artifact(
                "test-hash-123",
                artifact_body,
                body.len(),
                456,
                None,
                &token,
                None,
                None,
                None,
                None,
            )
            .await?;

        let response = client
            .fetch_artifact("test-hash-123", &token, None, None)
            .await?;
        assert!(response.is_some());

        let exists = client
            .artifact_exists("test-hash-123", &token, None, None)
            .await?;
        assert!(exists.is_some());

        let missing = client
            .artifact_exists("nonexistent-hash", &token, None, None)
            .await?;
        assert!(missing.is_none());

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_api_client_with_upload_timeout() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        // Ensure separate timeout and upload_timeout both work
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            Some(Duration::from_secs(30)),
            "2.0.0",
            false,
        )?;
        let token = Credential::generic("test-token".to_string().into());

        // Regular API calls should work (uses client with regular timeout)
        let user = client.get_user(&token).await?;
        assert_eq!(user.user.id, turborepo_vercel_api_mock::EXPECTED_USER_ID);

        // Cache upload should work (uses cache_client with upload timeout)
        let body = b"upload timeout test";
        let artifact_body = tokio_stream::once(Ok(Bytes::copy_from_slice(body)));
        client
            .put_artifact(
                "upload-test",
                artifact_body,
                body.len(),
                789,
                None,
                &token,
                None,
                None,
                None,
                None,
            )
            .await?;

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_api_client_no_timeout() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        // No timeout specified — should still work
        let client = APIClient::new(&base_url, None, None, "2.0.0", false)?;
        let token = Credential::generic("test-token".to_string().into());

        let user = client.get_user(&token).await?;
        assert_eq!(user.user.id, turborepo_vercel_api_mock::EXPECTED_USER_ID);

        let body = b"no timeout test";
        let artifact_body = tokio_stream::once(Ok(Bytes::copy_from_slice(body)));
        client
            .put_artifact(
                "no-timeout",
                artifact_body,
                body.len(),
                100,
                None,
                &token,
                None,
                None,
                None,
                None,
            )
            .await?;

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_anon_client_no_timeout() -> Result<()> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        // timeout=0 means no timeout
        let client = AnonAPIClient::new(&base_url, 0, "2.0.0")?;

        let events = vec![TelemetryEvent::Generic(TelemetryGenericEvent {
            id: "no-timeout-id".to_string(),
            key: "key".to_string(),
            value: "value".to_string(),
            parent_id: None,
        })];

        let result = client
            .record_telemetry(events, "test-id", "test-session")
            .await;
        assert!(result.is_ok());

        handle.abort();
        Ok(())
    }

    /// Starts a mock server and returns an APIClient pointed at it, along with
    /// the server handle for cleanup.
    async fn start_vca_test_client() -> Result<(APIClient, tokio::task::JoinHandle<Result<()>>)> {
        let port = turborepo_vercel_api_mock::request_open_port().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(start_test_server(port, Some(ready_tx)));
        tokio::time::timeout(Duration::from_secs(5), ready_rx).await??;

        let base_url = format!("http://localhost:{port}");
        let client = APIClient::new(
            &base_url,
            Some(Duration::from_secs(5)),
            None,
            "2.0.0",
            false,
        )?;
        Ok((client, handle))
    }

    #[tokio::test]
    async fn test_get_user_vca_token() -> Result<()> {
        let (client, handle) = start_vca_test_client().await?;
        let token = Credential::generic("vca_test_token".to_string().into());

        let response = client.get_user(&token).await?;
        assert_eq!(
            response.user.id,
            turborepo_vercel_api_mock::EXPECTED_USER_ID
        );
        assert_eq!(
            response.user.email,
            turborepo_vercel_api_mock::EXPECTED_EMAIL
        );
        assert_eq!(
            response.user.username,
            turborepo_vercel_api_mock::EXPECTED_USERNAME
        );
        assert_eq!(
            response.user.name,
            Some(turborepo_vercel_api_mock::EXPECTED_USERNAME.to_string())
        );

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_get_user_vca_token_without_email() -> Result<()> {
        let (client, handle) = start_vca_test_client().await?;
        let token = Credential::generic("vca_missing_email".to_string().into());

        let response = client.get_user(&token).await?;
        assert_eq!(
            response.user.id,
            turborepo_vercel_api_mock::EXPECTED_USER_ID
        );
        assert_eq!(
            response.user.email,
            turborepo_vercel_api_mock::EXPECTED_USERNAME
        );
        assert_eq!(
            response.user.username,
            turborepo_vercel_api_mock::EXPECTED_USERNAME
        );
        assert_eq!(
            response.user.name,
            Some(turborepo_vercel_api_mock::EXPECTED_USERNAME.to_string())
        );

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_get_metadata_vca_token() -> Result<()> {
        let (client, handle) = start_vca_test_client().await?;
        let token = Credential::generic("vca_test_token".to_string().into());

        let metadata = client.get_metadata(&token).await?;
        assert_eq!(metadata.scopes.len(), 1);
        assert_eq!(metadata.scopes[0].scope_type, "openid");
        assert_eq!(metadata.scopes[0].expires_at, Some(1700000000u128 * 1000));
        assert_eq!(metadata.active_at, 1690000000u128 * 1000);
        assert_eq!(
            metadata.client_id.as_deref(),
            Some(turborepo_vercel_api_mock::EXPECTED_CLIENT_ID)
        );

        handle.abort();
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_token_vca_token() -> Result<()> {
        let (client, handle) = start_vca_test_client().await?;
        let token = Credential::generic("vca_test_token".to_string().into());

        let result = client.delete_token(&token).await;
        assert!(result.is_ok());

        handle.abort();
        Ok(())
    }
}
