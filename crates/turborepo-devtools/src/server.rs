//! WebSocket server for devtools.
//!
//! Provides a WebSocket endpoint that clients can connect to receive
//! real-time graph updates as the repository changes.

use std::sync::Arc;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::Method,
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{broadcast, RwLock},
};
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{package_graph::PackageGraphBuilder, package_json::PackageJson};

use crate::{
    graph::{package_graph_to_data, task_graph_to_data},
    types::{GraphState, ServerMessage},
    watcher::{DevtoolsWatcher, WatchEvent},
};

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
}

/// Shared state for the WebSocket server
#[derive(Clone)]
struct AppState {
    /// Current graph state
    graph_state: Arc<RwLock<GraphState>>,
    /// Channel to notify clients of updates
    update_tx: broadcast::Sender<()>,
}

/// The devtools WebSocket server
pub struct DevtoolsServer {
    repo_root: AbsoluteSystemPathBuf,
    port: u16,
}

impl DevtoolsServer {
    /// Creates a new devtools server for the given repository
    pub fn new(repo_root: AbsoluteSystemPathBuf, port: u16) -> Self {
        Self { repo_root, port }
    }

    /// Returns the port the server will listen on
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Run the server until shutdown
    pub async fn run(self) -> Result<(), ServerError> {
        // Build initial graph state
        let initial_state = build_graph_state(&self.repo_root).await?;
        let graph_state = Arc::new(RwLock::new(initial_state));
        let (update_tx, _) = broadcast::channel::<()>(16);

        // Start file watcher
        let watcher = DevtoolsWatcher::new(self.repo_root.clone())?;
        let mut watch_rx = watcher.subscribe();

        // Spawn task to handle file changes and rebuild graph
        let graph_state_clone = graph_state.clone();
        let update_tx_clone = update_tx.clone();
        let repo_root_clone = self.repo_root.clone();
        tokio::spawn(async move {
            while let Ok(event) = watch_rx.recv().await {
                match event {
                    WatchEvent::FilesChanged => {
                        info!("Files changed, rebuilding graph...");
                        match build_graph_state(&repo_root_clone).await {
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

        // Set up CORS
        let cors = CorsLayer::new()
            .allow_methods([Method::GET])
            .allow_headers(Any)
            .allow_origin(Any);

        // Create app state
        let app_state = AppState {
            graph_state,
            update_tx,
        };

        // Build router
        let app = Router::new()
            .route("/", get(ws_handler))
            .layer(cors)
            .with_state(app_state);

        // Bind and serve
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await.map_err(|e| ServerError::Bind {
            port: self.port,
            source: e,
        })?;

        info!("Devtools server listening on ws://{}", addr);

        axum::serve(listener, app).await?;

        Ok(())
    }
}

/// WebSocket upgrade handler
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Send initial state
    let init_state = state.graph_state.read().await.clone();
    let init_msg = ServerMessage::Init { data: init_state };

    if let Err(e) = sender
        .send(Message::Text(serde_json::to_string(&init_msg).unwrap()))
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
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
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
                    .send(Message::Text(serde_json::to_string(&update_msg).unwrap()))
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
async fn build_graph_state(repo_root: &AbsoluteSystemPathBuf) -> Result<GraphState, ServerError> {
    // Load root package.json
    let root_package_json_path = repo_root.join_component("package.json");
    let root_package_json = PackageJson::load(&root_package_json_path)
        .map_err(|e| ServerError::PackageJson(e.to_string()))?;

    // Build package graph using local discovery (no daemon)
    // We use allow_no_package_manager to be more permissive about package manager detection
    let pkg_graph = PackageGraphBuilder::new(repo_root, root_package_json)
        .with_allow_no_package_manager(true)
        .build()
        .await
        .map_err(|e| ServerError::PackageGraph(e.to_string()))?;

    // Convert to our serializable formats
    let package_graph = package_graph_to_data(&pkg_graph);
    let task_graph = task_graph_to_data(&pkg_graph);

    Ok(GraphState {
        package_graph,
        task_graph,
        repo_root: repo_root.to_string(),
        turbo_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
