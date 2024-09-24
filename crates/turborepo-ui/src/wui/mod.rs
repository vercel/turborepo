//! Web UI for Turborepo. Creates a WebSocket server that can be subscribed to
//! by a web client to display the status of tasks.

pub mod event;
pub mod sender;
pub mod server;
pub mod subscriber;

use event::WebUIEvent;
pub use server::RunQuery;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to start server")]
    Server(#[from] std::io::Error),
    #[error("failed to start websocket server: {0}")]
    WebSocket(#[source] axum::Error),
    #[error("failed to serialize message: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("failed to send message")]
    Send(#[from] axum::Error),
    #[error("failed to send message through channel")]
    Broadcast(#[from] tokio::sync::mpsc::error::SendError<WebUIEvent>),
}
