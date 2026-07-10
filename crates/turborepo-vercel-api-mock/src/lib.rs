//! Mock server implementation for a subset of the Vercel API.

#![deny(clippy::all)]

use std::{collections::HashMap, fs::OpenOptions, io::Write, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    Form, Json, Router,
    body::Body,
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_LENGTH},
    response::IntoResponse,
    routing::{get, head, options, post, put},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::Mutex};
use turborepo_vercel_api::{
    AnalyticsEvent, CachingCapabilities, CachingStatus, CachingStatusResponse,
    IncrementalArtifactsCapability, Membership, Role, Team, TeamsResponse, User, UserResponse,
    telemetry::TelemetryEvent,
};

pub const EXPECTED_TOKEN: &str = "expected_token";
pub const EXPECTED_INCREMENTAL_PUBLISH_TOKEN: &str = "expected_incremental_publish_token";
pub const MAX_INCREMENTAL_ARTIFACT_SIZE: u64 = 1024 * 1024;
pub const EXPECTED_USER_ID: &str = "expected_user_id";
pub const EXPECTED_USERNAME: &str = "expected_username";
pub const EXPECTED_EMAIL: &str = "expected_email";

pub const EXPECTED_TEAM_ID: &str = "expected_team_id";
pub const EXPECTED_TEAM_SLUG: &str = "expected_team_slug";
pub const EXPECTED_TEAM_NAME: &str = "expected_team_name";
pub const EXPECTED_TEAM_CREATED_AT: u64 = 0;

pub const EXPECTED_SSO_TEAM_ID: &str = "expected_sso_team_id";
pub const EXPECTED_SSO_TEAM_SLUG: &str = "expected_sso_team_slug";

pub const EXPECTED_CLIENT_ID: &str = "cl_kyUx2zVvA4MGptBohkmtYHJly2XltXzD";

#[derive(Deserialize)]
struct VercelAppTokenIntrospectRequest {
    token: String,
}

#[derive(Deserialize)]
struct VercelAppTokenRevokeRequest {
    #[allow(dead_code)]
    token: String,
    client_id: String,
}

/// Per-artifact SCM metadata: (sha, dirty_hash).
type ArtifactScmMetadata = HashMap<String, (Option<String>, Option<String>)>;

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {token}"))
}

fn valid_incremental_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn incremental_headers(body: &[u8]) -> Option<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len()));
    let digest = format!("sha-256=:{}:", STANDARD.encode(Sha256::digest(body)));
    headers.insert("content-digest", HeaderValue::from_str(&digest).ok()?);
    Some(headers)
}

fn forbidden(message: &str) -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": {
                "code": "forbidden",
                "message": message,
            }
        })),
    )
        .into_response()
}

