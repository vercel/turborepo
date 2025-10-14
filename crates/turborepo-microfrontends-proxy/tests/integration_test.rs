use std::{net::SocketAddr, time::Duration};

use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::{client::legacy::Client, rt::TokioIo};
use tokio::net::TcpListener;
use turborepo_microfrontends::Config;
use turborepo_microfrontends_proxy::{ProxyServer, Router};

const WEBSOCKET_CLOSE_DELAY: Duration = Duration::from_millis(100);

#[tokio::test]
async fn test_port_availability_check_ipv4() {
    let config_json = r#"{
        "version": "1",
        "options": {
            "localProxyPort": 9999
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
    let server = ProxyServer::new(config.clone()).unwrap();

    let _listener = TcpListener::bind("127.0.0.1:9999").await.unwrap();

    let result = server.check_port_available().await;
    assert!(!result, "Port should not be available when already bound");
}

#[tokio::test]
async fn test_port_availability_check_ipv6() {
    let config_json = r#"{
        "version": "1",
        "options": {
            "localProxyPort": 9997
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

    let _listener = TcpListener::bind("127.0.0.1:9997").await.unwrap();

    let result = server.check_port_available().await;
    assert!(!result, "Port should not be available when already bound");
}

#[tokio::test]
async fn test_router_with_config() {
    let config_json = r#"{
        "version": "1",
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 3000 }
                }
            },
            "docs": {
                "development": {
                    "local": { "port": 3001 }
                },
                "routing": [
                    { "paths": ["/docs", "/docs/:path*"] }
                ]
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let router = Router::new(&config).unwrap();

    let route = router.match_route("/");
    assert_eq!(route.app_name.as_ref(), "web");
    assert_eq!(route.port, 3000);

    let route = router.match_route("/docs");
    assert_eq!(route.app_name.as_ref(), "docs");
    assert_eq!(route.port, 3001);

    let route = router.match_route("/docs/api/reference");
    assert_eq!(route.app_name.as_ref(), "docs");
    assert_eq!(route.port, 3001);

    let route = router.match_route("/about");
    assert_eq!(route.app_name.as_ref(), "web");
    assert_eq!(route.port, 3000);
}

#[tokio::test]
async fn test_multiple_child_apps() {
    let config_json = r#"{
        "version": "1",
        "applications": {
            "main": {
                "development": {
                    "local": { "port": 3000 }
                }
            },
            "blog": {
                "development": {
                    "local": { "port": 3001 }
                },
                "routing": [
                    { "paths": ["/blog", "/blog/:path*"] }
                ]
            },
            "docs": {
                "development": {
                    "local": { "port": 3002 }
                },
                "routing": [
                    { "paths": ["/docs", "/docs/:path*"] }
                ]
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let router = Router::new(&config).unwrap();

    assert_eq!(router.match_route("/").app_name.as_ref(), "main");
    assert_eq!(router.match_route("/blog").app_name.as_ref(), "blog");
    assert_eq!(router.match_route("/blog/post").app_name.as_ref(), "blog");
    assert_eq!(router.match_route("/docs").app_name.as_ref(), "docs");
    assert_eq!(router.match_route("/docs/api").app_name.as_ref(), "docs");
    assert_eq!(router.match_route("/other").app_name.as_ref(), "main");
}

#[tokio::test]
async fn test_proxy_server_creation() {
    let config_json = r#"{
        "version": "1",
        "options": {
            "localProxyPort": 4000
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
    let server = ProxyServer::new(config);

    assert!(server.is_ok());
}

#[tokio::test]
async fn test_pattern_matching_edge_cases() {
    let config_json = r#"{
        "version": "1",
        "applications": {
            "main": {
                "development": {
                    "local": { "port": 3000 }
                }
            },
            "api": {
                "development": {
                    "local": { "port": 3001 }
                },
                "routing": [
                    { "paths": ["/api/v1/:endpoint"] }
                ]
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let router = Router::new(&config).unwrap();

    assert_eq!(router.match_route("/api/v1/users").app_name.as_ref(), "api");
    assert_eq!(router.match_route("/api/v1/posts").app_name.as_ref(), "api");

    assert_eq!(
        router.match_route("/api/v1/users/123").app_name.as_ref(),
        "main"
    );
    assert_eq!(
        router.match_route("/api/v2/users").app_name.as_ref(),
        "main"
    );
}

async fn find_available_port() -> Result<u16, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

async fn mock_server(
    port: u16,
    response_text: &'static str,
) -> Result<tokio::task::JoinHandle<()>, Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);

            let service = service_fn(move |_req: Request<Incoming>| async move {
                Ok::<_, hyper::Error>(
                    Response::builder()
                        .status(200)
                        .body(Full::new(Bytes::from(response_text)))
                        .unwrap(),
                )
            });

            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        }
    });

    tokio::time::sleep(WEBSOCKET_CLOSE_DELAY).await;
    Ok(handle)
}

