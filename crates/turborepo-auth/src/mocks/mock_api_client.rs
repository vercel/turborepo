use std::time::SystemTime;

use async_trait::async_trait;
use reqwest::{Method, RequestBuilder, Response};
use turborepo_api_client::Client;
use turborepo_vercel_api::{
    CachingStatusResponse, Membership, PreflightResponse, Role, Space, SpacesResponse, Team,
    TeamsResponse, User, UserResponse, VerifiedSsoUser,
};

#[derive(Debug, thiserror::Error)]
pub enum MockApiError {
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

#[derive(Default)]
pub struct MockApiClient {
    pub base_url: String,
}

impl MockApiClient {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            base_url: "custom-domain".to_string(),
        }
    }
}

#[async_trait]
impl Client for MockApiClient {
    fn base_url(&self) -> &str {
        &self.base_url
    }
    async fn get_user(&self, token: &str) -> turborepo_api_client::Result<UserResponse> {
        if token.is_empty() {
            return Err(MockApiError::EmptyToken.into());
        }

        Ok(UserResponse {
            user: User {
                id: "user id".to_string(),
                username: "username".to_string(),
                email: "email".to_string(),
                name: Some("Voz".to_string()),
                created_at: Some(
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                ),
            },
        })
    }
    async fn get_teams(&self, token: &str) -> turborepo_api_client::Result<TeamsResponse> {
        if token.is_empty() {
            return Err(MockApiError::EmptyToken.into());
        }

        Ok(TeamsResponse {
            teams: vec![Team {
                id: "team id".to_string(),
                slug: "voz-slug".to_string(),
                name: "team-voz".to_string(),
                created_at: 0,
                created: chrono::Utc::now(),
                membership: Membership::new(Role::Member),
                spaces: vec![Space {
                    id: "space-id".to_string(),
                    name: "space1 name".to_string(),
                }],
            }],
        })
    }
    async fn get_team(
        &self,
        _token: &str,
        _team_id: &str,
    ) -> turborepo_api_client::Result<Option<Team>> {
        unimplemented!("get_team")
    }
    fn add_ci_header(_request_builder: RequestBuilder) -> RequestBuilder {
        unimplemented!("add_ci_header")
    }
    fn add_team_params(
        _request_builder: RequestBuilder,
        _team_id: &str,
        _team_slug: Option<&str>,
    ) -> RequestBuilder {
        unimplemented!("add_team_params")
    }
    async fn get_caching_status(
        &self,
        _token: &str,
        _team_id: &str,
        _team_slug: Option<&str>,
    ) -> turborepo_api_client::Result<CachingStatusResponse> {
        unimplemented!("get_caching_status")
    }
    async fn get_spaces(
        &self,
        token: &str,
        _team_id: Option<&str>,
    ) -> turborepo_api_client::Result<SpacesResponse> {
        if token.is_empty() {
            return Err(MockApiError::EmptyToken.into());
        }
        Ok(SpacesResponse {
            spaces: vec![Space {
                id: "space id".to_string(),
                name: "space jam".to_string(),
            }],
        })
    }
    async fn verify_sso_token(
        &self,
        token: &str,
        _: &str,
    ) -> turborepo_api_client::Result<VerifiedSsoUser> {
        Ok(VerifiedSsoUser {
            token: token.to_string(),
            team_id: Some("team_id".to_string()),
        })
    }
    async fn put_artifact(
        &self,
        _hash: &str,
        _artifact_body: &[u8],
        _duration: u64,
        _tag: Option<&str>,
        _token: &str,
    ) -> turborepo_api_client::Result<()> {
        unimplemented!("put_artifact")
    }
    async fn handle_403(_response: Response) -> turborepo_api_client::Error {
        unimplemented!("handle_403")
    }
    async fn fetch_artifact(
        &self,
        _hash: &str,
        _token: &str,
        _team_id: &str,
        _team_slug: Option<&str>,
    ) -> turborepo_api_client::Result<Option<Response>> {
        unimplemented!("fetch_artifact")
    }
    async fn artifact_exists(
        &self,
        _hash: &str,
        _token: &str,
        _team_id: &str,
        _team_slug: Option<&str>,
    ) -> turborepo_api_client::Result<Option<Response>> {
        unimplemented!("artifact_exists")
    }
    async fn get_artifact(
        &self,
        _hash: &str,
        _token: &str,
        _team_id: &str,
        _team_slug: Option<&str>,
        _method: Method,
    ) -> turborepo_api_client::Result<Option<Response>> {
        unimplemented!("get_artifact")
    }
    async fn do_preflight(
        &self,
        _token: &str,
        _request_url: &str,
        _request_method: &str,
        _request_headers: &str,
    ) -> turborepo_api_client::Result<PreflightResponse> {
        unimplemented!("do_preflight")
    }
    fn make_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }
}
