use anyhow::{anyhow, Result};
use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, DefaultLoginServer, DefaultSSOLoginServer,
};

use crate::{commands::CommandBase, rewrite_json::set_path};

pub async fn sso_login(base: &mut CommandBase, sso_team: &str) -> Result<()> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        let global_config_path = base.global_config_path()?;
        let before = global_config_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| {
                anyhow!(
                    "Encountered an IO error while attempting to read {}: {}",
                    global_config_path,
                    e
                )
            })?;
        let after = set_path(&before, &["token"], &format!("\"{}\"", token))?;
        global_config_path.ensure_dir()?;
        global_config_path.create_with_contents(after)?;
        Ok(())
    };

    auth_sso_login(
        &api_client,
        &ui,
        base.config()?.token(),
        set_token,
        &login_url_config,
        sso_team,
        &DefaultSSOLoginServer::new(),
    )
    .await
}

pub async fn login(base: &mut CommandBase) -> Result<()> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        let global_config_path = base.global_config_path()?;
        let before = global_config_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| {
                anyhow!(
                    "Encountered an IO error while attempting to read {}: {}",
                    global_config_path,
                    e
                )
            })?;
        let after = set_path(&before, &["token"], &format!("\"{}\"", token))?;
        global_config_path.ensure_dir()?;
        global_config_path.create_with_contents(after)?;
        Ok(())
    };

    auth_login(
        &api_client,
        &ui,
        base.config()?.token(),
        set_token,
        &login_url_config,
        &DefaultLoginServer::new(),
    )
    .await
}