pub async fn start_test_server(
    port: u16,
    ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<()> {
    let get_durations_ref = Arc::new(Mutex::new(HashMap::new()));
    let head_durations_ref = get_durations_ref.clone();
    let put_durations_ref = get_durations_ref.clone();

    let get_metadata_ref: Arc<Mutex<ArtifactScmMetadata>> = Arc::new(Mutex::new(HashMap::new()));
    let head_metadata_ref = get_metadata_ref.clone();
    let put_metadata_ref = get_metadata_ref.clone();
    let put_tempdir_ref = Arc::new(tempfile::tempdir()?);
    let get_tempdir_ref = put_tempdir_ref.clone();

    let get_analytics_events_ref = Arc::new(Mutex::new(Vec::new()));
    let post_analytics_events_ref = get_analytics_events_ref.clone();

    let telemetry_events_ref = Arc::new(Mutex::new(Vec::new()));

    let incremental_artifacts_ref = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
    let get_incremental_artifacts_ref = incremental_artifacts_ref.clone();
    let head_incremental_artifacts_ref = incremental_artifacts_ref.clone();
    let put_incremental_artifacts_ref = incremental_artifacts_ref.clone();

    let app = Router::new()
        .route(
            "/v2/user",
            get(|headers: HeaderMap| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");

                if auth.starts_with("Bearer vca_") {
                    return StatusCode::NOT_FOUND.into_response();
                }

                Json(UserResponse {
                    user: User {
                        id: EXPECTED_USER_ID.to_string(),
                        username: EXPECTED_USERNAME.to_string(),
                        email: EXPECTED_EMAIL.to_string(),
                        name: None,
                    },
                })
                .into_response()
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
            "/v8/artifacts/status",
            get(|| async {
                Json(CachingStatusResponse {
                    status: CachingStatus::Enabled,
                    capabilities: Some(CachingCapabilities {
                        incremental_artifacts_v1: Some(IncrementalArtifactsCapability {
                            max_artifact_size: MAX_INCREMENTAL_ARTIFACT_SIZE,
                        }),
                    }),
                })
            }),
        )
        .route(
            "/v8/artifacts/incremental/{key}",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Access-Control-Allow-Headers",
                    HeaderValue::from_static(
                        "Authorization, Content-Length, Content-Type, User-Agent",
                    ),
                );
                headers
            }),
        )
        .route(
            "/v8/artifacts/incremental/{key}",
            put(
                |Path(key): Path<String>, headers: HeaderMap, body: Body| async move {
                    if !authorized(&headers, EXPECTED_INCREMENTAL_PUBLISH_TOKEN) {
                        return forbidden("token cannot publish incremental artifacts");
                    }
                    if !valid_incremental_key(&key) {
                        return StatusCode::BAD_REQUEST.into_response();
                    }
                    let Some(content_length) = headers
                        .get(CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                    else {
                        return StatusCode::BAD_REQUEST.into_response();
                    };
                    if content_length > MAX_INCREMENTAL_ARTIFACT_SIZE {
                        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
                    }

                    let mut artifact = Vec::with_capacity(content_length as usize);
                    let mut body_stream = body.into_data_stream();
                    while let Some(chunk) = body_stream.next().await {
                        let Ok(chunk) = chunk else {
                            return StatusCode::BAD_REQUEST.into_response();
                        };
                        artifact.extend_from_slice(&chunk);
                        if artifact.len() as u64 > MAX_INCREMENTAL_ARTIFACT_SIZE {
                            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
                        }
                    }
                    if artifact.len() as u64 != content_length {
                        return StatusCode::BAD_REQUEST.into_response();
                    }

                    put_incremental_artifacts_ref
                        .lock()
                        .await
                        .insert(key, artifact);
                    StatusCode::CREATED.into_response()
                },
            ),
        )
        .route(
            "/v8/artifacts/incremental/{key}",
            get(|Path(key): Path<String>, headers: HeaderMap| async move {
                if !authorized(&headers, EXPECTED_TOKEN) {
                    return forbidden("token cannot read incremental artifacts");
                }
                if !valid_incremental_key(&key) {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                let Some(artifact) = get_incremental_artifacts_ref
                    .lock()
                    .await
                    .get(&key)
                    .cloned()
                else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                let Some(headers) = incremental_headers(&artifact) else {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };
                (StatusCode::OK, headers, artifact).into_response()
            }),
        )
        .route(
            "/v8/artifacts/incremental/{key}",
            head(|Path(key): Path<String>, headers: HeaderMap| async move {
                if !authorized(&headers, EXPECTED_TOKEN) {
                    return forbidden("token cannot read incremental artifacts");
                }
                if !valid_incremental_key(&key) {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                let Some(artifact) = head_incremental_artifacts_ref
                    .lock()
                    .await
                    .get(&key)
                    .cloned()
                else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                let Some(headers) = incremental_headers(&artifact) else {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };
                (StatusCode::OK, headers).into_response()
            }),
        )
        .route(
            "/registration/verify",
            get(|| async move {
                #[derive(Serialize)]
                #[serde(rename_all = "camelCase")]
                struct MockVerificationResponse {
                    token: String,
                    team_id: Option<String>,
                }
                Json(MockVerificationResponse {
                    token: EXPECTED_TOKEN.to_string(),
                    team_id: Some(EXPECTED_SSO_TEAM_ID.to_string()),
                })
            }),
        )
        .route(
            "/v8/artifacts/{hash}",
            put(
                |Path(hash): Path<String>, headers: HeaderMap, body: Body| async move {
                    let root_path = put_tempdir_ref.path();
                    let file_path = root_path.join(&hash);
                    let mut file = match OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&file_path)
                    {
                        Ok(file) => file,
                        Err(err) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("failed to open artifact file: {err}"),
                            )
                                .into_response();
                        }
                    };

                    let Some(duration) = headers
                        .get("x-artifact-duration")
                        .and_then(|header_value| header_value.to_str().ok())
                        .and_then(|duration| duration.parse::<u32>().ok())
                    else {
                        return (
                            StatusCode::BAD_REQUEST,
                            "x-artifact-duration header is missing",
                        )
                            .into_response();
                    };

                    if headers.get(CONTENT_LENGTH).is_none() {
                        return (StatusCode::BAD_REQUEST, "content-length header is missing")
                            .into_response();
                    }

                    let mut durations_map = put_durations_ref.lock().await;
                    durations_map.insert(hash.clone(), duration);

                    let sha = headers
                        .get("x-artifact-sha")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    let dirty_hash = headers
                        .get("x-artifact-dirty-hash")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    put_metadata_ref
                        .lock()
                        .await
                        .insert(hash.clone(), (sha, dirty_hash));

                    let mut body_stream = body.into_data_stream();
                    while let Some(item) = body_stream.next().await {
                        let chunk = match item {
                            Ok(chunk) => chunk,
                            Err(err) => {
                                return (
                                    StatusCode::BAD_REQUEST,
                                    format!("failed to read artifact body: {err}"),
                                )
                                    .into_response();
                            }
                        };
                        if let Err(err) = file.write_all(&chunk) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("failed to write artifact body: {err}"),
                            )
                                .into_response();
                        }
                    }

                    (StatusCode::CREATED, Json(hash)).into_response()
                },
            ),
        )
        .route(
            "/v8/artifacts/{hash}",
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

                headers.insert("x-artifact-duration", HeaderValue::from(duration));

                if let Some((sha, dirty_hash)) = get_metadata_ref.lock().await.get(&hash).cloned() {
                    if let Some(sha) = sha {
                        let Ok(value) = HeaderValue::from_str(&sha) else {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                HeaderMap::new(),
                                Vec::new(),
                            );
                        };
                        headers.insert("x-artifact-sha", value);
                    }
                    if let Some(dirty_hash) = dirty_hash {
                        let Ok(value) = HeaderValue::from_str(&dirty_hash) else {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                HeaderMap::new(),
                                Vec::new(),
                            );
                        };
                        headers.insert("x-artifact-dirty-hash", value);
                    }
                }

                (StatusCode::FOUND, headers, buffer)
            }),
        )
        .route(
            "/v8/artifacts/{hash}",
            head(|Path(hash): Path<String>| async move {
                let mut headers = HeaderMap::new();

                let Some(duration) = head_durations_ref.lock().await.get(&hash).cloned() else {
                    return (StatusCode::NOT_FOUND, headers);
                };

                headers.insert("x-artifact-duration", HeaderValue::from(duration));

                if let Some((sha, dirty_hash)) = head_metadata_ref.lock().await.get(&hash).cloned()
                {
                    if let Some(sha) = sha {
                        let Ok(value) = HeaderValue::from_str(&sha) else {
                            return (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new());
                        };
                        headers.insert("x-artifact-sha", value);
                    }
                    if let Some(dirty_hash) = dirty_hash {
                        let Ok(value) = HeaderValue::from_str(&dirty_hash) else {
                            return (StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new());
                        };
                        headers.insert("x-artifact-dirty-hash", value);
                    }
                }

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
                headers.insert(
                    "Location",
                    HeaderValue::from_static("http://example.com/about"),
                );

                headers
            }),
        )
        .route(
            "/preflight/relative-location",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert("Location", HeaderValue::from_static("/about/me"));

                headers
            }),
        )
        .route(
            "/preflight/allow-auth",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Access-Control-Allow-Headers",
                    HeaderValue::from_static(
                        "Authorization, Location, Access-Control-Allow-Headers",
                    ),
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
                    HeaderValue::from_static("x-authorization-foo, Location"),
                );

                headers
            }),
        )
        .route(
            "/preflight/wildcard-allow-auth",
            options(|| async {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "Access-Control-Allow-Headers",
                    HeaderValue::from_static("*"),
                );

                headers
            }),
        )
        .route(
            "/login/oauth/userinfo",
            get(|headers: HeaderMap| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if auth == "Bearer vca_missing_email" {
                    Json(serde_json::json!({
                        "sub": EXPECTED_USER_ID,
                        "preferred_username": EXPECTED_USERNAME,
                    }))
                    .into_response()
                } else if auth.starts_with("Bearer vca_") {
                    Json(serde_json::json!({
                        "sub": EXPECTED_USER_ID,
                        "email": EXPECTED_EMAIL,
                        "preferred_username": EXPECTED_USERNAME,
                    }))
                    .into_response()
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }),
        )
        .route(
            "/login/oauth/token/introspect",
            post(
                |Form(form): Form<VercelAppTokenIntrospectRequest>| async move {
                    if form.token.starts_with("vca_") {
                        Json(serde_json::json!({
                            "active": true,
                            "scope": "openid",
                            "exp": 1700000000u64,
                            "iat": 1690000000u64,
                            "client_id": EXPECTED_CLIENT_ID,
                        }))
                    } else {
                        Json(serde_json::json!({ "active": false }))
                    }
                },
            ),
        )
        .route(
            "/login/oauth/token/revoke",
            post(|Form(form): Form<VercelAppTokenRevokeRequest>| async move {
                if form.client_id != EXPECTED_CLIENT_ID {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({
                            "error": "Client not allowed to revoke the token"
                        })),
                    )
                        .into_response();
                }
                StatusCode::OK.into_response()
            }),
        )
        .route(
            "/api/turborepo/v1/events",
            post(
                |Json(telemetry_events): Json<Vec<TelemetryEvent>>| async move {
                    telemetry_events_ref.lock().await.extend(telemetry_events);
                    StatusCode::OK
                },
            ),
        );

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;

    // Signal that the server is ready to accept connections
    if let Some(tx) = ready_tx {
        let _ = tx.send(());
    }

    // We print the port so integration tests can use it
    println!("{port}");
    axum::serve(listener, app).await?;

    Ok(())
}
