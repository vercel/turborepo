//! Types for interacting with the Vercel API. Used for both
//! the client (`turborepo-api-client`) and for the
//! mock server (`turborepo-vercel-api-mock`)
use serde::{Deserialize, Serialize};
use url::Url;
pub mod telemetry;
pub mod token;

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
pub struct SpaceRun {
    pub id: String,
    pub url: String,
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

#[derive(Debug)]
pub struct PreflightResponse {
    pub location: Url,
    pub allow_authorization_header: bool,
}

#[derive(Deserialize)]
pub struct APIError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum CacheSource {
    Local,
    Remote,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum CacheEvent {
    Hit,
    Miss,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub source: CacheSource,
    pub event: CacheEvent,
    pub hash: String,
    pub duration: u64,
}

impl AnalyticsEvent {
    pub fn set_session_id(&mut self, id: String) {
        self.session_id = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::{AnalyticsEvent, CacheEvent, CacheSource};

    #[test_case(
      AnalyticsEvent {
        session_id: Some("session-id".to_string()),
        source: CacheSource::Local,
        event: CacheEvent::Hit,
        hash: "this-is-my-hash".to_string(),
        duration: 58,
      },
      "with-id-local-hit"
    )]
    #[test_case(
      AnalyticsEvent {
        session_id: Some("session-id".to_string()),
        source: CacheSource::Remote,
        event: CacheEvent::Miss,
        hash: "this-is-my-hash-2".to_string(),
        duration: 21,
      },
      "with-id-remote-miss"
    )]
    #[test_case(
      AnalyticsEvent {
        session_id: None,
        source: CacheSource::Remote,
        event: CacheEvent::Miss,
        hash: "this-is-my-hash-2".to_string(),
        duration: 21,
      },
      "without-id-remote-miss"
    )]
    fn test_serialize_analytics_event(event: AnalyticsEvent, name: &str) {
        let json = serde_json::to_string(&event).unwrap();
        insta::assert_json_snapshot!(name, json);
    }
}
