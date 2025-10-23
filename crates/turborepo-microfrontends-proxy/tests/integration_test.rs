use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::{client::legacy::Client, rt::TokioIo};
use serial_test::serial;
use tokio::net::TcpListener;
use turborepo_microfrontends::Config;
use turborepo_microfrontends_proxy::{ProxyServer, Router};

#[tokio::test]
#[serial]
async fn test_port_availability_check_ipv4() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let config_json = format!(
        r#"{{
        "options": {{
            "localProxyPort": {port}
        }},
        "applications": {{
            "web": {{
                "development": {{
                    "local": {{ "port": 3000 }}
                }}
            }}
        }}
    }}"#
    );

    let config = Config::from_str(&config_json, "test.json").unwrap();
    let server = ProxyServer::new(config.clone()).unwrap();

    let result = server.check_port_available().await;
    assert!(!result, "Port should not be available when already bound");

    drop(listener);
}

#[tokio::test]
#[serial]
async fn test_port_availability_check_ipv6() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let config_json = format!(
        r#"{{
        "options": {{
            "localProxyPort": {port}
        }},
        "applications": {{
            "web": {{
                "development": {{
                    "local": {{ "port": 3000 }}
                }}
            }}
        }}
    }}"#
    );

    let config = Config::from_str(&config_json, "test.json").unwrap();
    let server = ProxyServer::new(config).unwrap();

    let result = server.check_port_available().await;
    assert!(!result, "Port should not be available when already bound");

    drop(listener);
}

#[tokio::test]
async fn test_router_with_config() {
    let config_json = r#"{
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let config_json = format!(
        r#"{{
        "options": {{
            "localProxyPort": {port}
        }},
        "applications": {{
            "web": {{
                "development": {{
                    "local": {{ "port": 3000 }}
                }}
            }}
        }}
    }}"#
    );

    let config = Config::from_str(&config_json, "test.json").unwrap();
    let server = ProxyServer::new(config);

    assert!(server.is_ok());
}

#[tokio::test]
async fn test_pattern_matching_edge_cases() {
    let config_json = r#"{
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

async fn find_available_port_range(
    count: usize,
) -> Result<(Vec<u16>, Vec<TcpListener>), Box<dyn std::error::Error>> {
    let mut available_ports = Vec::new();
    let mut listeners = Vec::new();

    for port in 3000..=9999 {
        if [3306, 5432, 6379].contains(&port) {
            continue;
        }
        if let Ok(listener) = TcpListener::bind(format!("127.0.0.1:{port}")).await {
            available_ports.push(port);
            listeners.push(listener);
            if available_ports.len() == count {
                return Ok((available_ports, listeners));
            }
        }
    }
    Err("Not enough available ports in allowed range".into())
}

fn mock_server(listener: TcpListener, response_text: &'static str) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
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
    })
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn test_end_to_end_proxy() {
    let (ports, mut listeners) = find_available_port_range(3).await.unwrap();
    let web_port = ports[0];
    let docs_port = ports[1];
    let proxy_port = ports[2];

    let web_listener = listeners.remove(0);
    let docs_listener = listeners.remove(0);
    let proxy_listener = listeners.remove(0);

    drop(proxy_listener);

    let web_handle = mock_server(web_listener, "web app");
    let docs_handle = mock_server(docs_listener, "docs app");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let config_json = format!(
        r#"{{
        "options": {{
            "localProxyPort": {proxy_port}
        }},
        "applications": {{
            "web": {{
                "development": {{
                    "local": {{ "port": {web_port} }}
                }}
            }},
            "docs": {{
                "development": {{
                    "local": {{ "port": {docs_port} }}
                }},
                "routing": [
                    {{ "paths": ["/docs", "/docs/:path*"] }}
                ]
            }}
        }}
    }}"#
    );

    let config = Config::from_str(&config_json, "test.json").unwrap();
    let mut server = ProxyServer::new(config).unwrap();

    let (shutdown_complete_tx, shutdown_complete_rx) = tokio::sync::oneshot::channel();
    server.set_shutdown_complete_tx(shutdown_complete_tx);
    let shutdown_handle = server.shutdown_handle();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let connector = hyper_util::client::legacy::connect::HttpConnector::new();
    let client: Client<_, Full<Bytes>> =
        Client::builder(hyper_util::rt::TokioExecutor::new()).build(connector);

    let web_response = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(format!("http://127.0.0.1:{proxy_port}/").parse().unwrap()),
    )
    .await
    .expect("Request timed out")
    .expect("Request failed");
    let web_body = web_response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(web_body, "web app");

    let docs_response = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(
            format!("http://127.0.0.1:{proxy_port}/docs")
                .parse()
                .unwrap(),
        ),
    )
    .await
    .expect("Request timed out")
    .expect("Request failed");
    let docs_body = docs_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(docs_body, "docs app");

    let docs_subpath_response = tokio::time::timeout(
        Duration::from_secs(5),
        client.get(
            format!("http://127.0.0.1:{proxy_port}/docs/api/reference")
                .parse()
                .unwrap(),
        ),
    )
    .await
    .expect("Request timed out")
    .expect("Request failed");
    let docs_subpath_body = docs_subpath_response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(docs_subpath_body, "docs app");

    let _ = shutdown_handle.send(());
    let _ = tokio::time::timeout(Duration::from_secs(3), shutdown_complete_rx).await;

    web_handle.abort();
    docs_handle.abort();

    tokio::time::sleep(Duration::from_millis(100)).await;
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

#[tokio::test]
async fn test_port_validation_blocks_invalid_ports() {
    // Test blocked port (SSH)
    let config_json = r#"{
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 22 }
                }
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let result = Router::new(&config);
    assert!(result.is_err(), "Should reject SSH port 22");
    if let Err(err) = result {
        assert!(err.contains("blocked for security reasons") || err.contains("Invalid port 22"));
    }

    // Test port below range
    let config_json = r#"{
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 1000 }
                }
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let result = Router::new(&config);
    assert!(result.is_err(), "Should reject port 1000 (below range)");
    if let Err(err) = result {
        assert!(err.contains("outside the allowed range") || err.contains("Invalid port 1000"));
    }

    // Test port above range
    let config_json = r#"{
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 10000 }
                }
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let result = Router::new(&config);
    assert!(result.is_err(), "Should reject port 10000 (above range)");
    if let Err(err) = result {
        assert!(err.contains("outside the allowed range") || err.contains("Invalid port 10000"));
    }

    // Test MySQL port (blocked even though in range)
    let config_json = r#"{
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 3306 }
                }
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let result = Router::new(&config);
    assert!(result.is_err(), "Should reject MySQL port 3306");
    if let Err(err) = result {
        assert!(err.contains("blocked for security reasons") || err.contains("Invalid port 3306"));
    }
}
