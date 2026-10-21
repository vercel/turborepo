mod manual;

use manual::login_manual;
use turborepo_api_client::APIClient;
use turborepo_auth::{
    AuthTokens, LoginOptions, TURBO_AUTH_FILE, TURBO_TOKEN_DIR, Token, TokenSet,
    login as auth_login, sso_login as auth_sso_login,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};
use turborepo_types::SecretString;

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
    SerdeJson(#[from] serde_json::Error),
    #[error(transparent)]
    TurboJsonParse(#[from] turborepo_turbo_json::parser::BiomeParseError),
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
    let login_url_source = base.opts.api_client_opts.login_url_source;
    let api_url_source = base.opts.api_client_opts.api_url_source;
    let sso_login_callback_port = base.opts.api_client_opts.sso_login_callback_port;
    let options = LoginOptions {
        existing_token: base.opts.api_client_opts.token.as_ref(),
        sso_team: Some(sso_team),
        force,
        login_url_source,
        api_url_source,
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
    let login_url_source = base.opts.api_client_opts.login_url_source;
    let api_url_source = base.opts.api_client_opts.api_url_source;
    let existing_token = base.opts.api_client_opts.token.as_ref();
    let linked_team_id = base.opts.api_client_opts.team_id.as_deref();
    let linked_team_slug = base.opts.api_client_opts.team_slug.as_deref();

    let options = LoginOptions {
        existing_token,
        force,
        login_url_source,
        api_url_source,
        linked_team_id,
        linked_team_slug,
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

fn write_token(
    base: &CommandBase,
    token: Token,
    token_set: Option<&TokenSet>,
) -> Result<(), Error> {
    let global_config_path = base.global_config_path()?;
    let Some(config_dir) =
        turborepo_dirs::config_dir().map_err(|_| crate::config::Error::NoGlobalConfigPath)?
    else {
        return Err(crate::config::Error::NoGlobalConfigPath.into());
    };
    let turbo_auth_path = config_dir.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
    write_token_to_paths(&global_config_path, &turbo_auth_path, token, token_set)
}

fn write_token_to_paths(
    global_config_path: &turbopath::AbsoluteSystemPath,
    turbo_auth_path: &turbopath::AbsoluteSystemPath,
    token: Token,
    token_set: Option<&TokenSet>,
) -> Result<(), Error> {
    let token = token.into_inner().clone();
    let auth_tokens = match token_set {
        Some(ts) => {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs());
            AuthTokens {
                token: Some(token),
                refresh_token: ts
                    .refresh_token
                    .as_ref()
                    .map(|rt| SecretString::new(rt.clone())),
                expires_at: Some(now_secs + ts.expires_in),
            }
        }
        None => AuthTokens {
            token: Some(token),
            refresh_token: None,
            expires_at: None,
        },
    };

    if auth_tokens
        .token
        .as_ref()
        .is_some_and(|token| token.is_saved_vercel())
    {
        auth_tokens.write_to_config_file(turbo_auth_path)?;
        return Ok(());
    }

    auth_tokens.write_to_config_file(global_config_path)?;
    AuthTokens::clear_from_config_file(turbo_auth_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{env, sync::Mutex};

    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_run_opts::Opts;
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::config::TurborepoConfigBuilder;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn create_command_base(repo_root: AbsoluteSystemPathBuf) -> CommandBase {
        let config = TurborepoConfigBuilder::new(&repo_root).build().unwrap();
        let opts = Opts::new(&repo_root, &Default::default(), &Default::default(), config).unwrap();

        CommandBase::from_opts(opts, repo_root, "test-version", ColorConfig::new(false))
    }

    #[test]
    fn login_persistence_uses_completed_flow_origin_not_token_prefix() {
        use turborepo_types::Credential;
        for secret in ["synthetic-legacy-sentinel", "vca_synthetic-oauth-sentinel"] {
            for vercel_flow in [false, true] {
                let tmp = tempdir().unwrap();
                let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
                let config_path = root.join_component("config.json");
                let auth_path = root.join_component("auth.json");
                config_path.create_with_contents(r#"{"apiUrl":"https://custom.example","token":"existing-custom","unrelated":true}"#).unwrap();
                let credential = if vercel_flow {
                    Credential::saved_vercel(secret.into())
                } else {
                    Credential::generic(secret.into())
                };
                write_token_to_paths(&config_path, &auth_path, Token::New(credential), None)
                    .unwrap();
                let target = if vercel_flow {
                    &auth_path
                } else {
                    &config_path
                };
                let other = if vercel_flow {
                    &config_path
                } else {
                    &auth_path
                };
                assert_eq!(
                    Token::from_file(target).unwrap().into_inner().expose(),
                    secret
                );
                if vercel_flow {
                    assert_eq!(
                        Token::from_file(other).unwrap().into_inner().expose(),
                        "existing-custom"
                    );
                } else {
                    assert!(matches!(
                        Token::from_file(other),
                        Err(turborepo_auth::Error::TokenNotFound)
                    ));
                }
                let global: serde_json::Value =
                    serde_json::from_str(&config_path.read_to_string().unwrap()).unwrap();
                assert_eq!(global["unrelated"], true);
                assert_eq!(global["apiUrl"], "https://custom.example");
                let persisted: serde_json::Value =
                    serde_json::from_str(&target.read_to_string().unwrap()).unwrap();
                assert!(persisted.get("issuer").is_none());
                let inputs = crate::config::ConfigurationFileInputs {
                    global_config_path: config_path.clone(),
                    global_auth_path: auth_path.clone(),
                    legacy_auth_path: None,
                };
                let config = TurborepoConfigBuilder::new(&root)
                    .build_with_inputs(Default::default(), inputs.clone())
                    .unwrap();
                assert_eq!(config.api_url(), "https://custom.example");
                assert_eq!(
                    config.token(),
                    Some(if vercel_flow {
                        "existing-custom"
                    } else {
                        secret
                    })
                );
                assert!(!config.credential().unwrap().is_saved_vercel());
                let vercel = TurborepoConfigBuilder::new(&root)
                    .with_api_url(Some(crate::config::DEFAULT_API_URL.into()))
                    .build_with_inputs(Default::default(), inputs)
                    .unwrap();
                assert_eq!(vercel.token(), Some(secret));
                assert_eq!(vercel.credential().unwrap().is_saved_vercel(), vercel_flow);
            }
        }
    }

    #[test]
    fn write_token_clears_stale_turbo_auth_for_manual_tokens() {
        let _lock = ENV_LOCK.lock().expect("env lock poisoned");
        let repo_root = tempdir().expect("failed to create repo tempdir");
        let config_root = tempdir().expect("failed to create config tempdir");
        let repo_root = AbsoluteSystemPathBuf::try_from(repo_root.path().to_path_buf())
            .expect("failed to create repo path");
        let config_root = AbsoluteSystemPathBuf::try_from(config_root.path().to_path_buf())
            .expect("failed to create config path");

        repo_root
            .join_component("turbo.json")
            .create_with_contents("{}")
            .expect("failed to write turbo.json");
        repo_root
            .join_component("package.json")
            .create_with_contents("{}")
            .expect("failed to write package.json");

        let turbo_auth_path = config_root.join_components(&[TURBO_TOKEN_DIR, TURBO_AUTH_FILE]);
        AuthTokens {
            token: Some(turborepo_types::Credential::saved_vercel(
                "vca_stale_token".into(),
            )),
            refresh_token: Some(SecretString::new("stale_refresh_token".to_owned())),
            expires_at: Some(4_102_444_800),
        }
        .write_to_config_file(&turbo_auth_path)
        .expect("failed to write stale turbo auth file");

        unsafe {
            env::set_var("TURBO_CONFIG_DIR_PATH", config_root.as_path());
        }

        let base = create_command_base(repo_root);
        write_token(&base, Token::new("manual_token".to_owned()), None)
            .expect("failed to write manual token");

        let global_config_path = config_root.join_components(&[TURBO_TOKEN_DIR, "config.json"]);
        let written_token = Token::from_file(&global_config_path)
            .expect("manual token should be written to config.json");
        assert_eq!(written_token.into_inner().expose(), "manual_token");

        let auth_tokens = Token::from_auth_file(&turbo_auth_path)
            .expect("auth.json should remain parseable after clearing");
        assert!(auth_tokens.token.is_none());
        assert!(auth_tokens.refresh_token.is_none());
        assert!(auth_tokens.expires_at.is_none());

        unsafe {
            env::remove_var("TURBO_CONFIG_DIR_PATH");
        }
    }
}
