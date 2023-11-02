use turborepo_api_client::{APIClient, Client};
use turborepo_auth::{
    login as auth_login, read_or_create_auth_file, sso_login as auth_sso_login, AuthToken,
    DefaultLoginServer, DefaultSSOLoginServer,
};

use crate::{cli::Error, commands::CommandBase, rewrite_json::set_path};

/// Entry point for `turbo login --sso`.
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

    let global_auth_path = base.global_auth_path()?;
    let before = global_auth_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| Error::FailedToReadAuth {
            auth_path: global_auth_path.clone(),
            error: e,
        })?;

    let after = set_path(&before, &["token"], &format!("\"{}\"", token))?;
    global_auth_path
        .ensure_dir()
        .map_err(|e| Error::FailedToSetAuth {
            auth_path: global_auth_path.clone(),
            error: e,
        })?;

    global_auth_path
        .create_with_contents(after)
        .map_err(|e| Error::FailedToSetAuth {
            auth_path: global_auth_path.clone(),
            error: e,
        })?;

    Ok(())
}

/// Entry point for `turbo login`.
pub async fn login(base: &mut CommandBase) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    // Get both possible token paths for existing tokens checks.
    let global_auth_path = base.global_auth_path()?;
    let global_config_path = base.global_config_path()?;

    let mut auth_file =
        read_or_create_auth_file(&global_auth_path, &global_config_path, &api_client).await?;

    if auth_file.get_token(api_client.base_url()).is_some() {
        // Token already exists, return early.
        return Ok(());
    }

    // Get the raw token from login. This will update the auth file as well.
    let token = auth_login(
        &api_client,
        &ui,
        &global_auth_path,
        &login_url_config,
        &DefaultLoginServer,
    )
    .await?;

    // Create the new token format.
    let auth_token = AuthToken {
        token: token.to_string(),
        api: api_client.base_url().to_string(),
        created_at: None,
        teams: Vec::new(),
    };

    // Write it to the disk and call it a day.
    auth_file.add_or_update_token(auth_token);
    auth_file.write_to_disk(&global_auth_path)?;

    Ok(())
}
