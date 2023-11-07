use turborepo_api_client::{APIClient, Client};
use turborepo_auth::{
    login as auth_login, read_or_create_auth_file, sso_login as auth_sso_login, DefaultLoginServer,
    DefaultSSOLoginServer,
};
use turborepo_ui::{BOLD, CYAN, UI};

use crate::{cli::Error, commands::CommandBase, config, rewrite_json::set_path};

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
        .map_err(|e| config::Error::FailedToReadAuth {
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

/// Entry point for `turbo login`. Checks for the existence of an auth file
/// token that matches the API base URL, and if we already have a token for it,
/// returns that one instead of fetching a new one. Otherwise, fetches a new
/// token and writes it to `auth.json` in the Turbo config directory.
pub async fn login(base: &mut CommandBase) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    // Get both possible token paths for existing token(s) checks.
    let global_auth_path = base.global_auth_path()?;
    let global_config_path = base.global_config_path()?;

    /*
     * We need to do the following:
     * 1) Read in an existing auth file if it exists.
     * 2) Check if the token exists in the auth file.
     * 3) If it does, return early, since the user is already logged in.
     * 4) If we can't find the auth file, check the config file for a token.
     * 5) If we find a token in the config file, convert it to an auth file.
     * 6) If we can't find a token in the config file, create a new auth file.
     */
    let mut auth_file =
        read_or_create_auth_file(&global_auth_path, &global_config_path, &api_client).await?;

    // TODO(voz): Do we need to do additional checks for token existence? Things
    // like user, team, etc?
    if let Some(token) = auth_file.get_token(api_client.base_url()) {
        // Token already exists, return early.
        println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
        print_cli_authorized(&token.token, &ui);
        return Ok(());
    }

    // Get the token from the login server.
    let token = auth_login(&api_client, &ui, &login_url_config, &DefaultLoginServer).await?;

    auth_file.add_or_update_token(token);
    auth_file.write_to_disk(&global_auth_path)?;

    Ok(())
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
    use camino::Utf8PathBuf;
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_auth::{
        mocks::MockApiClient, read_or_create_auth_file, AuthFile, AuthToken, Space, Team,
        TURBOREPO_AUTH_FILE_NAME,
    };

    use crate::{
        cli::Verbosity,
        commands::{login::login, CommandBase},
        Args,
    };

    fn setup_base() -> CommandBase {
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();

        let cwd = Utf8PathBuf::from_path_buf(temp_dir.path().to_path_buf())
            .expect("Failed to create cwd");

        let args = Args {
            version: false,
            skip_infer: false,
            no_update_notifier: false,
            api: Some("mock-api".to_string()),
            color: false,
            cpu_profile: None,
            cwd: Some(cwd),
            heap: None,
            login: None,
            no_color: true,
            preflight: false,
            remote_cache_timeout: None,
            team: None,
            token: None,
            trace: None,
            verbosity: Verbosity {
                verbosity: Some(0),
                v: 0,
            },
            check_for_update: false,
            test_run: false,
            run_args: None,
            command: None,
        };
        let repo_root = AbsoluteSystemPathBuf::try_from(temp_dir.path().to_path_buf()).unwrap();
        let ui = turborepo_ui::UI::new(false);

        CommandBase::new(args, repo_root, "0.0.0", ui)
            .with_global_auth_path(auth_file_path.clone())
            .with_global_config_path(auth_file_path.clone())
    }

    #[tokio::test]
    async fn test_login_with_existing_token() {
        // Setup: Test dirs and mocks.
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();
        // Mock out the existing file.
        let mock_auth_file = AuthFile {
            tokens: vec![AuthToken {
                token: "mock-token".to_string(),
                api: "mock-api".to_string(),
                created_at: Some(0),
                user: turborepo_vercel_api::User {
                    id: 0.to_string(),
                    email: "mock-email".to_string(),
                    username: "mock-username".to_string(),
                    name: Some("mock-name".to_string()),
                    created_at: Some(0),
                },
                teams: vec![Team {
                    id: "team-id".to_string(),
                    spaces: vec![Space {
                        id: "space-id".to_string(),
                    }],
                }],
            }],
        };
        mock_auth_file.write_to_disk(&auth_file_path).unwrap();

        let mock_api_client = MockApiClient::new();

        let mut base = setup_base();

        // Test: Call login function and see if we got the existing token on the FS
        // back.
        let result = login(&mut base).await;
        assert!(result.is_ok());

        // Since we don't return anything if the login found an existing token, we
        // should read the FS for the auth token. Whatever we get back should be the
        // same as the mock auth file.
        // Pass in the auth file path for both possible paths becuase we should never
        // read the config from here.
        let found_auth_file =
            read_or_create_auth_file(&auth_file_path, &auth_file_path, &mock_api_client)
                .await
                .unwrap();
        assert_eq!(
            mock_auth_file.get_token("mock-api"),
            found_auth_file.get_token("mock-api")
        )
    }

    #[tokio::test]
    async fn test_login_no_existing_token() {
        // Setup: Test dirs and mocks.
        let temp_dir = tempfile::tempdir().unwrap();
        let auth_file_path =
            AbsoluteSystemPathBuf::try_from(temp_dir.path().join(TURBOREPO_AUTH_FILE_NAME))
                .unwrap();

        let mock_api_client = MockApiClient::new();

        let mut base = setup_base();
        let result = login(&mut base).await;
        assert!(result.is_ok());

        let found_auth_file =
            read_or_create_auth_file(&auth_file_path, &auth_file_path, &mock_api_client)
                .await
                .unwrap();

        assert_eq!(found_auth_file.tokens.len(), 1);
    }
}
