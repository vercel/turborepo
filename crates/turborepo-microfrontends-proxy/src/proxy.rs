use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{CONNECTION, UPGRADE},
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
};
use hyper_util::{client::legacy::Client, rt::TokioIo};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast, oneshot},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};
use tracing::{debug, error, info, warn};
use turborepo_microfrontends::Config;

use crate::{
    error::{ErrorPage, ProxyError},
    router::Router,
};

type BoxedBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;
type HttpClient = Client<hyper_util::client::legacy::connect::HttpConnector, Incoming>;

const MAX_WEBSOCKET_CONNECTIONS: usize = 1000;

#[derive(Clone)]
struct WebSocketHandle {
    id: usize,
    shutdown_tx: broadcast::Sender<()>,
}

pub struct ProxyServer {
    config: Config,
    router: Router,
    port: u16,
    shutdown_tx: broadcast::Sender<()>,
    ws_handles: Arc<Mutex<Vec<WebSocketHandle>>>,
    ws_id_counter: Arc<AtomicUsize>,
    http_client: HttpClient,
    shutdown_complete_tx: Option<oneshot::Sender<()>>,
}

impl ProxyServer {
    pub fn new(config: Config) -> Result<Self, ProxyError> {
        let router = Router::new(&config)
            .map_err(|e| ProxyError::Config(format!("Failed to build router: {}", e)))?;

        let port = config.local_proxy_port().unwrap_or(3024);
        let (shutdown_tx, _) = broadcast::channel(1);

        let http_client = Client::builder(hyper_util::rt::TokioExecutor::new()).build_http();

        Ok(Self {
            config,
            router,
            port,
            shutdown_tx,
            ws_handles: Arc::new(Mutex::new(Vec::new())),
            ws_id_counter: Arc::new(AtomicUsize::new(0)),
            http_client,
            shutdown_complete_tx: None,
        })
    }

    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    pub fn set_shutdown_complete_tx(&mut self, tx: oneshot::Sender<()>) {
        self.shutdown_complete_tx = Some(tx);
    }

    pub async fn check_port_available(&self) -> bool {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        TcpListener::bind(addr).await.is_ok()
    }

    pub async fn run(self) -> Result<(), ProxyError> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| ProxyError::BindError {
                port: self.port,
                source: e,
            })?;

        info!(
            "Turborepo microfrontends proxy listening on http://{}",
            addr
        );
        self.print_routes();

        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let ws_handles = self.ws_handles.clone();
        let shutdown_complete_tx = self.shutdown_complete_tx;

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal, closing websocket connections...");

                    let handles = ws_handles.lock().await;
                    info!("Closing {} active websocket connection(s)", handles.len());

                    for handle in handles.iter() {
                        let _ = handle.shutdown_tx.send(());
                    }

                    drop(handles);

                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                    info!("Turborepo microfrontends proxy shut down");

                    if let Some(tx) = shutdown_complete_tx {
                        let _ = tx.send(());
                    }

                    return Ok(());
                }
                result = listener.accept() => {
                    let (stream, remote_addr) = result?;
                    let io = TokioIo::new(stream);

                    let router = self.router.clone();
                    let config = self.config.clone();
                    let ws_handles_clone = ws_handles.clone();
                    let ws_id_counter_clone = self.ws_id_counter.clone();
                    let http_client = self.http_client.clone();

                    tokio::task::spawn(async move {
                        debug!("New connection from {}", remote_addr);

                        let service = service_fn(move |req| {
                            let router = router.clone();
                            let config = config.clone();
                            let ws_handles = ws_handles_clone.clone();
                            let ws_id_counter = ws_id_counter_clone.clone();
                            let http_client = http_client.clone();
                            async move { handle_request(req, router, config, remote_addr, ws_handles, ws_id_counter, http_client).await }
                        });

                        let conn = http1::Builder::new()
                            .serve_connection(io, service)
                            .with_upgrades();

                        match conn.await {
                            Ok(()) => {
                                debug!("Connection from {} closed successfully", remote_addr);
                            }
                            Err(err) => {
                                let err_str = err.to_string();
                                if err_str.contains("IncompleteMessage") {
                                    error!(
                                        "IncompleteMessage error on connection from {}: {:?}. \
                                        This may indicate the client closed the connection before receiving the full response.",
                                        remote_addr, err
                                    );
                                } else if err_str.contains("connection closed") || err_str.contains("broken pipe") {
                                    debug!("Connection from {} closed by client: {:?}", remote_addr, err);
                                } else {
                                    error!("Error serving connection from {}: {:?}", remote_addr, err);
                                }
                            }
                        }
                    });
                }
            }
        }
    }

    fn print_routes(&self) {
        info!("Route configuration:");

        for task in self.config.development_tasks() {
            let app_name = task.application_name;
            if let Some(port) = self.config.port(app_name) {
                if let Some(routing) = self.config.routing(app_name) {
                    for path_group in routing {
                        for path in &path_group.paths {
                            info!("  {} → http://localhost:{}", path, port);
                        }
                    }
                } else {
                    info!("  * (default) → http://localhost:{}", port);
                }
            }
        }
    }
}

