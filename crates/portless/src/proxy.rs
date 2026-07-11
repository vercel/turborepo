//! Streaming HTTP reverse proxy used by Portless.

use std::{
    convert::Infallible,
    error::Error as StdError,
    fmt::Write as _,
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{CONTENT_TYPE, HOST},
    service::service_fn,
    HeaderMap, Request, Response, StatusCode, Uri,
};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::{TokioExecutor, TokioIo},
};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpSocket},
    sync::watch,
    task::JoinSet,
};

use crate::pages::{render_page, ARROW_SVG};

/// Response header used by health checks to identify Portless.
pub const PORTLESS_HEADER: &str = "X-Portless";
const PORTLESS_HOPS_HEADER: &str = "x-portless-hops";
const MAX_PROXY_HOPS: u8 = 5;

type BoxError = Box<dyn StdError + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, BoxError>;
type ErrorLogger = Arc<dyn Fn(String) + Send + Sync>;
type RouteProvider = Arc<dyn Fn() -> Vec<ProxyRoute> + Send + Sync>;

/// The subset of a route needed by the proxy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyRoute {
    pub hostname: String,
    pub port: u16,
}

impl ProxyRoute {
    #[must_use]
    pub fn new(hostname: impl Into<String>, port: u16) -> Self {
        Self {
            hostname: hostname.into(),
            port,
        }
    }
}

impl From<(String, u16)> for ProxyRoute {
    fn from((hostname, port): (String, u16)) -> Self {
        Self { hostname, port }
    }
}

impl From<crate::routes::Route> for ProxyRoute {
    fn from(route: crate::routes::Route) -> Self {
        Self {
            hostname: route.hostname,
            port: route.port,
        }
    }
}

/// TLS material accepted by the API.
///
/// Portless's JavaScript server terminates TLS and negotiates HTTP/2. The
/// crate's declared dependencies contain no TLS implementation, so supplying
/// this configuration currently makes [`ProxyServer::bind`] return
/// [`ProxyError::TlsUnavailable`] instead of silently serving plaintext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TlsConfig {
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
    pub ca: Option<Vec<u8>>,
}

/// Runtime proxy configuration.
#[derive(Clone)]
pub struct ProxyOptions {
    pub get_routes: RouteProvider,
    pub proxy_port: u16,
    pub tld: String,
    pub tlds: Vec<String>,
    pub strict: bool,
    pub bind_address: Option<IpAddr>,
    pub tls: Option<TlsConfig>,
    pub on_error: ErrorLogger,
}

impl std::fmt::Debug for ProxyOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyOptions")
            .field("proxy_port", &self.proxy_port)
            .field("tld", &self.tld)
            .field("tlds", &self.tlds)
            .field("strict", &self.strict)
            .field("bind_address", &self.bind_address)
            .field("tls", &self.tls)
            .finish_non_exhaustive()
    }
}

impl ProxyOptions {
    #[must_use]
    pub fn new<F, I, R>(proxy_port: u16, get_routes: F) -> Self
    where
        F: Fn() -> I + Send + Sync + 'static,
        I: IntoIterator<Item = R>,
        R: Into<ProxyRoute>,
    {
        Self {
            get_routes: Arc::new(move || get_routes().into_iter().map(Into::into).collect()),
            proxy_port,
            tld: "localhost".to_owned(),
            tlds: Vec::new(),
            strict: true,
            bind_address: None,
            tls: None,
            on_error: Arc::new(|message| eprintln!("{message}")),
        }
    }

    #[must_use]
    pub fn with_error_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        self.on_error = Arc::new(handler);
        self
    }
}

