use std::{net::SocketAddr, sync::Arc};

use axum::{extract::Query, response::Redirect, routing::get, Router};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::{config::RepoConfig, get_version};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

pub async fn login(repo_config: RepoConfig) {
    let login_url_base = &repo_config.login_url;
    debug!("turbo v{}", get_version());
    debug!("api url: {}", repo_config.api_url);
    debug!("login url: {login_url_base}");

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let login_url = format!("{login_url_base}/turborepo/token?redirect_uri={redirect_url}");

    info!(">>> Opening browser to {login_url}");
    direct_user_to_url(&login_url);

    let query = Arc::new(OnceCell::new());
    new_one_shot_server(DEFAULT_PORT, repo_config.login_url, query.clone()).await;
}

fn direct_user_to_url(url: &str) {
    if webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LoginPayload {
    token: String,
}

async fn new_one_shot_server(
    port: u16,
    login_url_base: String,
    login_token: Arc<OnceCell<String>>,
) {
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
        .unwrap();
}
