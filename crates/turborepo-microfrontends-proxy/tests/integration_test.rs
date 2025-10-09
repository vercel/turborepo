use std::net::SocketAddr;

use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use turborepo_microfrontends::Config;
use turborepo_microfrontends_proxy::{ProxyServer, Router};

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
    assert!(result.is_err(), "Should fail when IPv4 port is occupied");
}

#[tokio::test]
async fn test_port_availability_check_ipv6() {
    let config_json = r#"{
        "version": "1",
        "options": {
            "localProxyPort": 9998
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

    let _listener = TcpListener::bind("[::1]:9998").await.unwrap();

    let result = server.check_port_available().await;
    assert!(result.is_err(), "Should fail when IPv6 port is occupied");
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
    assert_eq!(route.app_name, "web");
    assert_eq!(route.port, 3000);

    let route = router.match_route("/docs");
    assert_eq!(route.app_name, "docs");
    assert_eq!(route.port, 3001);

    let route = router.match_route("/docs/api/reference");
    assert_eq!(route.app_name, "docs");
    assert_eq!(route.port, 3001);

    let route = router.match_route("/about");
    assert_eq!(route.app_name, "web");
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

    assert_eq!(router.match_route("/").app_name, "main");
    assert_eq!(router.match_route("/blog").app_name, "blog");
    assert_eq!(router.match_route("/blog/post").app_name, "blog");
    assert_eq!(router.match_route("/docs").app_name, "docs");
    assert_eq!(router.match_route("/docs/api").app_name, "docs");
    assert_eq!(router.match_route("/other").app_name, "main");
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

    assert_eq!(router.match_route("/api/v1/users").app_name, "api");
    assert_eq!(router.match_route("/api/v1/posts").app_name, "api");

    assert_eq!(router.match_route("/api/v1/users/123").app_name, "main");
    assert_eq!(router.match_route("/api/v2/users").app_name, "main");
}

async fn mock_server(
    port: u16,
    response_text: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;

    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);

            let service = service_fn(move |_req: Request<Incoming>| async move {
                Ok::<_, hyper::Error>(
                    Response::builder()
                        .status(200)
                        .body(response_text.to_string())
                        .unwrap(),
                )
            });

            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    Ok(())
}

#[tokio::test]
#[ignore] // This test requires actual HTTP servers and may conflict with other tests
async fn test_end_to_end_proxy() {
    mock_server(5000, "web app").await.unwrap();
    mock_server(5001, "docs app").await.unwrap();

    let config_json = r#"{
        "version": "1",
        "options": {
            "localProxyPort": 5024
        },
        "applications": {
            "web": {
                "development": {
                    "local": { "port": 5000 }
                }
            },
            "docs": {
                "development": {
                    "local": { "port": 5001 }
                },
                "routing": [
                    { "paths": ["/docs", "/docs/:path*"] }
                ]
            }
        }
    }"#;

    let config = Config::from_str(config_json, "test.json").unwrap();
    let server = ProxyServer::new(config).unwrap();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Note: Actual HTTP requests would go here
    // This is a placeholder for when we want to add full E2E tests
}