/// Errors produced while creating or running a proxy.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("failed to bind Portless proxy: {0}")]
    Bind(#[source] std::io::Error),
    #[error(
        "TLS/HTTP2 was configured, but the declared crate dependencies do not include a TLS \
         implementation"
    )]
    TlsUnavailable,
    #[error("failed to query proxy listener address: {0}")]
    LocalAddress(#[source] std::io::Error),
    #[error("proxy accept failed: {0}")]
    Accept(#[source] std::io::Error),
}

/// A handle that requests graceful server shutdown.
#[derive(Clone, Debug)]
pub struct ProxyShutdown(watch::Sender<bool>);

impl ProxyShutdown {
    pub fn shutdown(&self) {
        let _ = self.0.send(true);
    }
}

/// A bound Portless proxy server.
pub struct ProxyServer {
    listener: TcpListener,
    options: Arc<ProxyOptions>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl ProxyServer {
    /// Bind the configured port. With no explicit address, an IPv6 wildcard
    /// socket with `IPV6_V6ONLY=0` accepts both IPv4 and IPv6 loopback traffic.
    pub async fn bind(options: ProxyOptions) -> Result<Self, ProxyError> {
        if options.tls.is_some() {
            return Err(ProxyError::TlsUnavailable);
        }

        let listener = bind_listener(options.bind_address, options.proxy_port)
            .await
            .map_err(ProxyError::Bind)?;
        Ok(Self::from_listener(listener, options))
    }

    #[must_use]
    pub fn from_listener(listener: TcpListener, options: ProxyOptions) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            listener,
            options: Arc::new(options),
            shutdown_tx,
            shutdown_rx,
        }
    }

    pub fn local_addr(&self) -> Result<SocketAddr, ProxyError> {
        self.listener.local_addr().map_err(ProxyError::LocalAddress)
    }

    #[must_use]
    pub fn shutdown_handle(&self) -> ProxyShutdown {
        ProxyShutdown(self.shutdown_tx.clone())
    }

    /// Serve until [`ProxyShutdown::shutdown`] is called.
    pub async fn run(self) -> Result<(), ProxyError> {
        self.run_inner(std::future::pending::<()>()).await
    }

    /// Serve until either the handle is used or the supplied future completes.
    pub async fn run_until<F>(self, shutdown: F) -> Result<(), ProxyError>
    where
        F: Future<Output = ()>,
    {
        self.run_inner(shutdown).await
    }

    async fn run_inner<F>(mut self, shutdown: F) -> Result<(), ProxyError>
    where
        F: Future<Output = ()>,
    {
        let mut connections = JoinSet::new();
        let client: Client<HttpConnector, Incoming> =
            Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, remote_addr) = accepted.map_err(ProxyError::Accept)?;
                    let options = Arc::clone(&self.options);
                    let client = client.clone();
                    let mut connection_shutdown = self.shutdown_rx.clone();
                    connections.spawn(async move {
                        let service = service_fn(move |request| {
                            proxy_request(
                                request,
                                remote_addr,
                                Arc::clone(&options),
                                client.clone(),
                            )
                        });
                        let connection = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .with_upgrades();
                        tokio::pin!(connection);
                        tokio::select! {
                            result = &mut connection => {
                                if let Err(error) = result {
                                    // Malformed and abruptly closed client connections are local
                                    // request failures, not fatal listener failures.
                                    let _ = error;
                                }
                            }
                            _ = connection_shutdown.changed() => {
                                connection.as_mut().graceful_shutdown();
                                let _ = connection.await;
                            }
                        }
                    });
                }
                _ = self.shutdown_rx.changed() => break,
                () = &mut shutdown => {
                    let _ = self.shutdown_tx.send(true);
                    break;
                }
            }
        }

        while tokio::time::timeout(Duration::from_secs(5), connections.join_next())
            .await
            .is_ok_and(|joined| joined.is_some())
        {}
        Ok(())
    }
}

async fn bind_listener(address: Option<IpAddr>, port: u16) -> std::io::Result<TcpListener> {
    match address {
        Some(IpAddr::V4(ip)) => TcpListener::bind((ip, port)).await,
        Some(IpAddr::V6(ip)) => TcpListener::bind((ip, port)).await,
        None => {
            let socket = TcpSocket::new_v6()?;
            match socket.bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)) {
                Ok(()) => socket.listen(1024),
                Err(_) => TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await,
            }
        }
    }
}

