use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use dashmap::DashMap;
use hyper::{Request, Response, StatusCode, body::Incoming, upgrade::Upgraded};
use hyper_util::rt::TokioIo;
use tokio::sync::broadcast;
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};
use tracing::{debug, error, info, warn};

use crate::{
    ProxyError,
    headers::validate_host_header,
    http::{BoxedBody, HttpClient, handle_forward_result},
    http_router::RouteMatch,
};

pub(crate) const MAX_WEBSOCKET_CONNECTIONS: usize = 1000;
pub(crate) const WEBSOCKET_CLOSE_DELAY: Duration = Duration::from_millis(100);
pub(crate) const WEBSOCKET_SHUTDOWN_CHANNEL_CAPACITY: usize = 1;

#[derive(Clone)]
pub(crate) struct WebSocketHandle {
    pub(crate) shutdown_tx: broadcast::Sender<()>,
}

pub(crate) struct WebSocketContext {
    pub(crate) handles: Arc<DashMap<usize, WebSocketHandle>>,
    pub(crate) id_counter: Arc<AtomicUsize>,
    pub(crate) connection_count: Arc<AtomicUsize>,
}

pub(crate) async fn handle_websocket_request(
    req: Request<Incoming>,
    route_match: RouteMatch,
    path: String,
    remote_addr: SocketAddr,
    req_upgrade: hyper::upgrade::OnUpgrade,
    ws_ctx: WebSocketContext,
    http_client: HttpClient,
) -> Result<Response<BoxedBody>, ProxyError> {
    let result = forward_websocket(
        req,
        route_match.app_name.clone(),
        route_match.port,
        remote_addr,
        req_upgrade,
        ws_ctx,
        http_client.clone(),
    )
    .await;

    handle_forward_result(
        result,
        path,
        route_match,
        remote_addr,
        http_client,
        "WebSocket",
    )
    .await
}

async fn forward_websocket(
    mut req: Request<Incoming>,
    app_name: Arc<str>,
    port: u16,
    remote_addr: SocketAddr,
    client_upgrade: hyper::upgrade::OnUpgrade,
    ws_ctx: WebSocketContext,
    http_client: HttpClient,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    prepare_websocket_request(&mut req, port, remote_addr)?;

    let mut response = http_client.request(req).await?;

    debug!(
        "WebSocket upgrade response from {}: {}",
        app_name,
        response.status()
    );

    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let server_upgrade = hyper::upgrade::on(&mut response);
        spawn_websocket_proxy(
            app_name,
            remote_addr,
            client_upgrade,
            server_upgrade,
            ws_ctx.handles,
            ws_ctx.id_counter,
            ws_ctx.connection_count,
        )?;
    }

    Ok(response)
}

fn prepare_websocket_request(
    req: &mut Request<Incoming>,
    port: u16,
    remote_addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let target_uri = format!(
        "http://localhost:{}{}",
        port,
        req.uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
    );

    let original_host = req.uri().host().unwrap_or("localhost").to_string();
    validate_host_header(&original_host)?;

    let headers = req.headers_mut();
    headers.insert("Host", format!("localhost:{port}").parse()?);
    headers.insert("X-Forwarded-For", remote_addr.ip().to_string().parse()?);
    headers.insert("X-Forwarded-Proto", "http".parse()?);
    headers.insert("X-Forwarded-Host", original_host.parse()?);

    *req.uri_mut() = target_uri.parse()?;

    Ok(())
}

fn spawn_websocket_proxy(
    app_name: Arc<str>,
    remote_addr: SocketAddr,
    client_upgrade: hyper::upgrade::OnUpgrade,
    server_upgrade: hyper::upgrade::OnUpgrade,
    ws_handles: Arc<DashMap<usize, WebSocketHandle>>,
    ws_id_counter: Arc<AtomicUsize>,
    connection_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Atomically check and increment the connection count to prevent TOCTOU race
    // condition
    let mut current_count = connection_count.load(Ordering::SeqCst);
    loop {
        if current_count >= MAX_WEBSOCKET_CONNECTIONS {
            warn!(
                "WebSocket connection limit reached ({} connections), rejecting new connection \
                 from {}",
                MAX_WEBSOCKET_CONNECTIONS, remote_addr
            );
            return Err("WebSocket connection limit reached".into());
        }

        // Try to atomically increment from current_count to current_count + 1
        match connection_count.compare_exchange(
            current_count,
            current_count + 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break, // Successfully incremented
            Err(actual_count) => {
                // Another thread changed the count, retry with the actual count
                current_count = actual_count;
            }
        }
    }

    let (ws_shutdown_tx, _) = broadcast::channel(WEBSOCKET_SHUTDOWN_CHANNEL_CAPACITY);
    let ws_id = ws_id_counter.fetch_add(1, Ordering::SeqCst);
    ws_handles.insert(
        ws_id,
        WebSocketHandle {
            shutdown_tx: ws_shutdown_tx.clone(),
        },
    );

    tokio::spawn(async move {
        handle_websocket_upgrades(
            client_upgrade,
            server_upgrade,
            app_name,
            ws_shutdown_tx,
            ws_handles,
            ws_id,
            connection_count,
        )
        .await;
    });

    Ok(())
}

