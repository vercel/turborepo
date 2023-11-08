#![deny(clippy::all)]

use std::{collections::HashMap, fs::OpenOptions, io::Write, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{BodyStream, Path},
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::{get, head, options, patch, post, put},
    Json, Router,
};
use futures_util::StreamExt;
use tokio::sync::Mutex;
use turborepo_vercel_api::{
    AnalyticsEvent, CachingStatus, CachingStatusResponse, Membership, Role, Space, SpaceRun,
    SpacesResponse, Team, TeamsResponse, User, UserResponse, VerificationResponse,
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
pub const EXPECTED_SPACE_RUN_ID: &str = "expected_space_run_id";
pub const EXPECTED_SPACE_RUN_URL: &str = "https://example.com";

pub const EXPECTED_SSO_TEAM_ID: &str = "expected_sso_team_id";
pub const EXPECTED_SSO_TEAM_SLUG: &str = "expected_sso_team_slug";

pub async fn start_test_server(port: u16) -> Result<()> {
    let get_durations_ref = Arc::new(Mutex::new(HashMap::new()));
    let head_durations_ref = get_durations_ref.clone();
    let put_durations_ref = get_durations_ref.clone();
    let put_tempdir_ref = Arc::new(tempfile::tempdir()?);
    let get_tempdir_ref = put_tempdir_ref.clone();

    let get_analytics_events_ref = Arc::new(Mutex::new(Vec::new()));
    let post_analytics_events_ref = get_analytics_events_ref.clone();

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
            "/v0/spaces/:space_id/runs",
            post(|Path(space_id): Path<String>| async move {
                if space_id != EXPECTED_SPACE_ID {
                    return (StatusCode::NOT_FOUND, Json(None));
                }

                (
                    StatusCode::OK,
                    Json(Some(SpaceRun {
                        id: EXPECTED_SPACE_RUN_ID.to_string(),
                        url: EXPECTED_SPACE_RUN_URL.to_string(),
                    })),
                )
            }),
        )
        .route(
            "/v0/spaces/:space_id/runs/:run_id",
            patch(
                |Path((space_id, run_id)): Path<(String, String)>| async move {
                    if space_id != EXPECTED_SPACE_ID || run_id != EXPECTED_SPACE_RUN_ID {
                        return StatusCode::NOT_FOUND;
                    }

                    StatusCode::OK
                },
            ),
        )
        .route(
            "/v0/spaces/:space_id/runs/:run_id/tasks",
            post(
                |Path((space_id, run_id)): Path<(String, String)>| async move {
                    if space_id != EXPECTED_SPACE_ID || run_id != EXPECTED_SPACE_RUN_ID {
                        return StatusCode::NOT_FOUND;
                    }

                    StatusCode::OK
                },
            ),
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
        )
        .route(
            "/v8/artifacts/:hash",
            put(
                |Path(hash): Path<String>, headers: HeaderMap, mut body: BodyStream| async move {
                    let root_path = put_tempdir_ref.path();
                    let file_path = root_path.join(&hash);
                    let mut file = OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&file_path)
                        .unwrap();

                    let duration = headers
                        .get("x-artifact-duration")
                        .and_then(|header_value| header_value.to_str().ok())
                        .and_then(|duration| duration.parse::<u32>().ok())
                        .expect("x-artifact-duration header is missing");

                    let mut durations_map = put_durations_ref.lock().await;
                    durations_map.insert(hash.clone(), duration);

                    while let Some(item) = body.next().await {
                        let chunk = item.unwrap();
                        file.write_all(&chunk).unwrap();
                    }

                    (StatusCode::CREATED, Json(hash))
                },
            ),
        )
        .route(
            "/v8/artifacts/:hash",
            get(|Path(hash): Path<String>| async move {
                let root_path = get_tempdir_ref.path();
                let file_path = root_path.join(&hash);
                let Ok(buffer) = std::fs::read(file_path) else {
                    return (StatusCode::NOT_FOUND, HeaderMap::new(), Vec::new());
                };
                let duration = get_durations_ref
                    .lock()
                    .await
                    .get(&hash)
                    .cloned()
                    .unwrap_or(0);
                let mut headers = HeaderMap::new();

                headers.insert(
                    "x-artifact-duration",
                    HeaderValue::from_str(&duration.to_string()).unwrap(),
                );

                (StatusCode::FOUND, headers, buffer)
            }),
        )
        .route(
            "/v8/artifacts/:hash",
            head(|Path(hash): Path<String>| async move {
                let mut headers = HeaderMap::new();

                let Some(duration) = head_durations_ref.lock().await.get(&hash).cloned() else {
                    return (StatusCode::NOT_FOUND, headers);
                };

                headers.insert(
                    "x-artifact-duration",
                    HeaderValue::from_str(&duration.to_string()).unwrap(),
                );

                (StatusCode::OK, headers)
            }),
        )
        .route(
            "/v8/artifacts/events",
            post(
                |Json(analytics_events): Json<Vec<AnalyticsEvent>>| async move {
                    post_analytics_events_ref
                        .lock()
                        .await
                        .extend(analytics_events);
                },
            ),
        )
        .route(
            "/v8/artifacts/events",
            get(|| async move { Json(get_analytics_events_ref.lock().await.clone()) }),
        )
        .route(
            "/preflight/absolute-location",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert("Location", "http://example.com/about".parse().unwrap());

                headers
            }),
        )
        .route(
            "/preflight/relative-location",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert("Location", "/about/me".parse().unwrap());

                headers
            }),
        )
        .route(
            "/preflight/allow-auth",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Access-Control-Allow-Headers",
                    "Authorization, Location, Access-Control-Allow-Headers"
                        .parse()
                        .unwrap(),
                );

                headers
            }),
        )
        .route(
            "/preflight/no-allow-auth",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Access-Control-Allow-Headers",
                    "x-authorization-foo, Location".parse().unwrap(),
                );

                headers
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
