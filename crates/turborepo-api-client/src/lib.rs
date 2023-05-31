#![feature(async_closure)]
#![feature(provide_any)]
#![feature(error_generic_member_access)]

use std::env;

use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};

pub use crate::error::{Error, Result};

mod error;
mod retry;

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

pub struct APIClient {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
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

    pub fn new(base_url: impl AsRef<str>, timeout: u64, version: &str) -> Result<Self> {
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
        })
    }

    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}
