use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use axum::{extract::Query, response::Redirect, routing::get, Router};
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::Error;

#[derive(Debug, PartialEq)]
pub enum UrlOpenStrategy {
    Real,
    Noop,
}

#[derive(Debug, Clone, Deserialize)]
struct LoginPayload {
    token: String,
}

#[async_trait]
pub trait LoginServer {
    async fn run(
        &self,
        port: u16,
        login_url_base: String,
        login_token: Arc<OnceCell<String>>,
    ) -> Result<(), Error>;
    fn open_web_browser(&self, url: &str) -> std::io::Result<()>;
}

/// Default login server opens a real login server.
pub struct DefaultLoginServer {
    pub open_strategy: UrlOpenStrategy,
}

impl Default for DefaultLoginServer {
    fn default() -> Self {
        Self {
            open_strategy: UrlOpenStrategy::Real,
        }
    }
}

impl DefaultLoginServer {
    pub fn new(open_strategy: UrlOpenStrategy) -> Self {
        Self { open_strategy }
    }
}

#[async_trait]
impl LoginServer for DefaultLoginServer {
    async fn run(
        &self,
        port: u16,
        login_url_base: String,
        login_token: Arc<OnceCell<String>>,
    ) -> Result<(), Error> {
        // Effectively acts as a mock server if the strategy is Noop.
        if self.open_strategy == UrlOpenStrategy::Noop {
            login_token
                .set(turborepo_vercel_api_mock::EXPECTED_TOKEN.to_string())
                .unwrap();
            return Ok(());
        }

        let handle = axum_server::Handle::new();
        let route_handle = handle.clone();
        let app = Router::new()
            // `GET /` goes to `root`
            .route(
                "/",
                get(|login_payload: Query<LoginPayload>| async move {
                    let _ = login_token.set(login_payload.0.token);
                    route_handle.shutdown();
                    Redirect::to(&format!("{login_url_base}/turborepo/success"))
                }),
            );
        let addr = SocketAddr::from(([127, 0, 0, 1], port));

        axum_server::bind(addr)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .expect("failed to start one-shot server");

        Ok(())
    }
    fn open_web_browser(&self, url: &str) -> std::io::Result<()> {
        match self.open_strategy {
            UrlOpenStrategy::Real => webbrowser::open(url),
            UrlOpenStrategy::Noop => Ok(()),
        }
    }
}