async fn proxy_request(
    mut request: Request<Incoming>,
    remote_addr: SocketAddr,
    options: Arc<ProxyOptions>,
    client: Client<HttpConnector, Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    let host_header = request_host(&request);
    let host = hostname(&host_header);

    if host.is_empty() {
        return Ok(text_response(
            StatusCode::BAD_REQUEST,
            "Missing Host header",
        ));
    }

    let hops = request
        .headers()
        .get(PORTLESS_HOPS_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(0);
    if hops >= MAX_PROXY_HOPS {
        (options.on_error)(format!(
            "Loop detected for {host}: request has passed through portless {hops} times. This \
             usually means a backend is proxying back through portless without rewriting the Host \
             header. If you use Vite/webpack proxy, set changeOrigin: true."
        ));
        return Ok(loop_response(hops, &options));
    }

    let routes = (options.get_routes)();
    let Some(route) = find_route(&routes, &host, options.strict) else {
        return Ok(not_found_response(&host, &routes, &options));
    };

    let is_upgrade = is_websocket_upgrade(request.headers());
    let client_upgrade = is_upgrade.then(|| hyper::upgrade::on(&mut request));
    add_forwarded_headers(request.headers_mut(), &host_header, remote_addr, false);
    if let Ok(value) = hyper::header::HeaderValue::from_str(&(hops + 1).to_string()) {
        request.headers_mut().insert(PORTLESS_HOPS_HEADER, value);
    }

    let path = request
        .uri()
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str);
    let target = format!("http://localhost:{}{path}", route.port);
    let Ok(uri) = target.parse::<Uri>() else {
        (options.on_error)(format!(
            "Proxy error for {host_header}: invalid backend URI"
        ));
        return Ok(bad_gateway_response(false));
    };
    *request.uri_mut() = uri;

    match client.request(request).await {
        Ok(mut response) => {
            response.headers_mut().insert(
                PORTLESS_HEADER,
                hyper::header::HeaderValue::from_static("1"),
            );
            if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                if let Some(client_upgrade) = client_upgrade {
                    let backend_upgrade = hyper::upgrade::on(&mut response);
                    tokio::spawn(async move {
                        if let (Ok(client_stream), Ok(backend_stream)) =
                            (client_upgrade.await, backend_upgrade.await)
                        {
                            let mut client_stream = TokioIo::new(client_stream);
                            let mut backend_stream = TokioIo::new(backend_stream);
                            let _ =
                                copy_bidirectional(&mut client_stream, &mut backend_stream).await;
                        }
                    });
                }
            }
            Ok(response.map(|body| {
                body.map_err(|error| -> BoxError { Box::new(error) })
                    .boxed_unsync()
            }))
        }
        Err(error) => {
            (options.on_error)(format!("Proxy error for {host_header}: {error}"));
            Ok(bad_gateway_response(error.is_connect()))
        }
    }
}

fn request_host(request: &Request<Incoming>) -> String {
    request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(hyper::http::uri::Authority::as_str)
        })
        .unwrap_or_default()
        .to_owned()
}

fn hostname(authority: &str) -> String {
    authority
        .parse::<hyper::http::uri::Authority>()
        .map_or_else(
            |_| authority.split(':').next().unwrap_or_default().to_owned(),
            |parsed| parsed.host().to_owned(),
        )
        .trim_matches(['[', ']'])
        .to_ascii_lowercase()
}

fn find_route<'a>(routes: &'a [ProxyRoute], host: &str, strict: bool) -> Option<&'a ProxyRoute> {
    routes
        .iter()
        .find(|route| route.hostname.eq_ignore_ascii_case(host))
        .or_else(|| {
            (!strict).then(|| {
                routes.iter().find(|route| {
                    host.len() > route.hostname.len()
                        && host
                            .get(host.len() - route.hostname.len()..)
                            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(&route.hostname))
                        && host.as_bytes().get(host.len() - route.hostname.len() - 1) == Some(&b'.')
                })
            })?
        })
}

fn add_forwarded_headers(headers: &mut HeaderMap, host: &str, remote_addr: SocketAddr, tls: bool) {
    let remote = remote_addr.ip().to_string();
    append_or_insert(headers, "x-forwarded-for", &remote);
    insert_if_absent(
        headers,
        "x-forwarded-proto",
        if tls { "https" } else { "http" },
    );
    insert_if_absent(headers, "x-forwarded-host", host);
    let forwarded_port = host
        .parse::<hyper::http::uri::Authority>()
        .ok()
        .and_then(|authority| authority.port_u16())
        .map_or_else(
            || {
                if tls {
                    "443".to_owned()
                } else {
                    "80".to_owned()
                }
            },
            |p| p.to_string(),
        );
    insert_if_absent(headers, "x-forwarded-port", &forwarded_port);
}

fn append_or_insert(headers: &mut HeaderMap, name: &'static str, value: &str) {
    let combined = headers
        .get(name)
        .and_then(|existing| existing.to_str().ok())
        .map_or_else(
            || value.to_owned(),
            |existing| format!("{existing}, {value}"),
        );
    if let Ok(value) = hyper::header::HeaderValue::from_str(&combined) {
        headers.insert(name, value);
    }
}

fn insert_if_absent(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if !headers.contains_key(name) {
        if let Ok(value) = hyper::header::HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    }
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(hyper::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        && headers
            .get(hyper::header::CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            })
}

fn empty_body() -> ProxyBody {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed_unsync()
}

