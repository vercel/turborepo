use std::net::SocketAddr;

use anyhow::Result;
use axum::{routing::get, Json, Router};
use turborepo_api_client::{
    CachingStatus, CachingStatusResponse, Membership, Role, Space, SpacesResponse, Team,
    TeamsResponse, User, UserResponse, VerificationResponse,
};

pub const EXPECTED_TOKEN: &str = "expected_token";
pub const EXPECTED_USER_ID: &str = "expected_user_id";
pub const EXPECTED_USERNAME: &str = "expected_username";
pub const EXPECTED_EMAIL: &str = "expected_email";
pub const EXPECTED_USER_CREATED_AT: Option<u64> = Some(0);

pub const EXPECTED_TEAM_ID: &str = "expected_team_id";
pub const EXPECTED_TEAM_SLUG: &str = "expected_team_slug";
pub const EXPECTED_TEAM_NAME: &str = "expected_team_name";
pub const EXPECTED_TEAM_CREATED_AT: u64 = 0;

pub const EXPECTED_SPACE_ID: &str = "expected_space_id";
pub const EXPECTED_SPACE_NAME: &str = "expected_space_name";

pub const EXPECTED_SSO_TEAM_ID: &str = "expected_sso_team_id";
pub const EXPECTED_SSO_TEAM_SLUG: &str = "expected_sso_team_slug";

pub async fn start_test_server(port: u16) -> Result<()> {
    let app = Router::new()
        .route(
            "/v2/user",
            get(|| async move {
                Json(UserResponse {
                    user: User {
                        id: EXPECTED_USER_ID.to_string(),
                        username: EXPECTED_USERNAME.to_string(),
                        email: EXPECTED_EMAIL.to_string(),
                        name: None,
                        created_at: EXPECTED_USER_CREATED_AT,
                    },
                })
            }),
        )
        .route(
            "/v2/teams",
            get(|| async move {
                Json(TeamsResponse {
                    teams: vec![Team {
                        id: EXPECTED_TEAM_ID.to_string(),
                        slug: EXPECTED_TEAM_SLUG.to_string(),
                        name: EXPECTED_TEAM_NAME.to_string(),
                        created_at: EXPECTED_TEAM_CREATED_AT,
                        created: Default::default(),
                        membership: Membership::new(Role::Owner),
                    }],
                })
            }),
        )
        .route(
            "/v0/spaces",
            get(|| async move {
                Json(SpacesResponse {
                    spaces: vec![Space {
                        id: EXPECTED_SPACE_ID.to_string(),
                        name: EXPECTED_SPACE_NAME.to_string(),
                    }],
                })
            }),
        )
        .route(
            "/v8/artifacts/status",
            get(|| async {
                Json(CachingStatusResponse {
                    status: CachingStatus::Enabled,
                })
            }),
        )
        .route(
            "/registration/verify",
            get(|| async move {
                Json(VerificationResponse {
                    token: EXPECTED_TOKEN.to_string(),
                    team_id: Some(EXPECTED_SSO_TEAM_ID.to_string()),
                })
            }),
        );
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    // We print the port so integration tests can use it
    println!("{}", port);
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
