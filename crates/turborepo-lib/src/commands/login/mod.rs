mod manual;

use manual::login_manual;
use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, AuthTokens, LoginOptions, Token, TokenSet,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};

use crate::commands::CommandBase;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to read user input. {0}")]
    UserInput(#[from] dialoguer::Error),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    Auth(#[from] turborepo_auth::Error),
    #[error("Unable to edit `turbo.json`. {0}")]
    JsonEdit(#[from] turborepo_json_rewrite::RewriteError),
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
        existing_token: base.opts.api_client_opts.token.as_ref().map(|t| t.expose()),
        sso_team: Some(sso_team),
        force,
        sso_login_callback_port,
        ..LoginOptions::new(&color_config, &login_url_config, &api_client)
    };

    let (token, token_set) = auth_sso_login(&options).await?;

    if matches!(token, Token::Existing(..)) {
        return Ok(());
    }

    write_token(base, token, token_set.as_ref())
}

async fn login_no_sso(base: &mut CommandBase, force: bool) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let color_config = base.color_config;
    let login_url_config = base.opts.api_client_opts.login_url.to_string();
    let existing_token = base.opts.api_client_opts.token.as_ref().map(|t| t.expose());

    let options = LoginOptions {
        existing_token,
        force,
        ..LoginOptions::new(&color_config, &login_url_config, &api_client)
    };

    let (token, token_set) = auth_login(&options).await?;

    if matches!(token, Token::Existing(..)) {
        return Ok(());
    }

    write_token(base, token, token_set.as_ref())
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
impl<'a> Drop for LoginTelemetry<'a> {
    fn drop(&mut self) {
        self.telemetry.track_login_method(self.method);
        self.telemetry.track_login_success(self.success);
    }
}

/// Writes a token to turborepo/config.json. If device-flow login returned
/// refresh metadata, persist that alongside the access token so Turbo can
/// refresh it without touching the Vercel CLI directory.
fn write_token(
    base: &CommandBase,
    token: Token,
    token_set: Option<&TokenSet>,
) -> Result<(), Error> {
    let global_config_path = base.global_config_path()?;
    let token = token.into_inner().clone();
    let auth_tokens = match token_set {
        Some(ts) => {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            AuthTokens {
                token: Some(token),
                refresh_token: ts
                    .refresh_token
                    .as_ref()
                    .map(|rt| turborepo_api_client::SecretString::new(rt.clone())),
                expires_at: Some(now_secs + ts.expires_in),
            }
        }
        None => AuthTokens {
            token: Some(token),
            refresh_token: None,
            expires_at: None,
        },
    };

    auth_tokens.write_to_config_file(&global_config_path)?;

    Ok(())
}
