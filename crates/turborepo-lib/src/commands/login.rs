use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, DefaultLoginServer, LoginOptions, Token,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};

use crate::{cli::Error, commands::CommandBase, config, rewrite_json::set_path};

pub async fn sso_login(
    base: &mut CommandBase,
    sso_team: &str,
    telemetry: CommandEventBuilder,
    force: bool,
) -> Result<(), Error> {
    telemetry.track_login_method(LoginMethod::SSO);
    let api_client: APIClient = base.api_client()?;
    let color_config = base.color_config;
    let login_url_config = base.config()?.login_url().to_string();
    let options = LoginOptions {
        existing_token: base.config()?.token(),
        sso_team: Some(sso_team),
        force,
        ..LoginOptions::new(
            &color_config,
            &login_url_config,
            &api_client,
            &DefaultLoginServer,
        )
    };

    let token = auth_sso_login(&options).await?;

    // Don't write to disk if the token is already there
    if matches!(token, Token::Existing(..)) {
        return Ok(());
    }

    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    let after = set_path(&before, &["token"], &format!("\"{}\"", token.into_inner()))?;

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

pub async fn login(
    base: &mut CommandBase,
    telemetry: CommandEventBuilder,
    force: bool,
) -> Result<(), Error> {
    let mut login_telemetry = LoginTelemetry::new(&telemetry, LoginMethod::Standard);

    let api_client: APIClient = base.api_client()?;
    let color_config = base.color_config;
    let login_url_config = base.config()?.login_url().to_string();
    let options = LoginOptions {
        existing_token: base.config()?.token(),
        force,
        ..LoginOptions::new(
            &color_config,
            &login_url_config,
            &api_client,
            &DefaultLoginServer,
        )
    };

    let token = auth_login(&options).await?;

    // Don't write to disk if the token is already there
    if matches!(token, Token::Existing(..)) {
        return Ok(());
    }

    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;
    let after = set_path(&before, &["token"], &format!("\"{}\"", token.into_inner()))?;

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

    login_telemetry.set_success(true);
    Ok(())
}

struct LoginTelemetry<'a> {
    telemetry: &'a CommandEventBuilder,
    method: LoginMethod,
    success: bool,
}
impl<'a> LoginTelemetry<'a> {
    fn new(telemetry: &'a CommandEventBuilder, method: LoginMethod) -> Self {
        Self {
            telemetry,
            method,
            success: false,
        }
    }
    fn set_success(&mut self, success: bool) {
        self.success = success;
    }
}
// If we get an early return, we still want to track the login attempt as a
// failure.
impl<'a> Drop for LoginTelemetry<'a> {
    fn drop(&mut self) {
        self.telemetry.track_login_method(self.method);
        self.telemetry.track_login_success(self.success);
    }
}