fn is_websocket_upgrade(req: &Request<Incoming>) -> bool {
    req.headers()
        .get(UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
        && req
            .headers()
            .get(CONNECTION)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(',')
                    .any(|s| s.trim().eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
}

async fn handle_request(
    mut req: Request<Incoming>,
    router: Router,
    _config: Config,
    remote_addr: SocketAddr,
    ws_handles: Arc<Mutex<Vec<WebSocketHandle>>>,
    ws_id_counter: Arc<AtomicUsize>,
    http_client: HttpClient,
) -> Result<Response<BoxedBody>, ProxyError> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();

    debug!("Request: {} {} from {}", method, path, remote_addr.ip());

    let route_match = router.match_route(&path);
    debug!(
        "Matched route: app={}, port={}",
        route_match.app_name, route_match.port
    );

    if is_websocket_upgrade(&req) {
        debug!("WebSocket upgrade request detected");

        let req_upgrade = hyper::upgrade::on(&mut req);

        match forward_websocket(
            req,
            &route_match.app_name,
            route_match.port,
            remote_addr,
            req_upgrade,
            ws_handles,
            ws_id_counter,
            http_client,
        )
        .await
        {
            Ok(response) => {
                let status = response.status();
                debug!(
                    "Forwarding WebSocket response from {} with status {} to client {}",
                    route_match.app_name,
                    status,
                    remote_addr.ip()
                );
                let (parts, body) = response.into_parts();
                let app_name = route_match.app_name.clone();
                let boxed_body = body
                    .map_err(move |e| {
                        error!(
                            "Error reading body from WebSocket upgrade {}: {}",
                            app_name, e
                        );
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    })
                    .boxed();
                Ok(Response::from_parts(parts, boxed_body))
            }
            Err(e) => {
                warn!(
                    "Failed to establish WebSocket connection to {}: {}",
                    route_match.app_name, e
                );

                let error_page = ErrorPage::new(
                    path,
                    route_match.app_name.clone(),
                    route_match.port,
                    e.to_string(),
                );

                let html = error_page.to_html();
                let response = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(
                        Full::new(Bytes::from(html))
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                            .boxed(),
                    )
                    .map_err(ProxyError::Http)?;

                Ok(response)
            }
        }
    } else {
        match forward_request(
            req,
            &route_match.app_name,
            route_match.port,
            remote_addr,
            http_client,
        )
        .await
        {
            Ok(response) => {
                let status = response.status();
                let (parts, body) = response.into_parts();
                debug!(
                    "Forwarding response from {} with status {} to client {}",
                    route_match.app_name,
                    status,
                    remote_addr.ip()
                );
                let app_name = route_match.app_name.clone();
                let boxed_body = body
                    .map_err(move |e| {
                        error!("Error reading body from upstream {}: {}", app_name, e);
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync>
                    })
                    .boxed();
                Ok(Response::from_parts(parts, boxed_body))
            }
            Err(e) => {
                warn!(
                    "Failed to forward request to {}: {}",
                    route_match.app_name, e
                );

                let error_page = ErrorPage::new(
                    path,
                    route_match.app_name.clone(),
                    route_match.port,
                    e.to_string(),
                );

                let html = error_page.to_html();
                let response = Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(
                        Full::new(Bytes::from(html))
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                            .boxed(),
                    )
                    .map_err(ProxyError::Http)?;

                Ok(response)
            }
        }
    }
}

async fn forward_websocket(
    mut req: Request<Incoming>,
    app_name: &str,
    port: u16,
    remote_addr: SocketAddr,
    client_upgrade: hyper::upgrade::OnUpgrade,
    ws_handles: Arc<Mutex<Vec<WebSocketHandle>>>,
    ws_id_counter: Arc<AtomicUsize>,
    http_client: HttpClient,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let target_uri = format!(
        "http://localhost:{}{}",
        port,
        req.uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
    );

    let original_host = req.uri().host().unwrap_or("localhost").to_string();

    let headers = req.headers_mut();
    headers.insert("Host", format!("localhost:{}", port).parse()?);
    headers.insert("X-Forwarded-For", remote_addr.ip().to_string().parse()?);
    headers.insert("X-Forwarded-Proto", "http".parse()?);
    headers.insert("X-Forwarded-Host", original_host.parse()?);

    *req.uri_mut() = target_uri.parse()?;

    let mut response = http_client.request(req).await?;

    debug!(
        "WebSocket upgrade response from {}: {}",
        app_name,
        response.status()
    );

    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let server_upgrade = hyper::upgrade::on(&mut response);
        let app_name_clone = app_name.to_string();

        let (ws_shutdown_tx, _) = broadcast::channel(1);
        let ws_id = {
            let mut handles = ws_handles.lock().await;
            if handles.len() >= MAX_WEBSOCKET_CONNECTIONS {
                warn!(
                    "WebSocket connection limit reached ({} connections), rejecting new \
                     connection from {}",
                    MAX_WEBSOCKET_CONNECTIONS, remote_addr
                );
                return Err("WebSocket connection limit reached".into());
            }

            let id = ws_id_counter.fetch_add(1, Ordering::SeqCst);
            handles.push(WebSocketHandle {
                id,
                shutdown_tx: ws_shutdown_tx.clone(),
            });
            id
        };

        tokio::spawn(async move {
            let client_result = client_upgrade.await;
            let server_result = server_upgrade.await;

            match (client_result, server_result) {
                (Ok(client_upgraded), Ok(server_upgraded)) => {
                    debug!("Both WebSocket upgrades successful for {}", app_name_clone);
                    if let Err(e) = proxy_websocket_connection(
                        client_upgraded,
                        server_upgraded,
                        app_name_clone,
                        ws_shutdown_tx,
                        ws_handles.clone(),
                        ws_id,
                    )
                    .await
                    {
                        error!("WebSocket proxy error: {}", e);
                    }
                }
                (Err(e), _) => {
                    error!("Failed to upgrade client WebSocket connection: {}", e);
                    let mut handles = ws_handles.lock().await;
                    handles.retain(|h| h.id != ws_id);
                }
                (_, Err(e)) => {
                    error!("Failed to upgrade server WebSocket connection: {}", e);
                    let mut handles = ws_handles.lock().await;
                    handles.retain(|h| h.id != ws_id);
                }
            }
        });
    }

    Ok(response)
}

