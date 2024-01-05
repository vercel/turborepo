use turborepo_api_client::{APIClient, Client};
use turborepo_auth::{
    login as auth_login, read_or_create_auth_file, sso_login as auth_sso_login, AuthFile,
    DefaultLoginServer, DefaultSSOLoginServer, LoginServer, SSOLoginServer,
};
use turborepo_telemetry::events::command::{CommandEventBuilder, LoginMethod};
use turborepo_ui::{BOLD, CYAN, UI};
use turborepo_vercel_api::TokenMetadata;

use crate::{cli::Error, commands::CommandBase};

pub struct Login<T> {
    pub(crate) server: T,
}

impl<T: LoginServer> Login<T> {
    pub(crate) async fn login(
        &self,
        base: &mut CommandBase,
        telemetry: CommandEventBuilder,
    ) -> Result<(), Error> {
        telemetry.track_login_method(LoginMethod::Standard);
        let api_client: APIClient = base.api_client()?;
        let ui = base.ui;
        let login_url_config = base.config()?.login_url().to_string();

        // Get both possible token paths for existing token(s) checks.
        let global_auth_path = base.global_auth_path()?;
        let global_config_path = base.global_config_path()?;

        let mut auth_file = read_or_create_auth_file(
            &global_auth_path,
            &global_config_path,
            api_client.base_url(),
        )?;

        if self
            .has_existing_token(&api_client, &mut auth_file, &ui)
            .await?
        {
            return Ok(());
        }

        let login_server = &self.server;

        let auth_token = auth_login(&api_client, &ui, &login_url_config, login_server).await?;

        auth_file.insert(api_client.base_url().to_owned(), auth_token.token);
        auth_file.write_to_disk(&global_auth_path)?;
        Ok(())
    }