fn body(value: impl Into<Bytes>) -> ProxyBody {
    Full::new(value.into())
        .map_err(|never| match never {})
        .boxed_unsync()
}

fn response(
    status: StatusCode,
    content_type: &str,
    content: impl Into<Bytes>,
) -> Response<ProxyBody> {
    let mut response = Response::new(body(content));
    *response.status_mut() = status;
    response.headers_mut().insert(
        PORTLESS_HEADER,
        hyper::header::HeaderValue::from_static("1"),
    );
    if let Ok(value) = hyper::header::HeaderValue::from_str(content_type) {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
}

fn text_response(status: StatusCode, content: &str) -> Response<ProxyBody> {
    response(status, "text/plain", content.to_owned())
}

fn loop_response(hops: u8, options: &ProxyOptions) -> Response<ProxyBody> {
    let primary_tld = options
        .tlds
        .first()
        .filter(|value| !value.is_empty())
        .unwrap_or(&options.tld);
    let body = format!(
        r#"<div class="content"><p class="desc">This request has passed through portless {hops} times. This usually means a dev server (Vite, webpack, etc.) is proxying requests back through portless without rewriting the Host header.</p><div class="section"><p class="label">Fix: add changeOrigin to your proxy config</p><pre class="terminal">proxy: {{
  "/api": {{
    target: "http://&lt;backend&gt;.{}:&lt;port&gt;",
    changeOrigin: true,
  }},
}}</pre></div></div>"#,
        escape_html(primary_tld)
    );
    response(
        StatusCode::LOOP_DETECTED,
        "text/html",
        render_page(508, "Loop Detected", &body),
    )
}

fn not_found_response(
    host: &str,
    routes: &[ProxyRoute],
    options: &ProxyOptions,
) -> Response<ProxyBody> {
    let escaped_host = escape_html(host);
    let suffixes = effective_tlds(options);
    let stripped = suffixes
        .iter()
        .find_map(|suffix| host.strip_suffix(suffix))
        .unwrap_or(host);
    let routes_list = if routes.is_empty() {
        r#"<p class="empty">No apps running.</p>"#.to_owned()
    } else {
        let mut items = String::new();
        for route in routes {
            let _ = write!(
                items,
                r#"<li><a href="{}" class="card-link"><span class="name">{}</span><span class="meta"><code class="port">127.0.0.1:{}</code><span class="arrow">{ARROW_SVG}</span></span></a></li>"#,
                escape_html(&format_url(&route.hostname, options.proxy_port, false)),
                escape_html(&route.hostname),
                route.port
            );
        }
        format!(
            r#"<div class="section"><p class="label">Active apps</p><ul class="card">{items}</ul></div>"#
        )
    };
    let content = format!(
        r#"<div class="content"><p class="desc">No app registered for <strong>{escaped_host}</strong></p>{routes_list}<div class="section"><p class="label">Start one</p><div class="terminal"><span class="prompt">$</span> portless {} your-command</div></div></div>"#,
        escape_html(stripped)
    );
    response(
        StatusCode::NOT_FOUND,
        "text/html",
        render_page(404, "Not Found", &content),
    )
}

fn effective_tlds(options: &ProxyOptions) -> Vec<String> {
    let tlds = if options.tlds.is_empty() {
        vec![options.tld.as_str()]
    } else {
        options.tlds.iter().map(String::as_str).collect()
    };
    let mut suffixes = Vec::new();
    for tld in tlds {
        let suffix = format!(".{tld}");
        if !suffixes.contains(&suffix) {
            suffixes.push(suffix);
        }
    }
    suffixes
}

