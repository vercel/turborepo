use std::net::SocketAddr;

use anyhow::Result;
use axum::{routing::get, Json, Router};
use tokio::sync::OnceCell;
use turborepo_api_client::{User, UserResponse};

// We make TEST_SERVER a singleton to avoid starting the test server multiple
// times.
static TEST_SERVER: OnceCell<TestServer> = OnceCell::const_new();

const PORT: u32 = 3000;

pub async fn start() -> Result<u32> {
    TEST_SERVER.get_or_try_init(start_test_server).await?;

    Ok(PORT)
}

struct TestServer;

async fn start_test_server() -> Result<TestServer> {
    let app = Router::new()
        .route(
            "/v2/user",
            get(|| async move {
                Json(UserResponse {
                    user: User {
                        id: "my_user_id".to_string(),
                        username: "my_username".to_string(),
                        email: "my_email".to_string(),
                        name: None,
                        created_at: Some(0),
                    },
                })
            }),
        )
        .route(
            "/v2/teams",
            get(|| async move {
                Json(TeamsResponse {
                    teams: vec![Team {
                        id: TEAM_ID.to_string(),
                        slug: "vercel".to_string(),
                        name: "vercel".to_string(),
                        created_at: 0,
                        created: Default::default(),
                        membership: Membership::new(Role::Owner),
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
        );
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));

    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await?;

    Ok(TestServer)
}
