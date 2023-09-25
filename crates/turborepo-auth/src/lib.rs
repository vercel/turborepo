#[cfg(not(test))]
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
#[cfg(not(test))]
use axum::{extract::Query, response::Redirect, routing::get, Router};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::OnceCell;
#[cfg(not(test))]
use tracing::warn;
use turborepo_ui::{start_spinner, BOLD, CYAN};

// TODO: fix these imports not to use turborepo-lib
use crate::commands::CommandBase;

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;
const DEFAULT_SSO_PROVIDER: &str = "SAML/OIDC Single Sign-On";

use thiserror::Error;
#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "loginUrl is configured to \"{value}\", but cannot be a base URL. This happens in \
         situations like using a `data:` URL."
    )]
    LoginUrlCannotBeABase { value: String },
}

pub async fn login(base: &mut CommandBase) -> Result<()> {
    let repo_config = base.repo_config()?;
    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let login_url_configuration = repo_config.login_url();
    let mut login_url = Url::parse(login_url_configuration)?;

    login_url
        .path_segments_mut()
        .map_err(|_: ()| Error::LoginUrlCannotBeABase {
            value: login_url_configuration.to_string(),
        })?
        .extend(["turborepo", "token"]);

    login_url
        .query_pairs_mut()
        .append_pair("redirect_uri", &redirect_url);

    println!(">>> Opening browser to {login_url}");
    let spinner = start_spinner("Waiting for your authorization...");
    direct_user_to_url(login_url.as_str());

    let token_cell = Arc::new(OnceCell::new());
    run_login_one_shot_server(
        DEFAULT_PORT,
        repo_config.login_url().to_string(),
        token_cell.clone(),
    )
    .await?;

    spinner.finish_and_clear();

    let token = token_cell
        .get()
        .ok_or_else(|| anyhow!("Failed to get token"))?;

    base.user_config_mut()?.set_token(Some(token.to_string()))?;

    let client = base.api_client()?;
    let user_response = client.get_user(token.as_str()).await?;

    let ui = &base.ui;

    println!(
        "
{} Turborepo CLI authorized for {}

{}

{}

",
        ui.rainbow(">>> Success!"),
        user_response.user.email,
        ui.apply(
            CYAN.apply_to("To connect to your Remote Cache, run the following in any turborepo:")
        ),
        ui.apply(BOLD.apply_to("  npx turbo link"))
    );
    Ok(())
}

// TODO: Duplicated
#[cfg(test)]
fn direct_user_to_url(_: &str) {}
#[cfg(not(test))]
fn direct_user_to_url(url: &str) {
    if webbrowser::open(url).is_err() {
        warn!("Failed to open browser. Please visit {url} in your browser.");
    }
}

#[derive(Debug, Clone, Deserialize)]
struct LoginPayload {
    #[cfg(not(test))]
    token: String,
}

#[cfg(test)]
async fn run_login_one_shot_server(
    _: u16,
    _: String,
    login_token: Arc<OnceCell<String>>,
) -> Result<()> {
    login_token
        .set(turborepo_vercel_api_mock::EXPECTED_TOKEN.to_string())
        .unwrap();
    Ok(())
}

#[cfg(not(test))]
async fn run_login_one_shot_server(
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
