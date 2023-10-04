use anyhow::Result;
use turborepo_api_client::APIClient;
use turborepo_auth::{login as auth_login, sso_login as auth_sso_login};

use crate::commands::CommandBase;

pub async fn sso_login(base: &mut CommandBase, sso_team: &str) -> Result<()> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.repo_config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        Ok(base.user_config_mut()?.set_token(Some(token.to_string()))?)
    };

    auth_sso_login(api_client, &ui, set_token, &login_url_config, sso_team).await
}

pub async fn login(base: &mut CommandBase) -> Result<()> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.repo_config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        Ok(base.user_config_mut()?.set_token(Some(token.to_string()))?)
    };

    auth_login(api_client, &ui, set_token, &login_url_config).await
}
