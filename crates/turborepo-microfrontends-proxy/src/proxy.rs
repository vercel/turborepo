use std::{net::SocketAddr, sync::Arc};

use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
};
use tracing::{debug, error};

use crate::{
    ProxyError,
    headers::{is_websocket_upgrade, validate_request_headers},
    http::{BoxedBody, HttpClient, handle_http_request},
    http_router::Router,
    websocket::{WebSocketContext, handle_websocket_request},
};

pub(crate) async fn handle_request(
    mut req: Request<Incoming>,
    router: Arc<Router>,
    remote_addr: SocketAddr,
    ws_ctx: WebSocketContext,
    http_client: HttpClient,
) -> Result<Response<BoxedBody>, hyper::Error> {
    if let Err(e) = validate_request_headers(&req) {
        error!("Request validation error: {}", e);
        return Ok(create_generic_error_response(e));
    }

    let path = req.uri().path().to_string();
    let method = req.method().clone();

    debug!("Request: {} {} from {}", method, path, remote_addr.ip());

    let route_match = router.match_route(&path);
    debug!(
        "Matched route: app={}, port={}",
        route_match.app_name, route_match.port
    );

    let result = if is_websocket_upgrade(&req) {
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
    };

    match result {
        Ok(response) => Ok(response),
        Err(e) => {
            error!("Proxy error: {}", e);
            Ok(create_generic_error_response(e))
        }
    }
}

fn create_generic_error_response(error: ProxyError) -> Response<BoxedBody> {
    let body_text = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Proxy Error</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: 'Geist', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: hsl(0, 0%, 100%);
            color: hsl(0, 0%, 9%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            background: hsl(0, 0%, 95%);
            border: 1px solid hsl(0, 0%, 92%);
            border-radius: 12px;
            box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
            max-width: 600px;
            width: 100%;
            padding: 40px;
        }}
        h1 {{
            color: hsl(358, 75%, 59%);
            font-size: 24px;
            margin-bottom: 16px;
            display: flex;
            align-items: center;
            gap: 12px;
        }}
        .error-icon {{
            font-size: 24px;
            flex-shrink: 0;
        }}
        .error-message {{
            background: hsl(0, 0%, 100%);
            border-left: 4px solid hsl(358, 75%, 59%);
            padding: 16px;
            margin: 20px 0;
            border-radius: 4px;
        }}
        .error-message code {{
            background: hsl(0, 0%, 92%);
            padding: 2px 6px;
            border-radius: 3px;
            font-family: 'Geist Mono', 'Monaco', 'Menlo', 'Consolas', monospace;
            font-size: 14px;
            color: hsl(0, 0%, 9%);
            word-break: break-all;
        }}
        .details {{
            color: hsl(0, 0%, 40%);
            font-size: 14px;
            line-height: 1.6;
            margin-top: 16px;
        }}
        @media (prefers-color-scheme: dark) {{
            body {{
                background: hsl(0, 0%, 3.9%);
                color: hsl(0, 0%, 93%);
            }}
            .container {{
                background: hsl(0, 0%, 10%);
                border-color: hsl(0, 0%, 12%);
                box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
            }}
            h1 {{
                color: hsl(358, 100%, 69%);
            }}
            .error-message {{
                background: hsl(0, 0%, 12%);
                border-left-color: hsl(358, 100%, 69%);
            }}
            .error-message code {{
                background: hsl(0, 0%, 16%);
                color: hsl(0, 0%, 93%);
            }}
            .details {{
                color: hsl(0, 0%, 63%);
            }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1><span class="error-icon">⚠️</span>Proxy Error</h1>
        <p class="details">
            The Turborepo microfrontends proxy encountered an error while processing your request.
        </p>
        <div class="error-message">
            <code>{error}</code>
        </div>
    </div>
</body>
</html>"#
    );

    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(
            Full::new(Bytes::from(body_text))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                .boxed(),
        )
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(
                    Full::new(Bytes::from("Internal Server Error"))
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                        .boxed(),
                )
                .unwrap()
        })
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
