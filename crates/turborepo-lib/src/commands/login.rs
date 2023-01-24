use std::{net::SocketAddr, sync::Arc};

use anyhow::{anyhow, Result};
use axum::{extract::Query, response::Redirect, routing::get, Router};
use log::{debug, warn};
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::{
    client::{APIClient, UserClient},
    config::{default_user_config_path, RepoConfig, UserConfig},
    get_version,
    ui::{BOLD, CYAN, UI},
};

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

pub async fn login(repo_config: RepoConfig) -> Result<()> {
    let login_url_base = &repo_config.login_url;
    debug!("turbo v{}", get_version());
    debug!("api url: {}", repo_config.api_url);
    debug!("login url: {login_url_base}");

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let login_url = format!("{login_url_base}/turborepo/token?redirect_uri={redirect_url}");
    println!(">>> Opening browser to {login_url}");

    direct_user_to_url(&login_url);

    let token_cell = Arc::new(OnceCell::new());
    new_one_shot_server(DEFAULT_PORT, repo_config.login_url, token_cell.clone()).await?;
    let token = token_cell
        .get()
        .ok_or_else(|| anyhow!("Failed to get token"))?;

    let mut user_config = UserConfig::load(&default_user_config_path()?, None)?;
    user_config.set_token(Some(token.to_string()))?;

    let client = APIClient::new(token, repo_config.api_url)?;
    let user_response = client.get_user().await?;

    let ui = UI::infer();

    println!();
    println!(
        "{} Turborepo CLI authorized for {}",
        ui.rainbow(">>> Success!"),
        user_response.user.email,
    );
    println!();
    println!(
        "{}",
        ui.apply(
            CYAN.apply_to("To connect to your Remote Cache, run the following in any turborepo:")
        )
    );
    println!();
    println!("{}", ui.apply(BOLD.apply_to("  npx turbo link")));
    println!();

    Ok(())
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