    async fn has_existing_token(
        &self,
        api_client: &APIClient,
        auth_file: &mut AuthFile,
        ui: &UI,
    ) -> Result<bool, Error> {
        if let Some(token) = auth_file.get_token(api_client.base_url()) {
            let metadata: TokenMetadata = api_client.get_token_metadata(&token.token).await?;
            let user_response = api_client.get_user(&token.token).await?;
            if metadata.origin == "manual" {
                println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
                print_cli_authorized(&user_response.user.username, ui);
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl<T: SSOLoginServer> Login<T> {
    pub(crate) async fn sso_login(
        &self,
        base: &mut CommandBase,
        sso_team: &str,
        telemetry: CommandEventBuilder,
    ) -> Result<(), Error> {
        telemetry.track_login_method(LoginMethod::SSO);
        let api_client: APIClient = base.api_client()?;
        let ui = base.ui;
        let login_url_config = base.config()?.login_url().to_string();

        // Get both possible token paths for existing token(s) checks.
        let global_auth_path = base.global_auth_path()?;
        let global_config_path = base.global_config_path()?;

        let mut auth_file = read_or_create_auth_file(
            &global_auth_path,
            &global_config_path,
            api_client.base_url(),
        )?;

        if self
            .has_existing_sso_token(&api_client, &mut auth_file, &base.ui, sso_team)
            .await?
        {
            return Ok(());
        }

        let login_server = &self.server;

        let auth_token =
            auth_sso_login(&api_client, &ui, &login_url_config, sso_team, login_server).await?;

        auth_file.insert(api_client.base_url().to_owned(), auth_token.token);
        auth_file.write_to_disk(&global_auth_path)?;

        Ok(())
    }

    async fn has_existing_sso_token(
        &self,
        api_client: &APIClient,
        auth_file: &mut AuthFile,
        ui: &UI,
        sso_team: &str,
    ) -> Result<bool, Error> {
        if let Some(token) = auth_file.get_token(api_client.base_url()) {
            let metadata: TokenMetadata = api_client.get_token_metadata(&token.token).await?;
            let user_response = api_client.get_user(&token.token).await?;
            let teams = api_client.get_teams(&token.token).await?;
            if metadata.origin == "saml" && teams.teams.iter().any(|team| team.slug == sso_team) {
                println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
                print_cli_authorized(&user_response.user.username, ui);
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Entry point for `turbo login --sso-team`.
pub async fn sso_login(
    base: &mut CommandBase,
    sso_team: &str,
    telemetry: CommandEventBuilder,
) -> Result<(), Error> {
    Login {
        server: DefaultSSOLoginServer {},
    }
    .sso_login(base, sso_team, telemetry)
    .await
}

/// Entry point for `turbo login`. Checks for the existence of an auth file
/// token that matches the API base URL, and if we already have a token for it,
/// returns that one instead of fetching a new one. Otherwise, fetches a new
/// token and writes it to `auth.json` in the Turbo config directory.
pub async fn login(base: &mut CommandBase, telemetry: CommandEventBuilder) -> Result<(), Error> {
    Login {
        server: DefaultLoginServer {},
    }
    .login(base, telemetry)
    .await
}

fn print_cli_authorized(user: &str, ui: &UI) {
    println!(
        "
{} Turborepo CLI authorized for {}
{}
{}
",
        ui.rainbow(">>> Success!"),
        user,
        ui.apply(
            CYAN.apply_to("To connect to your Remote Cache, run the following in any turborepo:")
        ),
        ui.apply(BOLD.apply_to("  npx turbo link"))
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use camino::Utf8PathBuf;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_auth::{mocks::*, AuthFile, TURBOREPO_AUTH_FILE_NAME};
    use turborepo_vercel_api_mock::start_test_server;

    use super::*;
    use crate::{commands::CommandBase, Args};

    fn setup_base(auth_path: &AbsoluteSystemPathBuf, port: u16) -> CommandBase {
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();

        let cwd = Utf8PathBuf::from_path_buf(temp_dir.path().to_path_buf())
            .expect("Failed to create cwd");

        let args = Args {
            api: Some(format!("http://localhost:{}", port)),
            cwd: Some(cwd),
            login: Some(format!("http://localhost:{}", port)),
            no_color: true,
            ..Default::default()
        };
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path().to_path_buf()).unwrap();
        let ui = turborepo_ui::UI::new(false);

        let mut base = CommandBase::new(args, repo_root, "0.0.0", ui)
            .with_global_auth_path(auth_file_path.clone())
            .with_global_config_path(auth_file_path.clone());

        base.config_init().unwrap();
        base.global_auth_path = Some(auth_path.clone());
        base
    }

    #[tokio::test]
    async fn test_login_with_existing_token() {
        // Setup: Test dirs and mocks.
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();
        // Mock out the existing file.
        let mut mock_auth_file = AuthFile::default();
        mock_auth_file.insert("mock-api".to_string(), "mock-token".to_string());
        mock_auth_file.write_to_disk(&auth_file_path).unwrap();

        let mock_api_client = MockApiClient::new();

        let mut base = setup_base(&auth_file_path, port);

        // Test: Call login function and see if we got the existing token on
        // the FS back.
        let login_with_mock_server = Login {
            server: MockLoginServer {
                hits: Arc::new(0.into()),
            },
        };
        let telemetry = CommandEventBuilder::new("login");
        let result = login_with_mock_server.login(&mut base, telemetry).await;
        assert!(result.is_ok());

        // Since we don't return anything if the login found an existing
        // token, we should read the FS for the auth token. Whatever we
        // get back should be the same as the mock auth file.
        // Pass in the auth file path for both possible paths becuase we
        // should never read the config from here.
        let found_auth_file =
            read_or_create_auth_file(&auth_file_path, &auth_file_path, mock_api_client.base_url())
                .unwrap();

        api_server.abort();
        assert_eq!(
            mock_auth_file.get_token("mock-api"),
            found_auth_file.get_token("mock-api")
        )
    }

    #[tokio::test]
    async fn test_login_no_existing_token() {
        // Setup: Test dirs and mocks.
        let port = port_scanner::request_open_port().unwrap();
        let api_server = tokio::spawn(start_test_server(port));
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();

        let mock_api_client = MockApiClient::new();

        let mut base = setup_base(&auth_file_path, port);

        // Test: Call login function and see if we got the expected token.
        let login_with_mock_server = Login {
            server: MockLoginServer {
                hits: Arc::new(0.into()),
            },
        };
        let telemetry = CommandEventBuilder::new("login");
        let result = login_with_mock_server.login(&mut base, telemetry).await;
        assert!(result.is_ok());

        let found_auth_file =
            read_or_create_auth_file(&auth_file_path, &auth_file_path, mock_api_client.base_url())
                .unwrap();

        api_server.abort();

        assert_eq!(found_auth_file.tokens().len(), 1);
    }
}