fn bad_gateway_response(connection_refused: bool) -> Response<ProxyBody> {
    let detail = if connection_refused {
        "The target app is not responding. It may have crashed."
    } else {
        "The target app may not be running."
    };
    let content = format!(r#"<div class="content"><p class="desc">{detail}</p></div>"#);
    response(
        StatusCode::BAD_GATEWAY,
        "text/html",
        render_page(502, "Bad Gateway", &content),
    )
}

fn format_url(hostname: &str, port: u16, tls: bool) -> String {
    let scheme = if tls { "https" } else { "http" };
    let default_port = if tls { 443 } else { 80 };
    if port == default_port {
        format!("{scheme}://{hostname}")
    } else {
        format!("{scheme}://{hostname}:{port}")
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Create the small redirect response used by TLS frontends.
#[must_use]
pub fn https_redirect(host: &str, path: &str, https_port: u16) -> Response<ProxyBody> {
    let hostname = hostname(host);
    let port = if https_port == 443 {
        String::new()
    } else {
        format!(":{https_port}")
    };
    let location = format!("https://{hostname}{port}{path}");
    let mut response = Response::new(empty_body());
    *response.status_mut() = StatusCode::FOUND;
    if let Ok(value) = hyper::header::HeaderValue::from_str(&location) {
        response
            .headers_mut()
            .insert(hyper::header::LOCATION, value);
    }
    response.headers_mut().insert(
        PORTLESS_HEADER,
        hyper::header::HeaderValue::from_static("1"),
    );
    response
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        sync::{Arc, Mutex},
    };

    use http_body_util::Full;
    use hyper::{body::Bytes, service::service_fn, HeaderMap, Request, Response};
    use hyper_util::rt::TokioIo;
    use tokio::{io::AsyncWriteExt, net::TcpListener};

    use super::{
        escape_html, find_route, format_url, ProxyError, ProxyOptions, ProxyRoute, ProxyServer,
        TlsConfig, MAX_PROXY_HOPS, PORTLESS_HEADER,
    };

    async fn backend() -> (u16, tokio::task::JoinHandle<()>, Arc<Mutex<Vec<HeaderMap>>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("backend binds");
        let port = listener.local_addr().expect("backend address").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let task_seen = Arc::clone(&seen);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let task_seen = Arc::clone(&task_seen);
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                        task_seen
                            .lock()
                            .expect("headers lock")
                            .push(request.headers().clone());
                        async {
                            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(
                                b"hello from backend",
                            ))))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (port, task, seen)
    }

    async fn start_proxy(
        options: ProxyOptions,
    ) -> (
        SocketAddr,
        super::ProxyShutdown,
        tokio::task::JoinHandle<()>,
    ) {
        let mut options = options;
        options.bind_address = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let server = ProxyServer::bind(options).await.expect("proxy binds");
        let address = server.local_addr().expect("proxy address");
        let shutdown = server.shutdown_handle();
        let task = tokio::spawn(async move {
            server.run().await.expect("proxy runs");
        });
        (address, shutdown, task)
    }

    async fn raw_request(address: SocketAddr, request: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connects to proxy");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("writes request");
        let mut response = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response)
            .await
            .expect("reads response");
        String::from_utf8(response).expect("HTTP is utf8")
    }

    #[test]
    fn exact_route_wins_and_wildcard_obeys_strict() {
        let routes = vec![
            ProxyRoute::new("app.localhost", 3000),
            ProxyRoute::new("tenant.app.localhost", 3001),
        ];
        assert_eq!(
            find_route(&routes, "tenant.app.localhost", false).map(|route| route.port),
            Some(3001)
        );
        assert!(find_route(&routes, "other.app.localhost", true).is_none());
        assert_eq!(
            find_route(&routes, "other.app.localhost", false).map(|route| route.port),
            Some(3000)
        );
        assert!(find_route(&routes, "notapp.localhost", false).is_none());
    }

    #[test]
    fn utility_output_matches_node_port() {
        assert_eq!(
            escape_html(r#"<a x="'">&"#),
            "&lt;a x=&quot;&#39;&quot;&gt;&amp;"
        );
        assert_eq!(
            format_url("app.localhost", 80, false),
            "http://app.localhost"
        );
        assert_eq!(
            format_url("app.localhost", 1355, false),
            "http://app.localhost:1355"
        );
    }

    #[tokio::test]
    async fn proxies_stream_and_adds_forwarded_headers() {
        let (backend_port, backend_task, seen) = backend().await;
        let options = ProxyOptions::new(0, move || {
            vec![ProxyRoute::new("app.localhost", backend_port)]
        });
        let (address, shutdown, proxy_task) = start_proxy(options).await;
        let response = raw_request(
            address,
            "GET /hello HTTP/1.1\r\nHost: app.localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.to_ascii_lowercase().contains("x-portless: 1"));
        assert!(response.ends_with("hello from backend"));
        {
            let headers = seen.lock().expect("headers lock");
            assert_eq!(headers[0]["x-forwarded-host"], "app.localhost");
            assert_eq!(headers[0]["x-forwarded-proto"], "http");
            assert_eq!(headers[0]["x-forwarded-port"], "80");
            assert_eq!(headers[0]["x-portless-hops"], "1");
        }
        shutdown.shutdown();
        proxy_task.await.expect("proxy joins");
        backend_task.abort();
    }

    #[tokio::test]
    async fn renders_400_404_502_and_508_pages() {
        let options = ProxyOptions::new(0, || vec![ProxyRoute::new("dead.localhost", 9)]);
        let (address, shutdown, proxy_task) = start_proxy(options).await;

        let missing = raw_request(address, "GET / HTTP/1.0\r\n\r\n").await;
        assert!(missing
            .lines()
            .next()
            .is_some_and(|line| line.contains(" 400 ")));
        assert!(missing.contains("Missing Host header"));

        let missing_route = raw_request(
            address,
            "GET / HTTP/1.1\r\nHost: unknown.localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(missing_route.starts_with("HTTP/1.1 404"));
        assert!(missing_route.contains("Not Found"));

        let dead = raw_request(
            address,
            "GET / HTTP/1.1\r\nHost: dead.localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(dead.starts_with("HTTP/1.1 502"));
        assert!(dead.contains("Bad Gateway"));

        let looped = raw_request(
            address,
            &format!(
                "GET / HTTP/1.1\r\nHost: dead.localhost\r\nx-portless-hops: \
                 {MAX_PROXY_HOPS}\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert!(looped.starts_with("HTTP/1.1 508"));
        assert!(looped.contains("Loop Detected"));
        assert!(looped.contains("changeOrigin"));
        assert!(looped
            .to_ascii_lowercase()
            .contains(&format!("{}: 1", PORTLESS_HEADER.to_ascii_lowercase())));

        shutdown.shutdown();
        proxy_task.await.expect("proxy joins");
    }

    #[tokio::test]
    async fn tls_configuration_fails_closed() {
        let mut options = ProxyOptions::new(0, Vec::<ProxyRoute>::new);
        options.tls = Some(TlsConfig {
            cert: vec![1],
            key: vec![2],
            ca: None,
        });
        assert!(matches!(
            ProxyServer::bind(options).await,
            Err(ProxyError::TlsUnavailable)
        ));
    }

    #[tokio::test]
    async fn default_listener_accepts_ipv4_and_ipv6_loopback() {
        let server = ProxyServer::bind(ProxyOptions::new(0, Vec::<ProxyRoute>::new))
            .await
            .expect("dual-stack proxy binds");
        let port = server.local_addr().expect("proxy address").port();
        let shutdown = server.shutdown_handle();
        let proxy_task = tokio::spawn(async move {
            server.run().await.expect("proxy runs");
        });

        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let response = raw_request(
                SocketAddr::new(ip, port),
                "GET / HTTP/1.1\r\nHost: unknown.localhost\r\nConnection: close\r\n\r\n",
            )
            .await;
            assert!(response.starts_with("HTTP/1.1 404"));
        }

        shutdown.shutdown();
        proxy_task.await.expect("proxy joins");
    }

    #[tokio::test]
    async fn relays_websocket_upgrade_and_stream_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let backend = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("websocket backend binds");
        let backend_port = backend.local_addr().expect("backend address").port();
        let backend_task = tokio::spawn(async move {
            let (mut stream, _) = backend.accept().await.expect("backend accepts");
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.expect("reads handshake");
                request.push(byte[0]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: \
                      websocket\r\nSec-WebSocket-Accept: test\r\n\r\n",
                )
                .await
                .expect("writes upgrade");
            let mut payload = [0_u8; 4];
            stream
                .read_exact(&mut payload)
                .await
                .expect("reads payload");
            stream.write_all(&payload).await.expect("echoes payload");
        });

        let options = ProxyOptions::new(0, move || {
            vec![ProxyRoute::new("ws.localhost", backend_port)]
        });
        let (address, shutdown, proxy_task) = start_proxy(options).await;
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connects to proxy");
        stream
            .write_all(
                b"GET /socket HTTP/1.1\r\nHost: ws.localhost\r\nConnection: Upgrade\r\nUpgrade: \
                  websocket\r\nSec-WebSocket-Key: dGVzdA==\r\nSec-WebSocket-Version: 13\r\n\r\n",
            )
            .await
            .expect("writes handshake");
        let mut response = Vec::new();
        let mut byte = [0_u8; 1];
        while !response.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.expect("reads handshake");
            response.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 101"));
        stream.write_all(b"ping").await.expect("writes payload");
        let mut echoed = [0_u8; 4];
        stream.read_exact(&mut echoed).await.expect("reads echo");
        assert_eq!(&echoed, b"ping");

        shutdown.shutdown();
        proxy_task.await.expect("proxy joins");
        backend_task.await.expect("backend joins");
    }
}
