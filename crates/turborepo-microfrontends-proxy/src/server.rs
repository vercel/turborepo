use std::{
    error::Error,
    net::SocketAddr,
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};

use dashmap::DashMap;
use hyper::server::conn::http1;
use hyper_util::{client::legacy::Client, rt::TokioIo};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast, oneshot},
};
use tracing::{debug, error, info};
use turborepo_microfrontends::Config;

use crate::{
    ProxyError,
    http::HttpClient,
    http_router::Router,
    websocket::{WebSocketContext, WebSocketHandle},
};

pub(crate) const DEFAULT_PROXY_PORT: u16 = 3024;
pub(crate) const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(1);
pub(crate) const HTTP_CLIENT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
pub(crate) const HTTP_CLIENT_MAX_IDLE_PER_HOST: usize = 32;
pub(crate) const MAX_CONCURRENT_CONNECTIONS: usize = 512;

fn is_connection_closed_error(err: &hyper::Error) -> bool {
    if err.is_closed() {
        return true;
    }

    if let Some(io_err) = err
        .source()
        .and_then(|e| e.downcast_ref::<std::io::Error>())
    {
        matches!(
            io_err.kind(),
            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
        )
    } else {
        false
    }
}

pub struct ProxyServer {
    config: Arc<Config>,
    router: Arc<Router>,
    port: u16,
    shutdown_tx: broadcast::Sender<()>,
    ws_handles: Arc<DashMap<usize, WebSocketHandle>>,
    ws_id_counter: Arc<AtomicUsize>,
    ws_connection_count: Arc<AtomicUsize>,
    http_client: HttpClient,
    shutdown_complete_tx: Option<oneshot::Sender<()>>,
    connection_semaphore: Arc<Semaphore>,
}

impl ProxyServer {
    pub fn new(config: Config) -> Result<Self, ProxyError> {
        let router = Router::new(&config)
            .map_err(|e| ProxyError::Config(format!("Failed to build router: {e}")))?;

        let port = config.local_proxy_port().unwrap_or(DEFAULT_PROXY_PORT);
        let (shutdown_tx, _) = broadcast::channel(1);

        let http_client = Client::builder(hyper_util::rt::TokioExecutor::new())
            .pool_idle_timeout(HTTP_CLIENT_POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(HTTP_CLIENT_MAX_IDLE_PER_HOST)
            .http2_adaptive_window(true)
            .build_http();

        Ok(Self {
            config: Arc::new(config),
            router: Arc::new(router),
            port,
            shutdown_tx,
            ws_handles: Arc::new(DashMap::new()),
            ws_id_counter: Arc::new(AtomicUsize::new(0)),
            ws_connection_count: Arc::new(AtomicUsize::new(0)),
            http_client,
            shutdown_complete_tx: None,
            connection_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
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
        let connection_semaphore = self.connection_semaphore.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    info!("Received shutdown signal, closing websocket connections...");

                    info!("Closing {} active websocket connection(s)", ws_handles.len());

                    for entry in ws_handles.iter() {
                        let _ = entry.value().shutdown_tx.send(());
                    }

                    tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;

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
                    let ws_handles_clone = ws_handles.clone();
                    let ws_id_counter_clone = self.ws_id_counter.clone();
                    let ws_connection_count_clone = self.ws_connection_count.clone();
                    let http_client = self.http_client.clone();
                    let semaphore = connection_semaphore.clone();

                    tokio::task::spawn(async move {
                        let _permit = semaphore.acquire().await.ok()?;

                        debug!("New connection from {}", remote_addr);

                        let service = hyper::service::service_fn(move |req| {
                            let router = router.clone();
                            let ws_ctx = WebSocketContext {
                                handles: ws_handles_clone.clone(),
                                id_counter: ws_id_counter_clone.clone(),
                                connection_count: ws_connection_count_clone.clone(),
                            };
                            let http_client = http_client.clone();
                            async move {
                                crate::proxy::handle_request(req, router, remote_addr, ws_ctx, http_client).await
                            }
                        });

                        let conn = http1::Builder::new()
                            .serve_connection(io, service)
                            .with_upgrades();

                        match conn.await {
                            Ok(()) => {
                                debug!("Connection from {} closed successfully", remote_addr);
                            }
                            Err(err) => {
                                if err.is_incomplete_message() {
                                    debug!(
                                        "IncompleteMessage error on connection from {}: {:?}. \
                                        This may indicate the client closed the connection before receiving the full response.",
                                        remote_addr, err
                                    );
                                } else if is_connection_closed_error(&err) {
                                    debug!("Connection from {} closed by client: {:?}", remote_addr, err);
                                } else {
                                    error!("Error serving connection from {}: {:?}", remote_addr, err);
                                }
                            }
                        }
                        Some(())
                    });
                }
            }
        }
    }

    fn print_routes(&self) {
        info!("Route configuration:");

        for app in self.config.applications() {
            let app_name = app.application_name;
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

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use tokio::{net::TcpListener, sync::oneshot};
    use turborepo_microfrontends::Config;

    use super::*;
    use crate::websocket::WEBSOCKET_CLOSE_DELAY;

    fn create_test_config() -> Config {
        let config_json = format!(
            r#"{{
            "options": {{
                "localProxyPort": {DEFAULT_PROXY_PORT}
            }},
            "applications": {{
                "web": {{
                    "development": {{
                        "local": {{ "port": 3000 }}
                    }}
                }},
                "docs": {{
                    "development": {{
                        "local": {{ "port": 3001 }}
                    }},
                    "routing": [
                        {{ "paths": ["/docs", "/docs/:path*"] }}
                    ]
                }}
            }}
        }}"#
        );
        Config::from_str(&config_json, "test.json").unwrap()
    }

    #[test]
    fn test_proxy_server_new() {
        let config = create_test_config();
        let result = ProxyServer::new(config);
        assert!(result.is_ok());

        let server = result.unwrap();
        assert_eq!(server.port, DEFAULT_PROXY_PORT);
    }

    #[test]
    fn test_proxy_server_new_with_default_port() {
        let config_json = r#"{
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
        assert_eq!(server.port, DEFAULT_PROXY_PORT);
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

    #[tokio::test]
    async fn test_proxy_server_with_invalid_config() {
        let config_json = r#"{
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

        let result1 = tokio::time::timeout(WEBSOCKET_CLOSE_DELAY, rx1.recv()).await;

        let result2 = tokio::time::timeout(WEBSOCKET_CLOSE_DELAY, rx2.recv()).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());
    }

    #[test]
    fn test_remote_addr_creation() {
        let addr = SocketAddr::from(([127, 0, 0, 1], DEFAULT_PROXY_PORT));
        assert_eq!(addr.port(), DEFAULT_PROXY_PORT);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn test_socket_addr_v4_creation() {
        let addr = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), DEFAULT_PROXY_PORT);
        assert_eq!(addr.port(), DEFAULT_PROXY_PORT);
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
    async fn test_oneshot_channel_communication() {
        let (tx, rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let _ = tx.send(());
        });

        let result = tokio::time::timeout(WEBSOCKET_CLOSE_DELAY, rx).await;

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