#[tokio::test]
async fn test_end_to_end_proxy() {
    let web_port = find_available_port().await.unwrap();
    let docs_port = find_available_port().await.unwrap();
    let proxy_port = find_available_port().await.unwrap();

    let web_handle = mock_server(web_port, "web app").await.unwrap();
    let docs_handle = mock_server(docs_port, "docs app").await.unwrap();

    let config_json = format!(
        r#"{{
        "version": "1",
        "options": {{
            "localProxyPort": {}
        }},
        "applications": {{
            "web": {{
                "development": {{
                    "local": {{ "port": {} }}
                }}
            }},
            "docs": {{
                "development": {{
                    "local": {{ "port": {} }}
                }},
                "routing": [
                    {{ "paths": ["/docs", "/docs/:path*"] }}
                ]
            }}
        }}
    }}"#,
        proxy_port, web_port, docs_port
    );

    let config = Config::from_str(&config_json, "test.json").unwrap();
    let mut server = ProxyServer::new(config).unwrap();

    let (shutdown_complete_tx, shutdown_complete_rx) = tokio::sync::oneshot::channel();
    server.set_shutdown_complete_tx(shutdown_complete_tx);
    let shutdown_handle = server.shutdown_handle();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let connector = hyper_util::client::legacy::connect::HttpConnector::new();
    let client: Client<_, Full<Bytes>> =
        Client::builder(hyper_util::rt::TokioExecutor::new()).build(connector);

    let web_response = client
        .get(format!("http://127.0.0.1:{}/", proxy_port).parse().unwrap())
        .await
        .unwrap();
    let web_body = web_response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(web_body, "web app");

    let docs_response = client
        .get(
            format!("http://127.0.0.1:{}/docs", proxy_port)
                .parse()
                .unwrap(),
        )
        .await
        .unwrap();
    let docs_body = docs_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(docs_body, "docs app");

    let docs_subpath_response = client
        .get(
            format!("http://127.0.0.1:{}/docs/api/reference", proxy_port)
                .parse()
                .unwrap(),
        )
        .await
        .unwrap();
    let docs_subpath_body = docs_subpath_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(docs_subpath_body, "docs app");

    let _ = shutdown_handle.send(());
    let _ = tokio::time::timeout(Duration::from_secs(2), shutdown_complete_rx).await;

    web_handle.abort();
    docs_handle.abort();
}

#[tokio::test]
async fn test_websocket_detection() {
    use hyper::{
        Request,
        header::{CONNECTION, UPGRADE},
    };

    let req = Request::builder()
        .uri("http://localhost:3000")
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .body(())
        .unwrap();

    assert!(req.headers().get(UPGRADE).is_some());
    assert!(req.headers().get(CONNECTION).is_some());
}

#[tokio::test]
async fn test_websocket_routing() {
    let config_json = r#"{
        "version": "1",
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 3000 }
                }
            },
            "api": {
                "development": {
                    "local": { "port": 3001 }
                },
                "routing": [
                    { "paths": ["/api/:path*"] }
                ]
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let router = Router::new(&config).unwrap();

    let route = router.match_route("/api/ws");
    assert_eq!(route.app_name.as_ref(), "api");
    assert_eq!(route.port, 3001);

    let route = router.match_route("/ws");
    assert_eq!(route.app_name.as_ref(), "web");
    assert_eq!(route.port, 3000);
}
