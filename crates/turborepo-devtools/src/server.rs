//! WebSocket server for devtools.
//!
//! Provides a WebSocket endpoint that clients can connect to receive
//! real-time graph updates as the repository changes.

use std::sync::Arc;

use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures::{SinkExt, StreamExt};
use rand::{
    distr::{Alphanumeric, SampleString},
    rng,
};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{RwLock, broadcast},
};
use tracing::{debug, error, info, warn};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{package_graph::PackageGraphBuilder, package_json::PackageJson};

use crate::{
    graph::package_graph_to_data,
    types::{GraphState, ServerMessage, TaskGraphBuilder},
    watcher::{DevtoolsWatcher, WatchEvent},
};

const AUTH_TOKEN_LENGTH: usize = 32;

/// Errors that can occur in the devtools server
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Failed to bind to port {port}: {source}")]
    Bind {
        port: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("Server error: {0}")]
    Server(#[from] std::io::Error),

    #[error("Failed to build package graph: {0}")]
    PackageGraph(String),

    #[error("Failed to load package.json: {0}")]
    PackageJson(String),

    #[error("File watcher error: {0}")]
    Watcher(#[from] crate::watcher::WatchError),

    #[error("Failed to build task graph: {0}")]
    TaskGraph(String),
}

/// Shared state for the WebSocket server
#[derive(Clone)]
struct AppState {
    /// Current graph state
    graph_state: Arc<RwLock<GraphState>>,
    /// Channel to notify clients of updates
    update_tx: broadcast::Sender<()>,
    /// Per-session token required for WebSocket upgrades
    auth_token: String,
    /// Browser origin allowed to connect to this local server
    allowed_origin: String,
}

/// The devtools WebSocket server
pub struct DevtoolsServer<T: TaskGraphBuilder> {
    repo_root: AbsoluteSystemPathBuf,
    port: u16,
    task_graph_builder: T,
    auth_token: String,
    allowed_origin: String,
}

impl<T: TaskGraphBuilder + 'static> DevtoolsServer<T> {
    /// Creates a new devtools server with a task graph builder.
    ///
    /// The task graph builder should use the same logic as `turbo run`
    /// to ensure consistency between what the devtools shows and what
    /// turbo actually executes.
    pub fn new(
        repo_root: AbsoluteSystemPathBuf,
        port: u16,
        task_graph_builder: T,
        allowed_origin: impl Into<String>,
    ) -> Self {
        Self {
            repo_root,
            port,
            task_graph_builder,
            auth_token: generate_auth_token(),
            allowed_origin: allowed_origin.into(),
        }
    }

    /// Returns the port the server will listen on
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Returns the per-session token clients must present when connecting
    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    /// Run the server until shutdown
    pub async fn run(self) -> Result<(), ServerError> {
        // Build initial graph state
        let initial_state = build_graph_state(&self.repo_root, &self.task_graph_builder).await?;
        let graph_state = Arc::new(RwLock::new(initial_state));
        let (update_tx, _) = broadcast::channel::<()>(16);

        // Start file watcher
        let watcher = DevtoolsWatcher::new(self.repo_root.clone())?;
        let mut watch_rx = watcher.subscribe();

        // Spawn task to handle file changes and rebuild graph
        let graph_state_clone = graph_state.clone();
        let update_tx_clone = update_tx.clone();
        let repo_root_clone = self.repo_root.clone();
        let task_graph_builder = Arc::new(self.task_graph_builder);
        let task_graph_builder_clone = task_graph_builder.clone();
        tokio::spawn(async move {
            while let Ok(event) = watch_rx.recv().await {
                match event {
                    WatchEvent::FilesChanged => {
                        info!("Files changed, rebuilding graph...");
                        match build_graph_state(&repo_root_clone, task_graph_builder_clone.as_ref())
                            .await
                        {
                            Ok(new_state) => {
                                *graph_state_clone.write().await = new_state;
                                // Notify all connected clients
                                let _ = update_tx_clone.send(());
                                info!("Graph rebuilt successfully");
                            }
                            Err(e) => {
                                warn!("Failed to rebuild graph: {}", e);
                            }
                        }
                    }
                }
            }
            debug!("File watcher task ended");
        });

        // Create app state
        let app_state = AppState {
            graph_state,
            update_tx,
            auth_token: self.auth_token,
            allowed_origin: self.allowed_origin,
        };

        // Build router
        let app = Router::new()
            .route("/", get(ws_handler))
            .with_state(app_state);

        // Bind and serve
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| ServerError::Bind {
                port: self.port,
                source: e,
            })?;

        info!("Devtools server listening on ws://{}", addr);

        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

fn generate_auth_token() -> String {
    let mut rng = rng();
    Alphanumeric.sample_string(&mut rng, AUTH_TOKEN_LENGTH)
}

fn validate_ws_request(
    headers: &HeaderMap,
    query: &AuthQuery,
    auth_token: &str,
    allowed_origin: &str,
) -> Result<(), StatusCode> {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
    else {
        return Err(StatusCode::FORBIDDEN);
    };

    if origin != allowed_origin {
        return Err(StatusCode::FORBIDDEN);
    }

    if query.token.as_deref() != Some(auth_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(())
}

/// WebSocket upgrade handler
async fn ws_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    if let Err(status) =
        validate_ws_request(&headers, &query, &state.auth_token, &state.allowed_origin)
    {
        return status.into_response();
    }

    ws.on_upgrade(|socket| handle_socket(socket, state))
        .into_response()
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Send initial state
    let init_state = state.graph_state.read().await.clone();
    let init_msg = ServerMessage::Init { data: init_state };

    if let Err(e) = sender
        .send(Message::Text(
            serde_json::to_string(&init_msg).unwrap().into(),
        ))
        .await
    {
        error!("Failed to send initial state: {}", e);
        return;
    }

    debug!("Client connected, sent initial state");

    // Subscribe to updates
    let mut update_rx = state.update_tx.subscribe();

    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(_text))) => {
                        // Currently we only expect pong messages, which we can ignore
                        // Future: handle RequestTaskGraph here
                    }
                    Some(Ok(Message::Close(_))) => {
                        debug!("Client disconnected");
                        break;
                    }
                    Some(Ok(Message::Ping(data)))
                        if sender.send(Message::Pong(data.clone())).await.is_err() => {
                            break;
                        }
                    Some(Err(e)) => {
                        // Connection resets without closing handshake are expected when
                        // clients disconnect abruptly (laptop sleep, network drop, etc.)
                        debug!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        debug!("Client connection closed");
                        break;
                    }
                    _ => {}
                }
            }

            // Handle graph updates
            result = update_rx.recv() => {
                if result.is_err() {
                    // Channel closed
                    break;
                }

                let new_state = state.graph_state.read().await.clone();
                let update_msg = ServerMessage::Update { data: new_state };

                if let Err(e) = sender
                    .send(Message::Text(
                        serde_json::to_string(&update_msg).unwrap().into(),
                    ))
                    .await
                {
                    warn!("Failed to send update: {}", e);
                    break;
                }

                debug!("Sent graph update to client");
            }
        }
    }
}

