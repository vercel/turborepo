#[cfg(not(test))]
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
#[cfg(not(test))]
use axum::{extract::Query, response::Redirect, routing::get, Router};
use log::debug;
#[cfg(not(test))]
use log::warn;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::{
    client::UserClient,
    commands::CommandBase,
    get_version,
    ui::{start_spinner, BOLD, CYAN},
};

#[cfg(test)]
pub const EXPECTED_TOKEN_TEST: &str = "expected_token";

const DEFAULT_HOST_NAME: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9789;

pub async fn login(base: &mut CommandBase) -> Result<()> {
    let repo_config = base.repo_config()?;
    let login_url_base = repo_config.login_url();
    debug!("turbo v{}", get_version());
    debug!("api url: {}", repo_config.api_url());
    debug!("login url: {login_url_base}");

    let redirect_url = format!("http://{DEFAULT_HOST_NAME}:{DEFAULT_PORT}");
    let login_url = format!("{login_url_base}/turborepo/token?redirect_uri={redirect_url}");
    println!(">>> Opening browser to {login_url}");
    direct_user_to_url(&login_url);
    let spinner = start_spinner("Waiting for your authorization...");
    let token_cell = Arc::new(OnceCell::new());
    new_one_shot_server(
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
    let client = base.api_client()?.unwrap();
    let user_response = client.get_user().await?;
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
async fn new_one_shot_server(_: u16, _: String, login_token: Arc<OnceCell<String>>) -> Result<()> {
    login_token.set(EXPECTED_TOKEN_TEST.to_string()).unwrap();
    Ok(())
}

#[cfg(not(test))]
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

#[cfg(test)]
mod test {
    use std::{fs, net::SocketAddr};

    use anyhow::Result;
    use axum::{routing::get, Json, Router};
    use serde::Deserialize;
    use tempfile::NamedTempFile;
    use tokio::sync::OnceCell;

    use crate::{
        client::{User, UserResponse},
        commands::{login, login::EXPECTED_TOKEN_TEST, CommandBase},
        config::{RepoConfigLoader, UserConfigLoader},
        ui::UI,
        Args,
    };

    #[tokio::test]
    async fn test_login() {
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();
        let repo_config_file = NamedTempFile::new().unwrap();
        fs::write(
            repo_config_file.path(),
            r#"{ "apiurl": "http://localhost:3000" }"#,
        )
        .unwrap();

        let handle = tokio::spawn(start_test_server());
        let mut base = CommandBase {
            repo_root: Default::default(),
            ui: UI::new(false),
            user_config: OnceCell::from(
                UserConfigLoader::new(user_config_file.path().to_path_buf())
                    .load()
                    .unwrap(),
            ),
            repo_config: OnceCell::from(
                RepoConfigLoader::new(repo_config_file.path().to_path_buf())
                    .with_api(Some("http://localhost:3001".to_string()))
                    .load()
                    .unwrap(),
            ),
            args: Args::default(),
        };

        login::login(&mut base).await.unwrap();

        handle.abort();

        assert_eq!(
            base.user_config().unwrap().token().unwrap(),
            EXPECTED_TOKEN_TEST
        );
    }

    #[derive(Debug, Clone, Deserialize)]
    struct TokenRequest {
        #[cfg(not(test))]
        redirect_uri: String,
    }

    /// NOTE: Each test server should be on its own port to avoid any
    /// concurrency bugs.
    async fn start_test_server() -> Result<()> {
        let app = Router::new()
            // `GET /` goes to `root`
            .route(
                "/v2/user",
                get(|| async move {
                    Json(UserResponse {
                        user: User {
                            id: "my_user_id".to_string(),
                            username: "my_username".to_string(),
                            email: "my_email".to_string(),
                            name: None,
                            created_at: 0,
                        },
                    })
                }),
            );
        let addr = SocketAddr::from(([127, 0, 0, 1], 3001));

        Ok(axum_server::bind(addr)
            .serve(app.into_make_service())
            .await?)
    }
}
