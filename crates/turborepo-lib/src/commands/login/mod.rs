mod manual;

use manual::login_manual;
use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, DefaultLoginServer, LoginOptions, Token,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};

use crate::{commands::CommandBase, config, rewrite_json::set_path};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to read user input. {0}")]
    UserInput(#[from] dialoguer::Error),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    Auth(#[from] turborepo_auth::Error),
    #[error("Unable to edit `turbo.json`. {0}")]
    JsonEdit(#[from] crate::rewrite_json::RewriteError),
    #[error("The provided credentials do not have cache access. Please double check them.")]
    NoCacheAccess,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TurboJsonParse(#[from] crate::turbo_json::parser::Error),
}

pub async fn login(
    base: &mut CommandBase,
    telemetry: CommandEventBuilder,
    sso_team: Option<&str>,
    force: bool,
    manual: bool,
) -> Result<(), Error> {
    match sso_team {
        Some(sso_team) => {
            telemetry.track_login_method(LoginMethod::SSO);
            sso_login(base, sso_team, force).await
        }
        None if manual => {
            telemetry.track_login_method(LoginMethod::Manual);
            login_manual(base, force).await
        }
        None => {
            let mut login_telemetry = LoginTelemetry::new(&telemetry, LoginMethod::Standard);
            login_no_sso(base, force).await?;
            login_telemetry.set_success(true);
            Ok(())
        }
    }
}

async fn sso_login(base: &mut CommandBase, sso_team: &str, force: bool) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let color_config = base.color_config;
    let login_url_config = base.opts.api_client_opts.login_url.to_string();
    let sso_login_callback_port = base.opts.api_client_opts.sso_login_callback_port;
    let options = LoginOptions {
        existing_token: base.opts.api_client_opts.token.as_deref(),
        sso_team: Some(sso_team),
        force,
        sso_login_callback_port,
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

    write_token(base, token)
}

async fn login_no_sso(base: &mut CommandBase, force: bool) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let color_config = base.color_config;
    let login_url_config = base.opts.api_client_opts.login_url.to_string();
    let existing_token = base.opts.api_client_opts.token.as_deref();
    let sso_login_callback_port = base.opts.api_client_opts.sso_login_callback_port;

    let options = LoginOptions {
        existing_token,
        force,
        sso_login_callback_port,
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

    write_token(base, token)
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

// Writes a given token to the global turbo configuration file
fn write_token(base: &CommandBase, token: Token) -> Result<(), Error> {
    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string()
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?
        .unwrap_or_else(|| String::from("{}"));
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