/// Build the current graph state from the repository
async fn build_graph_state(
    repo_root: &AbsoluteSystemPathBuf,
    task_graph_builder: &dyn TaskGraphBuilder,
) -> Result<GraphState, ServerError> {
    // Load root package.json
    let root_package_json_path = repo_root.join_component("package.json");
    let root_package_json = PackageJson::load(&root_package_json_path)
        .map_err(|e| ServerError::PackageJson(e.to_string()))?;

    // Build package graph using local discovery (no daemon)
    // We use allow_no_package_manager to be more permissive about package manager
    // detection
    let pkg_graph = PackageGraphBuilder::new(repo_root, root_package_json)
        .with_allow_no_package_manager(true)
        .build()
        .await
        .map_err(|e| ServerError::PackageGraph(e.to_string()))?;

    // Convert package graph to serializable format
    let package_graph = package_graph_to_data(&pkg_graph);

    // Build task graph using the provided builder (which uses proper turbo run
    // logic)
    let task_graph = task_graph_builder
        .build_task_graph()
        .await
        .map_err(|e| ServerError::TaskGraph(e.to_string()))?;

    Ok(GraphState {
        package_graph,
        task_graph,
        repo_root: repo_root.to_string(),
        turbo_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const AUTH_TOKEN: &str = "session-token";
    const ALLOWED_ORIGIN: &str = "https://turborepo.dev";

    fn headers_with_origin(origin: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_str(origin).expect("origin should be a valid header value"),
        );
        headers
    }

    fn query_with_token(token: Option<&str>) -> AuthQuery {
        AuthQuery {
            token: token.map(ToString::to_string),
        }
    }

    #[test]
    fn auth_token_is_unguessable_url_safe_value() {
        let token = generate_auth_token();

        assert_eq!(token.len(), AUTH_TOKEN_LENGTH);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn allows_matching_origin_and_token() {
        let headers = headers_with_origin(ALLOWED_ORIGIN);
        let query = query_with_token(Some(AUTH_TOKEN));

        assert_eq!(
            validate_ws_request(&headers, &query, AUTH_TOKEN, ALLOWED_ORIGIN),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_origin() {
        let headers = HeaderMap::new();
        let query = query_with_token(Some(AUTH_TOKEN));

        assert_eq!(
            validate_ws_request(&headers, &query, AUTH_TOKEN, ALLOWED_ORIGIN),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn rejects_wrong_origin() {
        let headers = headers_with_origin("https://example.com");
        let query = query_with_token(Some(AUTH_TOKEN));

        assert_eq!(
            validate_ws_request(&headers, &query, AUTH_TOKEN, ALLOWED_ORIGIN),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn rejects_missing_token() {
        let headers = headers_with_origin(ALLOWED_ORIGIN);
        let query = query_with_token(None);

        assert_eq!(
            validate_ws_request(&headers, &query, AUTH_TOKEN, ALLOWED_ORIGIN),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn rejects_wrong_token() {
        let headers = headers_with_origin(ALLOWED_ORIGIN);
        let query = query_with_token(Some("wrong-token"));

        assert_eq!(
            validate_ws_request(&headers, &query, AUTH_TOKEN, ALLOWED_ORIGIN),
            Err(StatusCode::UNAUTHORIZED)
        );
    }
}