async fn handle_websocket_upgrades(
    client_upgrade: hyper::upgrade::OnUpgrade,
    server_upgrade: hyper::upgrade::OnUpgrade,
    app_name: Arc<str>,
    ws_shutdown_tx: broadcast::Sender<()>,
    ws_handles: Arc<DashMap<usize, WebSocketHandle>>,
    ws_id: usize,
    connection_count: Arc<AtomicUsize>,
) {
    let client_result = client_upgrade.await;
    let server_result = server_upgrade.await;

    match (client_result, server_result) {
        (Ok(client_upgraded), Ok(server_upgraded)) => {
            debug!("Both WebSocket upgrades successful for {}", app_name);
            if let Err(e) = proxy_websocket_connection(
                client_upgraded,
                server_upgraded,
                app_name,
                ws_shutdown_tx,
                ws_handles.clone(),
                ws_id,
                connection_count.clone(),
            )
            .await
            {
                error!("WebSocket proxy error: {}", e);
            }
        }
        (Err(e), _) => {
            error!("Failed to upgrade client WebSocket connection: {}", e);
            ws_handles.remove(&ws_id);
            connection_count.fetch_sub(1, Ordering::SeqCst);
        }
        (_, Err(e)) => {
            error!("Failed to upgrade server WebSocket connection: {}", e);
            ws_handles.remove(&ws_id);
            connection_count.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

async fn proxy_websocket_connection(
    client_upgraded: Upgraded,
    server_upgraded: Upgraded,
    app_name: Arc<str>,
    ws_shutdown_tx: broadcast::Sender<()>,
    ws_handles: Arc<DashMap<usize, WebSocketHandle>>,
    ws_id: usize,
    connection_count: Arc<AtomicUsize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::StreamExt;

    let client_ws =
        WebSocketStream::from_raw_socket(TokioIo::new(client_upgraded), Role::Server, None).await;

    let server_ws =
        WebSocketStream::from_raw_socket(TokioIo::new(server_upgraded), Role::Client, None).await;

    debug!("WebSocket bidirectional proxy established for {}", app_name);

    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut server_sink, mut server_stream) = server_ws.split();

    let mut shutdown_rx = ws_shutdown_tx.subscribe();

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                handle_websocket_shutdown(&mut client_sink, &mut server_sink, &app_name).await;
                break;
            }
            client_msg = client_stream.next() => {
                if !handle_client_message(client_msg, &mut server_sink).await {
                    break;
                }
            }
            server_msg = server_stream.next() => {
                if !handle_server_message(server_msg, &mut client_sink).await {
                    break;
                }
            }
        }
    }

    cleanup_websocket_connection(&ws_handles, ws_id, &app_name, connection_count);

    Ok(())
}

async fn handle_websocket_shutdown<S>(client_sink: &mut S, server_sink: &mut S, app_name: &str)
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    <S as futures_util::Sink<tokio_tungstenite::tungstenite::Message>>::Error: std::fmt::Display,
{
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    info!(
        "Received shutdown signal for websocket connection to {}",
        app_name
    );
    debug!("Sending close frames to client and server for {}", app_name);

    if let Err(e) = client_sink.send(Message::Close(None)).await {
        warn!(
            "Failed to send close frame to client for {}: {}",
            app_name, e
        );
    }
    if let Err(e) = server_sink.send(Message::Close(None)).await {
        warn!(
            "Failed to send close frame to server for {}: {}",
            app_name, e
        );
    }
    let _ = client_sink.flush().await;
    let _ = server_sink.flush().await;
    debug!("Close frames sent and flushed for {}", app_name);

    tokio::time::sleep(WEBSOCKET_CLOSE_DELAY).await;

    let _ = client_sink.close().await;
    let _ = server_sink.close().await;
    info!("Websocket connection to {} closed gracefully", app_name);
}

async fn handle_client_message<S>(
    client_msg: Option<
        Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
    >,
    server_sink: &mut S,
) -> bool
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    <S as futures_util::Sink<tokio_tungstenite::tungstenite::Message>>::Error: std::fmt::Display,
{
    use futures_util::SinkExt;

    match client_msg {
        Some(Ok(msg)) => {
            if msg.is_close() {
                debug!("Client sent close frame");
                let _ = server_sink.send(msg).await;
                let _ = server_sink.close().await;
                return false;
            }
            if let Err(e) = server_sink.send(msg).await {
                error!("Error forwarding client -> server: {}", e);
                return false;
            }
            true
        }
        Some(Err(e)) => {
            error!("Error reading from client: {}", e);
            false
        }
        None => {
            debug!("Client stream ended");
            false
        }
    }
}

