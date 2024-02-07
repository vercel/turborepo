use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, DefaultLoginServer, DefaultSSOLoginServer,
    LoginOptions,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};

use crate::{cli::Error, commands::CommandBase, config, rewrite_json::set_path};

pub async fn sso_login(
    base: &mut CommandBase,
    sso_team: &str,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    telemetry.track_login_method(LoginMethod::SSO);
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

pub async fn login(base: &mut CommandBase, telemetry: CommandEventBuilder) -> Result<(), Error> {
    telemetry.track_login_method(LoginMethod::Standard);
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();
    let options = LoginOptions {
        existing_token: base.config()?.token(),
        ..LoginOptions::new(&ui, &login_url_config, &api_client, &DefaultLoginServer)
    };

    let token = auth_login(&options).await?;

    // Don't write to disk if the token is already there
    if token.exists {
        return Ok(());
    }

    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;
    let after = set_path(&before, &["token"], &format!("\"{}\"", token.token))?;

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
