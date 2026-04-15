mod manual;

use manual::login_manual;
use turborepo_api_client::APIClient;
use turborepo_auth::{
    login as auth_login, sso_login as auth_sso_login, AuthTokens, LoginOptions, Token, TokenSet,
    TURBO_AUTH_FILE, TURBO_TOKEN_DIR,
};
use turborepo_json_rewrite::set_path;
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};

use crate::{commands::CommandBase, config};

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

/// Writes a token to disk. Device-flow OAuth tokens go into Turbo's auth.json
/// so older Turbos never treat them as legacy API tokens.
fn write_token(
    base: &CommandBase,
    token: Token,
    token_set: Option<&TokenSet>,
) -> Result<(), Error> {
    let token_str = token.into_inner().expose().to_string();

    // Keep OAuth sessions in a Turbo-owned auth file so older Turbos don't try
    // to validate them via legacy token endpoints.
    if let Some(ts) = token_set {
        if let Ok(Some(config_dir)) = turborepo_dirs::config_dir() {
            let auth_path = config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs();
            let auth_tokens = AuthTokens {
                token: Some(turborepo_api_client::SecretString::new(
                    ts.access_token.clone(),
                )),
                refresh_token: ts
                    .refresh_token
                    .as_ref()
                    .map(|rt| turborepo_api_client::SecretString::new(rt.clone())),
                expires_at: Some(now_secs + ts.expires_in),
            };
            if let Err(e) = auth_tokens.write_to_auth_file(&auth_path) {
                tracing::warn!(
                    "Failed to write Turbo auth.json at {auth_path}: {e}. Login succeeded but \
                     Turbo will need to re-authenticate next time."
                );
            }
        }
    }

    if token_str.starts_with("vca_") {
        return Ok(());
    }

    // Legacy API tokens stay in turborepo/config.json for existing releases.
    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string()
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?
        .unwrap_or_else(|| String::from("{}"));
    let after = set_path(&before, &["token"], &format!("\"{token_str}\""))?;

    global_config_path
        .ensure_dir()
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    global_config_path
        .create_with_contents_secret(after)
        .map_err(|e| config::Error::FailedToSetConfig {
            config_path: global_config_path.clone(),
            error: e,
        })?;

    Ok(())
}
