use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use axum::{extract::Query, response::Redirect, routing::get, Router};
use serde::Deserialize;
use tokio::sync::OnceCell;

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
    ) -> Result<()>;
}

/// TODO: Document this.
#[derive(Default)]
pub struct DefaultLoginServer;

impl DefaultLoginServer {
    pub fn new() -> Self {
        DefaultLoginServer {}
    }
}

#[async_trait]
impl LoginServer for DefaultLoginServer {
    async fn run(
        &self,
        port: u16,
        login_url_base: String,
        login_token: Arc<OnceCell<String>>,
    ) -> Result<()> {
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

        Ok(axum_server::bind(addr)
            .handle(handle)
            .serve(app.into_make_service())
            .await?)
    }
}