async fn handle_server_message<S>(
    server_msg: Option<
        Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
    >,
    client_sink: &mut S,
) -> bool
where
    S: futures_util::Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
    <S as futures_util::Sink<tokio_tungstenite::tungstenite::Message>>::Error: std::fmt::Display,
{
    use futures_util::SinkExt;

    match server_msg {
        Some(Ok(msg)) => {
            if msg.is_close() {
                debug!("Server sent close frame");
                let _ = client_sink.send(msg).await;
                let _ = client_sink.close().await;
                return false;
            }
            if let Err(e) = client_sink.send(msg).await {
                error!("Error forwarding server -> client: {}", e);
                return false;
            }
            true
        }
        Some(Err(e)) => {
            error!("Error reading from server: {}", e);
            false
        }
        None => {
            debug!("Server stream ended");
            false
        }
    }
}

fn cleanup_websocket_connection(
    ws_handles: &Arc<DashMap<usize, WebSocketHandle>>,
    ws_id: usize,
    app_name: &str,
    connection_count: Arc<AtomicUsize>,
) {
    ws_handles.remove(&ws_id);
    connection_count.fetch_sub(1, Ordering::SeqCst);
    debug!(
        "WebSocket connection closed for {} (id: {})",
        app_name, ws_id
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use tokio::sync::broadcast;

    use super::*;

    #[test]
    fn test_websocket_handle_creation() {
        let (tx, _rx) = broadcast::channel(1);
        let _handle = WebSocketHandle { shutdown_tx: tx };
    }

    #[test]
    fn test_websocket_handle_clone() {
        let (tx, _rx) = broadcast::channel(1);
        let handle = WebSocketHandle { shutdown_tx: tx };

        let _cloned = handle.clone();
    }

    #[tokio::test]
    async fn test_websocket_counter_increment() {
        let counter = Arc::new(AtomicUsize::new(0));

        let id1 = counter.fetch_add(1, Ordering::SeqCst);
        let id2 = counter.fetch_add(1, Ordering::SeqCst);
        let id3 = counter.fetch_add(1, Ordering::SeqCst);

        assert_eq!(id1, 0);
        assert_eq!(id2, 1);
        assert_eq!(id3, 2);
    }

    #[tokio::test]
    async fn test_websocket_handles_management() {
        let ws_handles: Arc<DashMap<usize, WebSocketHandle>> = Arc::new(DashMap::new());
        let (tx, _rx) = broadcast::channel(1);

        ws_handles.insert(
            1,
            WebSocketHandle {
                shutdown_tx: tx.clone(),
            },
        );
        ws_handles.insert(
            2,
            WebSocketHandle {
                shutdown_tx: tx.clone(),
            },
        );

        assert_eq!(ws_handles.len(), 2);

        ws_handles.remove(&1);

        assert_eq!(ws_handles.len(), 1);
        assert!(ws_handles.contains_key(&2));
    }

    #[tokio::test]
    async fn test_max_websocket_connections() {
        assert_eq!(MAX_WEBSOCKET_CONNECTIONS, 1000);

        let ws_handles: Arc<DashMap<usize, WebSocketHandle>> = Arc::new(DashMap::new());
        let (tx, _rx) = broadcast::channel(1);

        for i in 0..MAX_WEBSOCKET_CONNECTIONS {
            ws_handles.insert(
                i,
                WebSocketHandle {
                    shutdown_tx: tx.clone(),
                },
            );
        }

        assert_eq!(ws_handles.len(), MAX_WEBSOCKET_CONNECTIONS);
    }

    #[tokio::test]
    async fn test_websocket_handle_shutdown_signal() {
        let (tx, mut rx) = broadcast::channel(1);
        let _handle = WebSocketHandle {
            shutdown_tx: tx.clone(),
        };

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = tokio::time::timeout(WEBSOCKET_CLOSE_DELAY, rx.recv()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ws_id_counter_concurrent_access() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..10 {
            let counter_clone = counter.clone();
            let handle = tokio::spawn(async move { counter_clone.fetch_add(1, Ordering::SeqCst) });
            handles.push(handle);
        }

        let mut ids = vec![];
        for handle in handles {
            ids.push(handle.await.unwrap());
        }

        ids.sort();
        assert_eq!(ids.len(), 10);
        assert_eq!(*ids.first().unwrap(), 0);
        assert_eq!(*ids.last().unwrap(), 9);
    }
}
