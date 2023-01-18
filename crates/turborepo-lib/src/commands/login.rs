use std::{net::SocketAddr, sync::Arc};

use axum::{routing::get, Router};
use log::{debug, info, warn};
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
    new_one_shot_server(DEFAULT_PORT, query.clone()).await;
    println!("{}", query.get().unwrap());
}

fn direct_user_to_url(url: &str) {
    if webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }
}

async fn new_one_shot_server(port: u16, query: Arc<OnceCell<String>>) {
    let handle = axum_server::Handle::new();
    let route_handle = handle.clone();
    let app = Router::new()
        // `GET /` goes to `root`
        .route(
            "/",
            get(|| async move {
                let _ = query.set("hello friends".to_string());
                route_handle.shutdown();
            }),
        );
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("listening on {}", addr);
    axum_server::bind(addr)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
