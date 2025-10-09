use std::net::SocketAddr;

use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::{CONNECTION, UPGRADE},
    server::conn::http1,
    service::service_fn,
    upgrade::Upgraded,
};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};
use tracing::{debug, error, info, warn};
use turborepo_microfrontends::Config;

use crate::{
    error::{ErrorPage, ProxyError},
    router::Router,
};

type BoxedBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

pub struct ProxyServer {
    config: Config,
    router: Router,
    port: u16,
}

impl ProxyServer {
    pub fn new(config: Config) -> Result<Self, ProxyError> {
        let router = Router::new(&config)
            .map_err(|e| ProxyError::Config(format!("Failed to build router: {}", e)))?;

        let port = config.local_proxy_port().unwrap_or(3024);

        Ok(Self {
            config,
            router,
            port,
        })
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

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let io = TokioIo::new(stream);

            let router = self.router.clone();
            let config = self.config.clone();

            tokio::task::spawn(async move {
                let service = service_fn(move |req| {
                    let router = router.clone();
                    let config = config.clone();
                    async move { handle_request(req, router, config, remote_addr).await }
                });

                let conn = http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades();

                if let Err(err) = conn.await {
                    error!("Error serving connection: {:?}", err);
                }
            });
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
        )
        .await
        {
            Ok(response) => {
                let (parts, body) = response.into_parts();
                let boxed_body = body
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
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
        match forward_request(req, &route_match.app_name, route_match.port, remote_addr).await {
            Ok(response) => {
                let (parts, body) = response.into_parts();
                let boxed_body = body
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
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

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http();

    let mut response = client.request(req).await?;

    debug!(
        "WebSocket upgrade response from {}: {}",
        app_name,
        response.status()
    );

    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let server_upgrade = hyper::upgrade::on(&mut response);
        let app_name_clone = app_name.to_string();

        tokio::spawn(async move {
            let client_result = client_upgrade.await;
            let server_result = server_upgrade.await;

            match (client_result, server_result) {
                (Ok(client_upgraded), Ok(server_upgraded)) => {
                    debug!("Both WebSocket upgrades successful for {}", app_name_clone);
                    if let Err(e) =
                        proxy_websocket_connection(client_upgraded, server_upgraded, app_name_clone)
                            .await
                    {
                        error!("WebSocket proxy error: {}", e);
                    }
                }
                (Err(e), _) => {
                    error!("Failed to upgrade client WebSocket connection: {}", e);
                }
                (_, Err(e)) => {
                    error!("Failed to upgrade server WebSocket connection: {}", e);
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::{SinkExt, StreamExt};

    let client_ws =
        WebSocketStream::from_raw_socket(TokioIo::new(client_upgraded), Role::Server, None).await;

    let server_ws =
        WebSocketStream::from_raw_socket(TokioIo::new(server_upgraded), Role::Client, None).await;

    debug!("WebSocket bidirectional proxy established for {}", app_name);

    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut server_sink, mut server_stream) = server_ws.split();

    let client_to_server = async {
        while let Some(msg) = client_stream.next().await {
            match msg {
                Ok(msg) => {
                    if msg.is_close() {
                        debug!("Client sent close frame");
                        let _ = server_sink.send(msg).await;
                        break;
                    }
                    if let Err(e) = server_sink.send(msg).await {
                        error!("Error forwarding client -> server: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("Error reading from client: {}", e);
                    break;
                }
            }
        }
    };

    let server_to_client = async {
        while let Some(msg) = server_stream.next().await {
            match msg {
                Ok(msg) => {
                    if msg.is_close() {
                        debug!("Server sent close frame");
                        let _ = client_sink.send(msg).await;
                        break;
                    }
                    if let Err(e) = client_sink.send(msg).await {
                        error!("Error forwarding server -> client: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("Error reading from server: {}", e);
                    break;
                }
            }
        }
    };

    use futures_util::future::join;

    let (_, _) = join(client_to_server, server_to_client).await;

    debug!("WebSocket connection closed for {}", app_name);
    Ok(())
}

async fn forward_request(
    mut req: Request<Incoming>,
    app_name: &str,
    port: u16,
    remote_addr: SocketAddr,
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

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http();

    let response = client.request(req).await?;

    debug!("Response from {}: {}", app_name, response.status());

    Ok(response)
}
