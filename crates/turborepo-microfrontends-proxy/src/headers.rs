use hyper::{
    Request,
    header::{CONNECTION, CONTENT_LENGTH, TRANSFER_ENCODING, UPGRADE},
};

use crate::error::ProxyError;

/// Validates request headers to prevent HTTP request smuggling attacks.
///
/// While this proxy is intended for local development only, we check for
/// conflicting Content-Length and Transfer-Encoding headers, which could be
/// exploited if different servers in the chain interpret them differently.
pub(crate) fn validate_request_headers<T>(req: &Request<T>) -> Result<(), ProxyError> {
    let has_content_length = req.headers().contains_key(CONTENT_LENGTH);
    let has_transfer_encoding = req.headers().contains_key(TRANSFER_ENCODING);

    if has_content_length && has_transfer_encoding {
        return Err(ProxyError::InvalidRequest(
            "Conflicting Content-Length and Transfer-Encoding headers".to_string(),
        ));
    }

    Ok(())
}

/// Validates the Host header to prevent host header injection attacks.
///
/// This proxy is intended for local development only, so we restrict
/// Host headers to localhost or 127.0.0.1 addresses only.
pub(crate) fn validate_host_header(host: &str) -> Result<(), ProxyError> {
    if let Some(colon_idx) = host.rfind(':') {
        let host_part = &host[..colon_idx];
        let port_part = &host[colon_idx + 1..];

        if (host_part == "localhost" || host_part == "127.0.0.1")
            && !port_part.is_empty()
            && port_part.chars().all(|c| c.is_ascii_digit())
        {
            return Ok(());
        }
    } else if host == "localhost" || host == "127.0.0.1" {
        return Ok(());
    }

    Err(ProxyError::InvalidRequest(
        "Invalid host header: only localhost and 127.0.0.1 are allowed".to_string(),
    ))
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
    use hyper::{Method, header::HeaderValue};

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
            .uri("http://localhost:3000/api")
            .body(())
            .unwrap();

        assert!(validate_request_headers(&req).is_ok());
    }

    #[test]
    fn test_validate_request_headers_transfer_encoding_only() {
        let req = Request::builder()
            .method(Method::POST)
            .uri("http://localhost:3000/api")
            .header(TRANSFER_ENCODING, "chunked")
            .body(())
            .unwrap();

        assert!(validate_request_headers(&req).is_ok());
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
    fn test_header_value_creation() {
        let host = HeaderValue::from_str("localhost:3000");
        assert!(host.is_ok());

        let forwarded_for = HeaderValue::from_str("127.0.0.1");
        assert!(forwarded_for.is_ok());

        let forwarded_proto = HeaderValue::from_str("http");
        assert!(forwarded_proto.is_ok());
    }
}