async fn proxy_websocket_connection(
    client_upgraded: Upgraded,
    server_upgraded: Upgraded,
    app_name: String,
    ws_shutdown_tx: broadcast::Sender<()>,
    ws_handles: Arc<Mutex<Vec<WebSocketHandle>>>,
    ws_id: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

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
                info!("Received shutdown signal for websocket connection to {}", app_name);
                debug!("Sending close frames to client and server for {}", app_name);
                // Send close frames to both sides
                if let Err(e) = client_sink.send(Message::Close(None)).await {
                    warn!("Failed to send close frame to client for {}: {}", app_name, e);
                }
                if let Err(e) = server_sink.send(Message::Close(None)).await {
                    warn!("Failed to send close frame to server for {}: {}", app_name, e);
                }
                let _ = client_sink.flush().await;
                let _ = server_sink.flush().await;
                debug!("Close frames sent and flushed for {}", app_name);

                // Give a moment for the close handshake to complete
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                let _ = client_sink.close().await;
                let _ = server_sink.close().await;
                info!("Websocket connection to {} closed gracefully", app_name);
                break;
            }
            client_msg = client_stream.next() => {
                match client_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            debug!("Client sent close frame");
                            let _ = server_sink.send(msg).await;
                            let _ = server_sink.close().await;
                            break;
                        }
                        if let Err(e) = server_sink.send(msg).await {
                            error!("Error forwarding client -> server: {}", e);
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        error!("Error reading from client: {}", e);
                        break;
                    }
                    None => {
                        debug!("Client stream ended");
                        break;
                    }
                }
            }
            server_msg = server_stream.next() => {
                match server_msg {
                    Some(Ok(msg)) => {
                        if msg.is_close() {
                            debug!("Server sent close frame");
                            let _ = client_sink.send(msg).await;
                            let _ = client_sink.close().await;
                            break;
                        }
                        if let Err(e) = client_sink.send(msg).await {
                            error!("Error forwarding server -> client: {}", e);
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        error!("Error reading from server: {}", e);
                        break;
                    }
                    None => {
                        debug!("Server stream ended");
                        break;
                    }
                }
            }
        }
    }

    let mut handles = ws_handles.lock().await;
    handles.retain(|h| h.id != ws_id);
    debug!(
        "WebSocket connection closed for {} (id: {})",
        app_name, ws_id
    );

    Ok(())
}

async fn forward_request(
    mut req: Request<Incoming>,
    app_name: &str,
    port: u16,
    remote_addr: SocketAddr,
    http_client: HttpClient,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let target_uri = format!(
        "http://localhost:{}{}",
        port,
        req.uri()
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
    );

    let original_host = req.uri().host().unwrap_or("localhost").to_string();

    let headers = req.headers_mut();
    headers.insert("Host", format!("localhost:{}", port).parse()?);
    headers.insert("X-Forwarded-For", remote_addr.ip().to_string().parse()?);
    headers.insert("X-Forwarded-Proto", "http".parse()?);
    headers.insert("X-Forwarded-Host", original_host.parse()?);

    *req.uri_mut() = target_uri.parse()?;

    let response = http_client.request(req).await?;

    debug!("Response from {}: {}", app_name, response.status());

    Ok(response)
}
