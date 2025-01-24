//! Web UI for Turborepo. Creates a WebSocket server that can be subscribed to
//! by a web client to display the status of tasks.

pub mod event;
pub mod query;
pub mod sender;
pub mod subscriber;

use event::WebUIEvent;
pub use query::RunQuery;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to start server.")]
    Server(#[from] std::io::Error),
    #[error("Failed to start websocket server: {0}")]
    WebSocket(#[source] axum::Error),
    #[error("Failed to serialize message: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Failed to send message.")]
    Send(#[from] axum::Error),
    #[error("Failed to send message through channel.")]
    Broadcast(#[from] tokio::sync::mpsc::error::SendError<WebUIEvent>),
}
