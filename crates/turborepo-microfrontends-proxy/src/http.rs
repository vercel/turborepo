use std::net::SocketAddr;

use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use hyper_util::client::legacy::Client;
use tracing::{debug, error, warn};

use crate::{ProxyError, error::ErrorPage, router::RouteMatch};

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
        http_client,
    )
    .await;

    handle_forward_result(
        result,
        path,
        route_match.app_name,
        route_match.port,
        remote_addr,
        "HTTP",
    )
}

pub(crate) fn handle_forward_result(
    result: Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>>,
    path: String,
    app_name: impl AsRef<str>,
    port: u16,
    remote_addr: SocketAddr,
    request_type: &str,
) -> Result<Response<BoxedBody>, ProxyError> {
    match result {
        Ok(response) => {
            debug!(
                "Forwarding {} response from {} with status {} to client {}",
                request_type,
                app_name.as_ref(),
                response.status(),
                remote_addr.ip()
            );
            convert_response_to_boxed_body(response, app_name.as_ref())
        }
        Err(e) => {
            warn!(
                "Failed to {} forward request to {}: {}",
                request_type.to_lowercase(),
                app_name.as_ref(),
                e
            );
            build_error_response(path, app_name.as_ref(), port)
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

pub(crate) async fn forward_request(
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
    headers.insert("Host", format!("localhost:{port}").parse()?);
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
}
