use turborepo_api_client::{APIClient, Client};
use turborepo_auth::{
    login as auth_login, read_or_create_auth_file, sso_login as auth_sso_login, DefaultLoginServer,
    DefaultSSOLoginServer,
};
use turborepo_ui::{BOLD, CYAN, UI};
use turborepo_vercel_api::TokenMetadata;

use crate::{cli::Error, commands::CommandBase};

/// Entry point for `turbo login --sso`.
pub async fn sso_login(base: &mut CommandBase, sso_team: &str) -> Result<(), Error> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    let login_url_config = base.config()?.login_url().to_string();

    // Get both possible token paths for existing token(s) checks.
    let global_auth_path = base.global_auth_path()?;
    let global_config_path = base.global_config_path()?;

    let mut auth_file =
        read_or_create_auth_file(&global_auth_path, &global_config_path, &api_client).await?;

    // Check if there's an existing token.
    if let Some(token) = auth_file.get_token(api_client.base_url()) {
        let metadata: TokenMetadata = api_client.get_token_metadata(&token.token).await?;
        let user_response = api_client.get_user(&token.token).await?;
        // We get all teams here and do a filter because there's an issue where certain
        // teams cause servers to go into a 508 loop.
        let teams = api_client.get_teams(&token.token).await?;

        if metadata.origin == "saml" && teams.teams.iter().any(|team| team.slug == sso_team) {
            // Token already exists, return early.
            println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
            print_cli_authorized(&user_response.user.username, &ui);
            return Ok(());
        }
    }

    let auth_token = auth_sso_login(
        &api_client,
        &ui,
        &login_url_config,
        sso_team,
        &DefaultSSOLoginServer,
    )
    .await?;

    auth_file.add_or_update_token(api_client.base_url().to_owned(), auth_token.token);
    auth_file.write_to_disk(&global_auth_path)?;

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

    let mut auth_file =
        read_or_create_auth_file(&global_auth_path, &global_config_path, &api_client).await?;

    // We might not have expiration on tokens, so checking that is iffy. Just make
    // sure it's a non-SAML token.
    if let Some(token) = auth_file.get_token(api_client.base_url()) {
        // Non-SAML tokens have an origin of "manual", so we use that to make sure
        // existing token is correctly scoped.
        let metadata: TokenMetadata = api_client.get_token_metadata(&token.token).await?;
        if metadata.origin == "manual" {
            println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
            // TODO(voz): Print out the user that the token is for instead of token itself.
            print_cli_authorized(&token.token, &ui);
            return Ok(());
        }
    }

    let auth_token = auth_login(&api_client, &ui, &login_url_config, &DefaultLoginServer).await?;

    auth_file.add_or_update_token(api_client.base_url().to_owned(), auth_token.token);
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
    use turborepo_auth::TURBOREPO_AUTH_FILE_NAME;

    use crate::{cli::Verbosity, commands::CommandBase, Args};

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

    // #[tokio::test]
    // async fn test_login_with_existing_token() {
    //     // Setup: Test dirs and mocks.
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let auth_file_path =
    //         AbsoluteSystemPathBuf::try_from(temp_dir.path().
    // join(TURBOREPO_AUTH_FILE_NAME))             .unwrap();
    //     // Mock out the existing file.
    //     let mock_auth_file = AuthFile {
    //         tokens: vec![AuthToken {
    //             token: "mock-token".to_string(),
    //             api: "mock-api".to_string(),
    //             // created_at: Some(0),
    //             // user: turborepo_vercel_api::User {
    //             //     id: 0.to_string(),
    //             //     email: "mock-email".to_string(),
    //             //     username: "mock-username".to_string(),
    //             //     name: Some("mock-name".to_string()),
    //             //     created_at: Some(0),
    //             // },
    //             // teams: vec![Team {
    //             //     id: "team-id".to_string(),
    //             //     spaces: vec![Space {
    //             //         id: "space-id".to_string(),
    //             //         name: "space1 name".to_string(),
    //             //     }],
    //             //     slug: "team slug".to_string(),
    //             //     name: "team name".to_string(),
    //             //     created_at: 0,
    //             //     created: chrono::Utc::now(),
    //             //     membership:
    // Membership::new(turborepo_vercel_api::Role::Developer),             //
    // }],         }],
    //     };
    //     mock_auth_file.write_to_disk(&auth_file_path).unwrap();

    //     let mock_api_client = MockApiClient::new();

    //     let mut base = setup_base();

    //     // Test: Call login function and see if we got the existing token on
    // the FS     // back.
    //     let result = login(&mut base).await;
    //     assert!(result.is_ok());

    //     // Since we don't return anything if the login found an existing
    // token, we     // should read the FS for the auth token. Whatever we
    // get back should be the     // same as the mock auth file.
    //     // Pass in the auth file path for both possible paths becuase we
    // should never     // read the config from here.
    //     let found_auth_file =
    //         read_or_create_auth_file(&auth_file_path, &auth_file_path,
    // &mock_api_client)             .await
    //             .unwrap();
    //     assert_eq!(
    //         mock_auth_file.get_token("mock-api"),
    //         found_auth_file.get_token("mock-api")
    //     )
    // }
    //     #[tokio::test]
    //     async fn test_login_no_existing_token() {
    //         // Setup: Test dirs and mocks.
    //         let temp_dir = tempfile::tempdir().unwrap();
    //         let auth_file_path =
    //
    // AbsoluteSystemPathBuf::try_from(temp_dir.path().
    // join(TURBOREPO_AUTH_FILE_NAME))                 .unwrap();

    //         let mock_api_client = MockApiClient::new();

    //         let mut base = setup_base();
    //         let result = login(&mut base).await;
    //         assert!(result.is_ok());

    //         let found_auth_file =
    //             read_or_create_auth_file(&auth_file_path, &auth_file_path,
    // &mock_api_client)                 .await
    //                 .unwrap();

    //         assert_eq!(found_auth_file.tokens.len(), 1);
    //     }
}
