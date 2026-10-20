use hyper::{
    Request,
    header::{CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING, UPGRADE},
};

use crate::error::ProxyError;

/// Validates request headers to prevent HTTP request smuggling attacks.
///
/// While this proxy is intended for local development only, we check for
/// conflicting Content-Length and Transfer-Encoding headers, which could be
/// exploited if different servers in the chain interpret them differently. We
/// also require a single localhost Host header so browser-driven requests
/// cannot use this proxy as a forwarding primitive for arbitrary hostnames.
pub(crate) fn validate_request_headers<T>(req: &Request<T>) -> Result<(), ProxyError> {
    let has_content_length = req.headers().contains_key(CONTENT_LENGTH);
    let has_transfer_encoding = req.headers().contains_key(TRANSFER_ENCODING);

    if has_content_length && has_transfer_encoding {
        return Err(ProxyError::InvalidRequest(
            "Conflicting Content-Length and Transfer-Encoding headers".to_string(),
        ));
    }

    let host = validated_host_header(req)?;
    validate_request_uri_host(req, host)?;

    Ok(())
}

pub(crate) fn validated_host_header<T>(req: &Request<T>) -> Result<&str, ProxyError> {
    let mut host_headers = req.headers().get_all(HOST).iter();
    let host_header = host_headers
        .next()
        .ok_or_else(|| ProxyError::InvalidRequest("Missing Host header".to_string()))?;

    if host_headers.next().is_some() {
        return Err(ProxyError::InvalidRequest(
            "Duplicate Host headers are not allowed".to_string(),
        ));
    }

    let host = host_header
        .to_str()
        .map_err(|_| ProxyError::InvalidRequest("Malformed Host header".to_string()))?;

    validate_host_header(host)?;

    Ok(host)
}

fn validate_request_uri_host<T>(req: &Request<T>, host: &str) -> Result<(), ProxyError> {
    if let Some(authority) = req.uri().authority()
        && authority.as_str() != host
    {
        return Err(ProxyError::InvalidRequest(
            "Request URI host does not match Host header".to_string(),
        ));
    }

    Ok(())
}

/// Validates the Host header to prevent host header injection attacks.
///
/// This proxy is intended for local development only, so we restrict
/// Host headers to localhost or 127.0.0.1 addresses only.
pub(crate) fn validate_host_header(host: &str) -> Result<(), ProxyError> {
    if host == "localhost"
        || host == "127.0.0.1"
        || valid_localhost_with_port(host, "localhost")
        || valid_localhost_with_port(host, "127.0.0.1")
    {
        return Ok(());
    }

    Err(ProxyError::InvalidRequest(
        "Invalid host header: only localhost and 127.0.0.1 are allowed".to_string(),
    ))
}

fn valid_localhost_with_port(host: &str, allowed_host: &str) -> bool {
    host.strip_prefix(allowed_host)
        .and_then(|remaining| remaining.strip_prefix(':'))
        .is_some_and(|port| !port.is_empty() && port.parse::<u16>().is_ok())
}

pub(crate) fn is_websocket_upgrade<T>(req: &Request<T>) -> bool {
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

#[cfg(test)]
mod tests {
    use hyper::{
        Method,
        header::{HOST, HeaderValue},
    };

    use super::*;

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
    fn test_validate_request_headers_valid() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("http://localhost:3000/api")
            .header(HOST, "localhost:3000")
            .header(CONTENT_LENGTH, "100")
            .body(())
            .unwrap();

        assert!(validate_request_headers(&req).is_ok());
    }

    #[test]
    fn test_validate_request_headers_conflicting() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("http://localhost:3000/api")
            .header(HOST, "localhost:3000")
            .header(CONTENT_LENGTH, "100")
            .header(TRANSFER_ENCODING, "chunked")
            .body(())
            .unwrap();

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Conflicting"));
        }
    }

    #[test]
    fn test_validate_request_headers_no_body_headers() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api")
            .header(HOST, "localhost:3000")
            .body(())
            .unwrap();

        assert!(validate_request_headers(&req).is_ok());
    }

    #[test]
    fn test_validate_request_headers_transfer_encoding_only() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("http://localhost:3000/api")
            .header(HOST, "localhost:3000")
            .header(TRANSFER_ENCODING, "chunked")
            .body(())
            .unwrap();

        assert!(validate_request_headers(&req).is_ok());
    }

    #[test]
    fn test_validate_request_headers_missing_host() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api")
            .body(())
            .unwrap();

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Missing Host header"));
        }
    }

    #[test]
    fn test_validate_request_headers_duplicate_host() {
        let mut req = Request::builder()
            .method(Method::GET)
            .uri("/api")
            .header(HOST, "localhost:3000")
            .body(())
            .unwrap();
        req.headers_mut()
            .append(HOST, HeaderValue::from_static("127.0.0.1:3000"));

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Duplicate Host headers"));
        }
    }

    #[test]
    fn test_validate_request_headers_malformed_host() {
        let mut req = Request::builder()
            .method(Method::GET)
            .uri("/api")
            .body(())
            .unwrap();
        req.headers_mut().insert(
            HOST,
            HeaderValue::from_bytes(b"localhost:3000\xff").unwrap(),
        );

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Malformed Host header"));
        }
    }

    #[test]
    fn test_validate_request_headers_rejects_non_localhost_host() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api")
            .header(HOST, "example.com")
            .body(())
            .unwrap();

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Invalid host header"));
        }
    }

    #[test]
    fn test_validate_request_headers_rejects_mismatched_uri_host() {
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://evil.com/api")
            .header(HOST, "localhost:3000")
            .body(())
            .unwrap();

        let result = validate_request_headers(&req);
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Request URI host does not match Host header"));
        }
    }

    #[test]
    fn test_validate_host_header_localhost() {
        assert!(validate_host_header("localhost:3000").is_ok());
        assert!(validate_host_header("localhost:8080").is_ok());
    }

    #[test]
    fn test_validate_host_header_127_0_0_1() {
        assert!(validate_host_header("127.0.0.1:3000").is_ok());
        assert!(validate_host_header("127.0.0.1:8080").is_ok());
    }

    #[test]
    fn test_validate_host_header_localhost_no_port() {
        assert!(validate_host_header("localhost").is_ok());
    }

    #[test]
    fn test_validate_host_header_127_0_0_1_no_port() {
        assert!(validate_host_header("127.0.0.1").is_ok());
    }

    #[test]
    fn test_validate_host_header_invalid_hostname() {
        let result = validate_host_header("example.com:3000");
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Invalid host header"));
        }
    }

    #[test]
    fn test_validate_host_header_invalid_ip() {
        let result = validate_host_header("192.168.1.1:3000");
        assert!(result.is_err());
        if let Err(ProxyError::InvalidRequest(msg)) = result {
            assert!(msg.contains("Invalid host header"));
        }
    }

    #[test]
    fn test_validate_host_header_malicious_injection() {
        let result = validate_host_header("localhost:3000\r\nX-Injected: evil");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_host_header_invalid_port() {
        assert!(validate_host_header("localhost:").is_err());
        assert!(validate_host_header("localhost:abc").is_err());
        assert!(validate_host_header("localhost:99999").is_err());
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
}
