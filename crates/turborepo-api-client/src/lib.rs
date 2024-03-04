#![feature(async_closure)]
#![feature(error_generic_member_access)]
#![deny(clippy::all)]

use std::{backtrace::Backtrace, env};

use async_trait::async_trait;
use lazy_static::lazy_static;
use regex::Regex;
pub use reqwest::Response;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Deserialize;
use turborepo_ci::{is_ci, Vendor};
use turborepo_vercel_api::{
    token::ResponseTokenMetadata, APIError, CachingStatus, CachingStatusResponse,
    PreflightResponse, SpacesResponse, Team, TeamsResponse, UserResponse, VerificationResponse,
    VerifiedSsoUser,
};
use url::Url;

pub use crate::error::{Error, Result};

pub mod analytics;
mod error;
mod retry;
pub mod spaces;
pub mod telemetry;

lazy_static! {
    static ref AUTHORIZATION_REGEX: Regex =
        Regex::new(r"(?i)(?:^|,) *authorization *(?:,|$)").unwrap();
}

#[async_trait]
pub trait Client {
    async fn get_user(&self, token: &str) -> Result<UserResponse>;
    async fn get_teams(&self, token: &str) -> Result<TeamsResponse>;
    async fn get_team(&self, token: &str, team_id: &str) -> Result<Option<Team>>;
    fn add_ci_header(request_builder: RequestBuilder) -> RequestBuilder;
    async fn get_spaces(&self, token: &str, team_id: Option<&str>) -> Result<SpacesResponse>;
    async fn verify_sso_token(&self, token: &str, token_name: &str) -> Result<VerifiedSsoUser>;
    async fn handle_403(response: Response) -> Error;
    fn make_url(&self, endpoint: &str) -> Result<Url>;
}

#[async_trait]
pub trait CacheClient {
    async fn get_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> Result<Option<Response>>;
    async fn fetch_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<Option<Response>>;
    #[allow(clippy::too_many_arguments)]
    async fn put_artifact(
        &self,
        hash: &str,
        artifact_body: &[u8],
        duration: u64,
        tag: Option<&str>,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<()>;
    async fn artifact_exists(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<Option<Response>>;
    async fn get_caching_status(
        &self,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<CachingStatusResponse>;
}

#[async_trait]
pub trait TokenClient {
    async fn get_metadata(&self, token: &str) -> Result<ResponseTokenMetadata>;
}

#[derive(Clone)]
pub struct APIClient {
    client: reqwest::Client,
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

pub fn is_linked(api_auth: &Option<APIAuth>) -> bool {
    api_auth
        .as_ref()
        .map_or(false, |api_auth| api_auth.is_linked())
}

#[async_trait]
impl Client for APIClient {
    async fn get_user(&self, token: &str) -> Result<UserResponse> {
        let url = self.make_url("/v2/user")?;
        let request_builder = self
            .client
            .get(url)
            .header("User-Agent", self.user_agent.clone())
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");
        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    async fn get_teams(&self, token: &str) -> Result<TeamsResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v2/teams?limit=100")?)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token));

        let response = retry::make_retryable_request(request_builder)
            .await?
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
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }
    fn add_ci_header(mut request_builder: RequestBuilder) -> RequestBuilder {
        if is_ci() {
            if let Some(vendor_constant) = Vendor::get_constant() {
                request_builder = request_builder.header("x-artifact-client-ci", vendor_constant);
            }
        }

        request_builder
    }

    async fn get_spaces(&self, token: &str, team_id: Option<&str>) -> Result<SpacesResponse> {
        // create url with teamId if provided
        let endpoint = match team_id {
            Some(team_id) => format!("/v0/spaces?limit=100&teamId={}", team_id),
            None => "/v0/spaces?limit=100".to_string(),
        };

        let request_builder = self
            .client
            .get(self.make_url(endpoint.as_str())?)
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token));

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    async fn verify_sso_token(&self, token: &str, token_name: &str) -> Result<VerifiedSsoUser> {
        let request_builder = self
            .client
            .get(self.make_url("/registration/verify")?)
            .query(&[("token", token), ("tokenName", token_name)])
            .header("User-Agent", self.user_agent.clone());

        let response = retry::make_retryable_request(request_builder)
            .await?
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
                }
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
                    )
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

#[async_trait]
impl CacheClient for APIClient {
    async fn get_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
        method: Method,
    ) -> Result<Option<Response>> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{}", hash))?;
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
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }

        request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response = retry::make_retryable_request(request_builder).await?;

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
        artifact_body: &[u8],
        duration: u64,
        tag: Option<&str>,
        token: &str,
        team_id: Option<&str>,
        team_slug: Option<&str>,
    ) -> Result<()> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{}", hash))?;
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

        let mut request_builder = self
            .client
            .put(request_url)
            .header("Content-Type", "application/octet-stream")
            .header("x-artifact-duration", duration.to_string())
            .header("User-Agent", self.user_agent.clone())
            .body(artifact_body.to_vec());

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }

        request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        request_builder = Self::add_ci_header(request_builder);

        if let Some(tag) = tag {
            request_builder = request_builder.header("x-artifact-tag", tag);
        }

        let response = retry::make_retryable_request(request_builder).await?;

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
            .header("Authorization", format!("Bearer {}", token));

        let request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }
}

#[async_trait]
impl TokenClient for APIClient {
    async fn get_metadata(&self, token: &str) -> Result<ResponseTokenMetadata> {
        let url = self.make_url("/v5/user/tokens/current")?;
        let request_builder = self
            .client
            .get(url)
            .header("User-Agent", self.user_agent.clone())
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json");
        let response = retry::make_retryable_request(request_builder).await?;

        #[derive(Deserialize, Debug)]
        struct Response {
            #[serde(rename = "token")]
            metadata: ResponseTokenMetadata,
        }
        let body = response.json::<Response>().await?;
        Ok(body.metadata)
    }
}

impl APIClient {
    pub fn new(
        base_url: impl AsRef<str>,
        timeout: u64,
        version: &str,
        use_preflight: bool,
    ) -> Result<Self> {
        let client_build = if timeout != 0 {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .build()
        } else {
            reqwest::Client::builder().build()
        };

        let client = client_build.map_err(Error::TlsError)?;

        let user_agent = build_user_agent(version);
        Ok(APIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
            use_preflight,
        })
    }

    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
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
            .header("Authorization", format!("Bearer {}", token));

        let response = retry::make_retryable_request(request_builder).await?;

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
                    })
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
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
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
                .timeout(std::time::Duration::from_secs(timeout))
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
    use anyhow::Result;
    use turborepo_vercel_api_mock::start_test_server;
    use url::Url;

    use crate::{APIClient, Client};

    #[tokio::test]
    async fn test_do_preflight() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let base_url = format!("http://localhost:{}", port);

        let client = APIClient::new(&base_url, 200, "2.0.0", true)?;

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{}/preflight/absolute-location", base_url)).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await;

        assert!(response.is_ok());

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{}/preflight/relative-location", base_url)).unwrap(),
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
                Url::parse(&format!("{}/preflight/allow-auth", base_url)).unwrap(),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(response.allow_authorization_header);

        let response = client
            .do_preflight(
                "",
                Url::parse(&format!("{}/preflight/no-allow-auth", base_url)).unwrap(),
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
        assert_eq!(
            err.to_string(),
            "unable to parse 'this isn't valid JSON' as JSON: expected ident at line 1 column 2"
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
        assert_eq!(err.to_string(), "unknown status forbidden: Not authorized");
    }
}
