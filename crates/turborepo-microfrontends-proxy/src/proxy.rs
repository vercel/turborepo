use std::{net::SocketAddr, sync::Arc};

use hyper::{Request, Response, body::Incoming};
use tracing::debug;
use turborepo_microfrontends::Config;

use crate::{
    ProxyError,
    headers::{is_websocket_upgrade, validate_request_headers},
    http::{BoxedBody, HttpClient, handle_http_request},
    router::Router,
    websocket::{WebSocketContext, handle_websocket_request},
};

pub(crate) async fn handle_request(
    mut req: Request<Incoming>,
    router: Arc<Router>,
    _config: Arc<Config>,
    remote_addr: SocketAddr,
    ws_ctx: WebSocketContext,
    http_client: HttpClient,
) -> Result<Response<BoxedBody>, ProxyError> {
    validate_request_headers(&req)?;

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

        handle_websocket_request(
            req,
            route_match,
            path,
            remote_addr,
            req_upgrade,
            ws_ctx,
            http_client,
        )
        .await
    } else {
        handle_http_request(req, route_match, path, remote_addr, http_client).await
    }
}

#[cfg(test)]
mod tests {
    use crate::ProxyError;

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
}
