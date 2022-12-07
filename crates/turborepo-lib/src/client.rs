use std::{env, future::Future};

use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{get_version, retry::retry_future};

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
pub struct TeamsResponse {
    pub teams: Vec<Team>,
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

pub struct APIClient {
    client: reqwest::Client,
    base_url: String,
}

impl APIClient {
    pub async fn get_user(&self, token: &str) -> Result<UserResponse> {
        let response = self
            .make_retryable_request(|| {
                let url = self.make_url("/v2/user");
                let request_builder = self
                    .client
                    .get(url)
                    .header("User-Agent", USER_AGENT.clone())
                    .header("Authorization", format!("Bearer {}", token))
                    .header("Content-Type", "application/json");

                request_builder.send()
            })
            .await?;

        response.json().await.map_err(|err| {
            anyhow!(
                "Error getting user: {}",
                err.status()
                    .and_then(|status| status.canonical_reason())
                    .unwrap_or(&err.to_string())
            )
        })
    }

    pub async fn get_teams(&self, token: &str) -> Result<TeamsResponse> {
        let response = self
            .make_retryable_request(|| {
                let request_builder = self
                    .client
                    .get(self.make_url("/v2/teams?limit=100"))
                    .header("User-Agent", USER_AGENT.clone())
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", token));

                request_builder.send()
            })
            .await?;

        response.json().await.map_err(|err| {
            anyhow!(
                "Error getting teams: {}",
                err.status()
                    .and_then(|status| status.canonical_reason())
                    .unwrap_or(&err.to_string())
            )
        })
    }

    pub async fn get_team(&self, token: &str, team_id: &str) -> Result<Option<Team>> {
        let response = {
            let request_builder = self
                .client
                .get(self.make_url("/v2/team"))
                .query(&[("teamId", team_id)])
                .header("User-Agent", USER_AGENT.clone())
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token));
            request_builder.send()
        }
        .await?;

        response.json().await.map_err(|err| {
            anyhow!(
                "Error getting team: {}",
                err.status()
                    .and_then(|status| status.canonical_reason())
                    .unwrap_or(&err.to_string())
            )
        })
    }

    pub async fn get_caching_status(
        &self,
        token: &str,
        team_id: &str,
        team_slug: Option<&str>,
    ) -> Result<CachingStatusResponse> {
        let response = self
            .make_retryable_request(|| {
                let mut request_builder = self
                    .client
                    .get(self.make_url("/v8/artifacts/status"))
                    .header("User-Agent", USER_AGENT.clone())
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", token));

                if let Some(slug) = team_slug {
                    request_builder = request_builder.query(&[("teamSlug", slug)]);
                }
                if team_id.starts_with("team_") {
                    request_builder = request_builder.query(&[("teamId", team_id)]);
                }

                request_builder.send()
            })
            .await?;

        response.json().await.map_err(|err| {
            anyhow!(
                "Error getting caching status: {}",
                err.status()
                    .and_then(|status| status.canonical_reason())
                    .unwrap_or(&err.to_string())
            )
        })
    }

    pub async fn verify_sso_token(&self, token: &str, token_name: &str) -> Result<VerifiedSsoUser> {
        let response = self
            .make_retryable_request(|| {
                let request_builder = self
                    .client
                    .get(self.make_url("/registration/verify"))
                    .query(&[("token", token), ("tokenName", token_name)])
                    .header("User-Agent", USER_AGENT.clone());

                request_builder.send()
            })
            .await?;

        let verification_response: VerificationResponse = response.json().await.map_err(|err| {
            anyhow!(
                "Error verifying token: {}",
                err.status()
                    .and_then(|status| status.canonical_reason())
                    .unwrap_or(&err.to_string())
            )
        })?;
        Ok(VerifiedSsoUser {
            token: verification_response.token,
            team_id: verification_response.team_id,
        })
    }

    const RETRY_MAX: u32 = 2;

    async fn make_retryable_request<
        F: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    >(
        &self,
        request_builder: impl Fn() -> F,
    ) -> Result<reqwest::Response> {
        retry_future(Self::RETRY_MAX, request_builder, Self::should_retry_request).await
    }

    fn should_retry_request(error: &reqwest::Error) -> bool {
        if let Some(status) = error.status() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }

            if status.as_u16() >= 500 && status.as_u16() != 501 {
                return true;
            }
        }

        false
    }

    pub fn new(base_url: impl AsRef<str>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        rustc_version_runtime::version();

        Ok(APIClient {
            client,
            base_url: base_url.as_ref().to_string(),
        })
    }

    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}

lazy_static! {
    static ref USER_AGENT: String = format!(
        "turbo {} {} {} {}",
        get_version(),
        rustc_version_runtime::version(),
        env::consts::OS,
        env::consts::ARCH
    );
}
