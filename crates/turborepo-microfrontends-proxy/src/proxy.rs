use std::net::SocketAddr;

use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
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

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
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

async fn handle_request(
    req: Request<Incoming>,
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
