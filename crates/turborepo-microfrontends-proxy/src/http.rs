use std::net::SocketAddr;

use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use hyper_util::client::legacy::Client;
use tracing::{debug, error, warn};

use crate::{
    ProxyError, error::ErrorPage, headers::validate_host_header, http_router::RouteMatch,
    ports::validate_port,
};

pub(crate) type BoxedBody = BoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) type HttpClient = Client<hyper_util::client::legacy::connect::HttpConnector, Incoming>;

pub(crate) async fn handle_http_request(
    req: Request<Incoming>,
    route_match: RouteMatch,
    path: String,
    remote_addr: SocketAddr,
    http_client: HttpClient,
) -> Result<Response<BoxedBody>, ProxyError> {
    let result = forward_request(
        req,
        &route_match.app_name,
        route_match.port,
        remote_addr,
        http_client.clone(),
    )
    .await;

    handle_forward_result(result, path, route_match, remote_addr, http_client, "HTTP").await
}

pub(crate) async fn forward_request(
    mut req: Request<Incoming>,
    app_name: &str,
    port: u16,
    remote_addr: SocketAddr,
    http_client: HttpClient,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    // Validate port to prevent SSRF attacks
    validate_port(port).map_err(|e| {
        warn!(
            "Port validation failed for {} (port {}): {}",
            app_name, port, e
        );
        Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Port validation failed: {e}"),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

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

    let response = http_client.request(req).await?;

    debug!("Response from {}: {}", app_name, response.status());

    Ok(response)
}

pub(crate) async fn handle_forward_result(
    result: Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>>,
    path: String,
    route_match: RouteMatch,
    remote_addr: SocketAddr,
    http_client: HttpClient,
    request_type: &str,
) -> Result<Response<BoxedBody>, ProxyError> {
    match result {
        Ok(response) => {
            debug!(
                "Forwarding {} response from {} with status {} to client {}",
                request_type,
                route_match.app_name,
                response.status(),
                remote_addr.ip()
            );
            convert_response_to_boxed_body(response, &route_match.app_name)
        }
        Err(e) => {
            debug!(
                "Failed to {} forward request to {}: {}",
                request_type.to_lowercase(),
                route_match.app_name,
                e
            );

            if let Some(fallback_url) = &route_match.fallback {
                match try_fallback(
                    &path,
                    fallback_url,
                    remote_addr,
                    http_client,
                    &route_match.app_name,
                )
                .await
                {
                    Ok(response) => return Ok(response),
                    Err(fallback_error) => {
                        warn!(
                            "Fallback URL {} also failed for {}: {}",
                            fallback_url, route_match.app_name, fallback_error
                        );
                    }
                }
            }

            build_error_response(path, &route_match.app_name, route_match.port)
        }
    }
}

pub(crate) fn convert_response_to_boxed_body(
    response: Response<Incoming>,
    app_name: &str,
) -> Result<Response<BoxedBody>, ProxyError> {
    let app_name = app_name.to_string();
    let (parts, body) = response.into_parts();
    let boxed_body = body
        .map_err(move |e| {
            error!("Error reading body from upstream {}: {}", app_name, e);
            Box::new(e) as Box<dyn std::error::Error + Send + Sync>
        })
        .boxed();
    Ok(Response::from_parts(parts, boxed_body))
}

pub(crate) fn build_error_response(
    path: String,
    app_name: &str,
    port: u16,
) -> Result<Response<BoxedBody>, ProxyError> {
    let error_page = ErrorPage::new(path, app_name.to_string(), port);

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

async fn try_fallback(
    path: &str,
    fallback_base_url: &str,
    remote_addr: SocketAddr,
    _http_client: HttpClient,
    app_name: &str,
) -> Result<Response<BoxedBody>, Box<dyn std::error::Error + Send + Sync>> {
    let fallback_url = normalize_fallback_url(fallback_base_url, path)?;

    debug!(
        "Attempting fallback for {} to URL: {}",
        app_name, fallback_url
    );

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()?
        .https_or_http()
        .enable_http1()
        .build();

    let client: Client<_, Empty<Bytes>> =
        Client::builder(hyper_util::rt::TokioExecutor::new()).build(https);

    let req = Request::builder()
        .uri(&fallback_url)
        .header("X-Forwarded-For", remote_addr.ip().to_string())
        .header("X-Forwarded-Proto", "http")
        .body(Empty::<Bytes>::new())?;

    let response = client.request(req).await?;

    debug!(
        "Fallback response for {} status: {}",
        app_name,
        response.status()
    );

    convert_response_to_boxed_body(response, app_name).map_err(|e| Box::new(e) as Box<_>)
}

fn normalize_fallback_url(
    fallback_base: &str,
    path: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Ensure the base has a scheme
    let base = if fallback_base.starts_with("http://") || fallback_base.starts_with("https://") {
        fallback_base.to_string()
    } else {
        format!("https://{fallback_base}")
    };

    // Parse the base URL - this validates it's well-formed
    let base_url = url::Url::parse(&base).map_err(|e| format!("Invalid fallback base URL: {e}"))?;

    // Store the original host for validation
    let original_host = base_url
        .host()
        .ok_or("Fallback base URL must have a host")?;

    // Normalize the path - if empty, use "/"
    let normalized_path = if path.is_empty() { "/" } else { path };

    // Use join() to safely combine base with path
    // This automatically normalizes .. segments and prevents directory traversal
    let final_url = base_url
        .join(normalized_path)
        .map_err(|e| format!("Invalid path for fallback URL: {e}"))?;

    // Security check: verify the host hasn't changed
    // This prevents attacks using absolute URLs or protocol-relative URLs in the
    // path
    let final_host = final_url.host().ok_or("Final URL must have a host")?;
    if final_host != original_host {
        return Err("Path must not change the fallback host".into());
    }

    Ok(final_url.to_string())
}

#[cfg(test)]
mod tests {
    use http_body_util::Full;
    use hyper::body::Bytes;

    use super::*;

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

    #[test]
    fn test_normalize_fallback_url() {
        assert_eq!(
            normalize_fallback_url("example.com", "/docs").unwrap(),
            "https://example.com/docs"
        );

        assert_eq!(
            normalize_fallback_url("https://example.com", "/api/test").unwrap(),
            "https://example.com/api/test"
        );

        assert_eq!(
            normalize_fallback_url("http://localhost:8080", "/").unwrap(),
            "http://localhost:8080/"
        );

        assert_eq!(
            normalize_fallback_url("example.com/", "/path").unwrap(),
            "https://example.com/path"
        );

        assert_eq!(
            normalize_fallback_url("example.com", "").unwrap(),
            "https://example.com/"
        );
    }

    #[test]
    fn test_normalize_fallback_url_path_traversal_prevention() {
        // Test basic path traversal attempt with ../
        let result = normalize_fallback_url("example.com", "/docs/../etc/passwd");
        assert!(result.is_ok());
        // The url crate normalizes this to /etc/passwd (which is still on example.com)
        assert_eq!(result.unwrap(), "https://example.com/etc/passwd");

        // Test multiple .. segments
        let result = normalize_fallback_url("example.com/app/api", "/../../../etc/passwd");
        assert!(result.is_ok());
        // Normalized to root, then etc/passwd
        assert_eq!(result.unwrap(), "https://example.com/etc/passwd");

        // Test that we stay on the same host even with traversal
        let result = normalize_fallback_url("https://example.com/base", "/../../test");
        assert!(result.is_ok());
        let url = result.unwrap();
        assert!(url.starts_with("https://example.com/"));
        assert!(!url.contains(".."));
    }

    #[test]
    fn test_normalize_fallback_url_absolute_url_rejection() {
        // Test that absolute URLs in path are rejected if they change the host
        let result = normalize_fallback_url("example.com", "https://evil.com/attack");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Path must not change the fallback host")
        );

        // Test protocol-relative URL
        let result = normalize_fallback_url("example.com", "//evil.com/attack");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Path must not change the fallback host")
        );
    }

    #[test]
    fn test_normalize_fallback_url_encoded_traversal() {
        // Test URL-encoded path traversal (the url crate handles decoding)
        let result = normalize_fallback_url("example.com", "/docs/%2e%2e/etc/passwd");
        assert!(result.is_ok());
        // The url crate will decode and normalize this
        let url = result.unwrap();
        assert!(url.starts_with("https://example.com/"));
        assert!(!url.contains("%2e"));
    }

    #[test]
    fn test_normalize_fallback_url_stays_on_host() {
        // Verify that various path manipulations keep us on the same host
        let test_cases = vec![
            ("example.com", "/normal/path"),
            ("example.com", "/path/./with/./dots"),
            ("example.com", "/path/../other"),
            ("https://example.com/base", "/new/path"),
            ("https://example.com/base/", "/new/path"),
        ];

        for (base, path) in test_cases {
            let result = normalize_fallback_url(base, path);
            assert!(result.is_ok(), "Failed for base={base}, path={path}");
            let url = result.unwrap();
            assert!(
                url.contains("example.com"),
                "URL {url} doesn't contain example.com"
            );
            // Ensure no .. remains in the final URL
            assert!(!url.contains(".."), "URL {url} still contains ..");
        }
    }
}
