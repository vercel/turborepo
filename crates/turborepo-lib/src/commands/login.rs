use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, DefaultLoginServer, DefaultSSOLoginServer,
};

use crate::{cli::Error, commands::CommandBase, config, rewrite_json::set_path};

pub async fn sso_login(base: &mut CommandBase, sso_team: &str) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    let token = auth_sso_login(
        &api_client,
        &ui,
        base.config()?.token(),
        &login_url_config,
        sso_team,
        &DefaultSSOLoginServer,
    )
    .await?;

    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    let after = set_path(&before, &["token"], &format!("\"{}\"", token))?;

    global_config_path
        .ensure_dir()
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    global_config_path
        .create_with_contents(after)
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    Ok(())
}

pub async fn login(base: &mut CommandBase) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    let token = auth_login(
        &api_client,
        &ui,
        base.config()?.token(),
        &login_url_config,
        &DefaultLoginServer,
    )
    .await?;

    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;
    let after = set_path(&before, &["token"], &format!("\"{}\"", token))?;

    global_config_path
        .ensure_dir()
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    global_config_path
        .create_with_contents(after)
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    Ok(())
}
