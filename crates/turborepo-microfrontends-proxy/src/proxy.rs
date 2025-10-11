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
    config: Arc<Config>,
    router: Arc<Router>,
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
            config: Arc::new(config),
            router: Arc::new(router),
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

fn is_websocket_upgrade<B>(req: &Request<B>) -> bool {
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
    router: Arc<Router>,
    _config: Arc<Config>,
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

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use hyper::{Method, header::HeaderValue};

    use super::*;

    fn create_test_config() -> Config {
        let config_json = r#"{
            "version": "1",
            "options": {
                "localProxyPort": 3024
            },
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                },
                "docs": {
                    "development": {
                        "local": { "port": 3001 }
                    },
                    "routing": [
                        { "paths": ["/docs", "/docs/:path*"] }
                    ]
                }
            }
        }"#;
        Config::from_str(config_json, "test.json").unwrap()
    }

    #[test]
    fn test_proxy_server_new() {
        let config = create_test_config();
        let result = ProxyServer::new(config);
        assert!(result.is_ok());

        let server = result.unwrap();
        assert_eq!(server.port, 3024);
    }

    #[test]
    fn test_proxy_server_new_with_default_port() {
        let config_json = r#"{
            "version": "1",
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                }
            }
        }"#;
        let config = Config::from_str(config_json, "test.json").unwrap();
        let result = ProxyServer::new(config);
        assert!(result.is_ok());

        let server = result.unwrap();
        assert_eq!(server.port, 3024);
    }

    #[test]
    fn test_proxy_server_shutdown_handle() {
        let config = create_test_config();
        let server = ProxyServer::new(config).unwrap();

        let handle = server.shutdown_handle();
        let _rx = handle.subscribe();
        assert_eq!(handle.receiver_count(), 1);
    }

    #[test]
    fn test_proxy_server_set_shutdown_complete_tx() {
        let config = create_test_config();
        let mut server = ProxyServer::new(config).unwrap();

        let (tx, _rx) = oneshot::channel();
        server.set_shutdown_complete_tx(tx);
        assert!(server.shutdown_complete_tx.is_some());
    }

    #[tokio::test]
    async fn test_check_port_available_when_free() {
        let config_json = r#"{
            "version": "1",
            "options": {
                "localProxyPort": 19999
            },
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                }
            }
        }"#;
        let config = Config::from_str(config_json, "test.json").unwrap();
        let server = ProxyServer::new(config).unwrap();

        let available = server.check_port_available().await;
        assert!(available);
    }

    #[tokio::test]
    async fn test_check_port_available_when_taken() {
        let config_json = r#"{
            "version": "1",
            "options": {
                "localProxyPort": 19998
            },
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                }
            }
        }"#;
        let config = Config::from_str(config_json, "test.json").unwrap();
        let server = ProxyServer::new(config).unwrap();

        let _listener = TcpListener::bind("127.0.0.1:19998").await.unwrap();

        let available = server.check_port_available().await;
        assert!(!available);
    }

    #[test]
    fn test_is_websocket_upgrade_valid() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "Upgrade")
            .body(())
            .unwrap();

        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_case_insensitive() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "WebSocket")
            .header(CONNECTION, "upgrade")
            .body(())
            .unwrap();

        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_with_multiple_connection_values() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "keep-alive, Upgrade")
            .body(())
            .unwrap();

        assert!(is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_missing_upgrade_header() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(CONNECTION, "Upgrade")
            .body(())
            .unwrap();

        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_missing_connection_header() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "websocket")
            .body(())
            .unwrap();

        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_wrong_upgrade_value() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "h2c")
            .header(CONNECTION, "Upgrade")
            .body(())
            .unwrap();

        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_wrong_connection_value() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "close")
            .body(())
            .unwrap();

        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_is_websocket_upgrade_no_headers() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://localhost:3000/ws")
            .body(())
            .unwrap();

        assert!(!is_websocket_upgrade(&req));
    }

    #[test]
    fn test_websocket_handle_creation() {
        let (tx, _rx) = broadcast::channel(1);
        let handle = WebSocketHandle {
            id: 42,
            shutdown_tx: tx,
        };

        assert_eq!(handle.id, 42);
    }

    #[test]
    fn test_websocket_handle_clone() {
        let (tx, _rx) = broadcast::channel(1);
        let handle = WebSocketHandle {
            id: 42,
            shutdown_tx: tx,
        };

        let cloned = handle.clone();
        assert_eq!(cloned.id, 42);
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
        let ws_handles: Arc<Mutex<Vec<WebSocketHandle>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, _rx) = broadcast::channel(1);

        {
            let mut handles = ws_handles.lock().await;
            handles.push(WebSocketHandle {
                id: 1,
                shutdown_tx: tx.clone(),
            });
            handles.push(WebSocketHandle {
                id: 2,
                shutdown_tx: tx.clone(),
            });
        }

        {
            let handles = ws_handles.lock().await;
            assert_eq!(handles.len(), 2);
        }

        {
            let mut handles = ws_handles.lock().await;
            handles.retain(|h| h.id != 1);
        }

        {
            let handles = ws_handles.lock().await;
            assert_eq!(handles.len(), 1);
            assert_eq!(handles[0].id, 2);
        }
    }

    #[tokio::test]
    async fn test_max_websocket_connections() {
        assert_eq!(MAX_WEBSOCKET_CONNECTIONS, 1000);

        let ws_handles: Arc<Mutex<Vec<WebSocketHandle>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, _rx) = broadcast::channel(1);

        {
            let mut handles = ws_handles.lock().await;
            for i in 0..MAX_WEBSOCKET_CONNECTIONS {
                handles.push(WebSocketHandle {
                    id: i,
                    shutdown_tx: tx.clone(),
                });
            }
        }

        let handles = ws_handles.lock().await;
        assert_eq!(handles.len(), MAX_WEBSOCKET_CONNECTIONS);
    }

    #[test]
    fn test_proxy_error_bind_error_display() {
        let error = ProxyError::BindError {
            port: 3024,
            source: std::io::Error::new(std::io::ErrorKind::AddrInUse, "address in use"),
        };

        let error_string = error.to_string();
        assert!(error_string.contains("3024"));
    }

    #[test]
    fn test_proxy_error_config_display() {
        let error = ProxyError::Config("Invalid configuration".to_string());
        assert_eq!(
            error.to_string(),
            "Configuration error: Invalid configuration"
        );
    }

    #[test]
    fn test_proxy_error_app_unreachable_display() {
        let error = ProxyError::AppUnreachable {
            app: "web".to_string(),
            port: 3000,
        };

        let error_string = error.to_string();
        assert!(error_string.contains("web"));
        assert!(error_string.contains("3000"));
    }

    #[test]
    fn test_boxed_body_type() {
        let body = Full::new(Bytes::from("test"))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            .boxed();

        assert_eq!(
            std::mem::size_of_val(&body),
            std::mem::size_of::<BoxedBody>()
        );
    }

    #[tokio::test]
    async fn test_proxy_server_with_invalid_config() {
        let config_json = r#"{
            "version": "1",
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    },
                    "routing": [
                        { "paths": ["/web/:path*"] }
                    ]
                }
            }
        }"#;

        let config = Config::from_str(config_json, "test.json").unwrap();
        let result = ProxyServer::new(config);

        assert!(result.is_err());
        if let Err(err) = result {
            assert!(matches!(err, ProxyError::Config(_)));
        }
    }

    #[tokio::test]
    async fn test_shutdown_signal_broadcasting() {
        let config = create_test_config();
        let server = ProxyServer::new(config).unwrap();

        let shutdown_tx = server.shutdown_handle();
        let mut rx1 = shutdown_tx.subscribe();
        let mut rx2 = shutdown_tx.subscribe();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = shutdown_tx.send(());
        });

        let result1 =
            tokio::time::timeout(tokio::time::Duration::from_millis(100), rx1.recv()).await;

        let result2 =
            tokio::time::timeout(tokio::time::Duration::from_millis(100), rx2.recv()).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());
    }

    #[test]
    fn test_remote_addr_creation() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 3024));
        assert_eq!(addr.port(), 3024);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn test_socket_addr_v4_creation() {
        let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 3024);
        assert_eq!(addr.port(), 3024);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[tokio::test]
    async fn test_http_client_creation() {
        let config = create_test_config();
        let server = ProxyServer::new(config).unwrap();

        let client = &server.http_client;
        assert_eq!(
            std::mem::size_of_val(client),
            std::mem::size_of::<HttpClient>()
        );
    }

    #[test]
    fn test_multiple_proxy_servers() {
        let config1_json = r#"{
            "version": "1",
            "options": { "localProxyPort": 4001 },
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                }
            }
        }"#;

        let config2_json = r#"{
            "version": "1",
            "options": { "localProxyPort": 4002 },
            "applications": {
                "web": {
                    "development": {
                        "local": { "port": 3000 }
                    }
                }
            }
        }"#;

        let config1 = Config::from_str(config1_json, "test1.json").unwrap();
        let config2 = Config::from_str(config2_json, "test2.json").unwrap();

        let server1 = ProxyServer::new(config1);
        let server2 = ProxyServer::new(config2);

        assert!(server1.is_ok());
        assert!(server2.is_ok());

        assert_eq!(server1.unwrap().port, 4001);
        assert_eq!(server2.unwrap().port, 4002);
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

    #[tokio::test]
    async fn test_websocket_handle_shutdown_signal() {
        let (tx, mut rx) = broadcast::channel(1);
        let _handle = WebSocketHandle {
            id: 1,
            shutdown_tx: tx.clone(),
        };

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx.recv()).await;

        assert!(result.is_ok());
    }

    #[test]
    fn test_header_value_creation() {
        let host = HeaderValue::from_str("localhost:3000");
        assert!(host.is_ok());

        let forwarded_for = HeaderValue::from_str("127.0.0.1");
        assert!(forwarded_for.is_ok());

        let forwarded_proto = HeaderValue::from_str("http");
        assert!(forwarded_proto.is_ok());
    }

    #[test]
    fn test_uri_construction() {
        let target_uri = format!("http://localhost:{}{}", 3000, "/api/test");
        assert_eq!(target_uri, "http://localhost:3000/api/test");

        let parsed = target_uri.parse::<hyper::Uri>();
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_uri_with_query_params() {
        let target_uri = format!("http://localhost:{}{}", 3000, "/api/test?foo=bar&baz=qux");
        assert_eq!(target_uri, "http://localhost:3000/api/test?foo=bar&baz=qux");

        let parsed = target_uri.parse::<hyper::Uri>();
        assert!(parsed.is_ok());

        let uri = parsed.unwrap();
        assert_eq!(uri.path(), "/api/test");
        assert_eq!(uri.query(), Some("foo=bar&baz=qux"));
    }

    #[tokio::test]
    async fn test_oneshot_channel_communication() {
        let (tx, rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = tokio::time::timeout(tokio::time::Duration::from_millis(100), rx).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_broadcast_channel_multiple_receivers() {
        let (tx, _rx) = broadcast::channel::<()>(10);

        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();
        let mut rx3 = tx.subscribe();

        assert_eq!(tx.receiver_count(), 4);

        tokio::spawn(async move {
            let _ = tx.send(());
        });

        let result1 = rx1.recv().await;
        let result2 = rx2.recv().await;
        let result3 = rx3.recv().await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());
        assert!(result3.is_ok());
    }
}
