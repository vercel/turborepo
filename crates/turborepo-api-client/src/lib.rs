#![feature(async_closure)]
#![feature(provide_any)]
#![feature(error_generic_member_access)]
#![deny(clippy::all)]

use std::{backtrace::Backtrace, env};

use lazy_static::lazy_static;
use regex::Regex;
pub use reqwest::Response;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use turborepo_ci::{is_ci, Vendor};
use url::Url;

pub use crate::error::{Error, Result};

mod error;
mod retry;

lazy_static! {
    static ref AUTHORIZATION_REGEX: Regex =
        Regex::new(r"(?i)(?:^|,) *authorization *(?:,|$)").unwrap();
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerifiedSsoUser {
    pub token: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResponse {
    pub token: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachingStatus {
    Disabled,
    Enabled,
    OverLimit,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachingStatusResponse {
    pub status: CachingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub duration: u64,
    pub expected_tag: Option<String>,
    pub body: Vec<u8>,
}

/// Membership is the relationship between the logged-in user and a particular
/// team
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    role: Role,
}

impl Membership {
    #[allow(dead_code)]
    pub fn new(role: Role) -> Self {
        Self { role }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Role {
    Member,
    Owner,
    Viewer,
    Developer,
    Billing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    pub created: chrono::DateTime<chrono::Utc>,
    pub membership: Membership,
}

impl Team {
    pub fn is_owner(&self) -> bool {
        matches!(self.membership.role, Role::Owner)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsResponse {
    pub teams: Vec<Team>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpacesResponse {
    pub spaces: Vec<Space>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub name: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserResponse {
    pub user: User,
}

pub struct PreflightResponse {
    location: Url,
    allow_authorization_header: bool,
}

#[derive(Deserialize)]
struct APIError {
    code: String,
    message: String,
}

pub struct APIClient {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
    use_preflight: bool,
}

impl APIClient {
    pub async fn get_user(&self, token: &str) -> Result<UserResponse> {
        let url = self.make_url("/v2/user");
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

    pub async fn get_teams(&self, token: &str) -> Result<TeamsResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v2/teams?limit=100"))
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token));

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn get_team(&self, token: &str, team_id: &str) -> Result<Option<Team>> {
        let response = self
            .client
            .get(self.make_url("/v2/team"))
            .query(&[("teamId", team_id)])
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

    fn add_team_params(
        mut request_builder: RequestBuilder,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> RequestBuilder {
        if let Some(slug) = team_slug {
            request_builder = request_builder.query(&[("teamSlug", slug)]);
        }
        if team_id.starts_with("team_") {
            request_builder = request_builder.query(&[("teamId", team_id)]);
        }

        request_builder
    }

    pub async fn get_caching_status(
        &self,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<CachingStatusResponse> {
        let request_builder = self
            .client
            .get(self.make_url("/v8/artifacts/status"))
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token));

        let request_builder = Self::add_team_params(request_builder, team_id, team_slug);

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn get_spaces(&self, token: &str, team_id: Option<&str>) -> Result<SpacesResponse> {
        // create url with teamId if provided
        let endpoint = match team_id {
            Some(team_id) => format!("/v0/spaces?limit=100&teamId={}", team_id),
            None => "/v0/spaces?limit=100".to_string(),
        };

        let request_builder = self
            .client
            .get(self.make_url(endpoint.as_str()))
            .header("User-Agent", self.user_agent.clone())
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token));

        let response = retry::make_retryable_request(request_builder)
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn verify_sso_token(&self, token: &str, token_name: &str) -> Result<VerifiedSsoUser> {
        let request_builder = self
            .client
            .get(self.make_url("/registration/verify"))
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

    pub async fn put_artifact(
        &self,
        hash: &str,
        artifact_body: &[u8],
        duration: u64,
        tag: Option<&str>,
        token: &str,
    ) -> Result<()> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{}", hash));
        let mut allow_auth = true;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(
                    token,
                    &request_url,
                    "PUT",
                    "Authorization, Content-Type, User-Agent, x-artifact-duration, x-artifact-tag",
                )
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location.to_string();
        }

        let mut request_builder = self
            .client
            .put(&request_url)
            .header("Content-Type", "application/octet-stream")
            .header("x-artifact-duration", duration.to_string())
            .header("User-Agent", self.user_agent.clone())
            .body(artifact_body.to_vec());

        if allow_auth {
            request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }

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

    async fn handle_403(response: Response) -> Error {
        let api_error: APIError = match response.json().await {
            Ok(api_error) => api_error,
            Err(e) => return Error::ReqwestError(e),
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

    pub async fn fetch_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<Response> {
        self.get_artifact(hash, token, team_id, team_slug, Method::GET)
            .await
    }

    pub async fn artifact_exists(
        &self,
        hash: &str,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<Response> {
        self.get_artifact(hash, token, team_id, team_slug, Method::HEAD)
            .await
    }

    async fn get_artifact(
        &self,
        hash: &str,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
        method: Method,
    ) -> Result<Response> {
        let mut request_url = self.make_url(&format!("/v8/artifacts/{}", hash));
        let mut allow_auth = true;

        if self.use_preflight {
            let preflight_response = self
                .do_preflight(token, &request_url, "GET", "Authorization, User-Agent")
                .await?;

            allow_auth = preflight_response.allow_authorization_header;
            request_url = preflight_response.location.to_string();
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

        if response.status() == StatusCode::FORBIDDEN {
            Err(Self::handle_403(response).await)
        } else {
            Ok(response.error_for_status()?)
        }
    }

    pub async fn do_preflight(
        &self,
        token: &str,
        request_url: &str,
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
                Err(url::ParseError::RelativeUrlWithoutBase) => {
                    Url::parse(&self.base_url)?.join(location)?
                }
                Err(e) => return Err(e.into()),
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

    pub fn new(
        base_url: impl AsRef<str>,
        timeout: u64,
        version: &str,
        use_preflight: bool,
    ) -> Result<Self> {
        let client = if timeout != 0 {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .build()?
        } else {
            reqwest::Client::builder().build()?
        };

        let user_agent = format!(
            "turbo {} {} {} {}",
            version,
            rustc_version_runtime::version(),
            env::consts::OS,
            env::consts::ARCH
        );
        Ok(APIClient {
            client,
            base_url: base_url.as_ref().to_string(),
            user_agent,
            use_preflight,
        })
    }

    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use vercel_api_mock::start_test_server;

    use crate::APIClient;

    #[tokio::test]
    async fn test_do_preflight() -> Result<()> {
        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let base_url = format!("http://localhost:{}", port);

        let client = APIClient::new(&base_url, 200, "2.0.0", true)?;

        let response = client
            .do_preflight(
                "",
                &format!("{}/preflight/absolute-location", base_url),
                "GET",
                "Authorization, User-Agent",
            )
            .await;

        assert!(response.is_ok());

        let response = client
            .do_preflight(
                "",
                &format!("{}/preflight/relative-location", base_url),
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
                &format!("{}/preflight/allow-auth", base_url),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(response.allow_authorization_header);

        let response = client
            .do_preflight(
                "",
                &format!("{}/preflight/no-allow-auth", base_url),
                "GET",
                "Authorization, User-Agent",
            )
            .await?;

        assert!(!response.allow_authorization_header);

        handle.abort();
        Ok(())
    }
}
