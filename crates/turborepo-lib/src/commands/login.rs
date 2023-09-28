use anyhow::Result;
use turborepo_api_client::APIClient;
use turborepo_auth::{login as auth_login, sso_login as auth_sso_login};
use turborepo_ui::{BOLD, CYAN, UI};

use crate::commands::CommandBase;

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

pub async fn sso_login(base: &mut CommandBase, sso_team: &str) -> Result<()> {
    let ui = base.ui;
    // Check if token exists first. Must be there for the user and contain the team
    // passed into this function.
    if let Some(token) = base.user_config()?.token() {
        let client_user: APIClient = base.api_client()?;
        let client_teams: APIClient = base.api_client()?;

        let (result_user, result_teams) =
            tokio::join!(client_user.get_user(token), client_teams.get_teams(token));

        if let (Ok(response_user), Ok(response_teams)) = (result_user, result_teams) {
            if response_teams
                .teams
                .iter()
                .any(|team| team.slug == sso_team)
            {
                println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
                print_cli_authorized(&response_user.user.email, &ui);
                return Ok(());
            }
        }
    }
    let api_client: APIClient = base.api_client()?;
    let login_url_config = base.repo_config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        Ok(base.user_config_mut()?.set_token(Some(token.to_string()))?)
    };

    auth_sso_login(api_client, &ui, set_token, &login_url_config, sso_team).await
}

pub async fn login(base: &mut CommandBase) -> Result<()> {
    let api_client: APIClient = base.api_client()?;
    let ui = base.ui;
    // Check if token exists first. Must be there for the user and contain the team
    // passed into this function.
    if let Some(token) = base.user_config()?.token() {
        if let Ok(response) = api_client.get_user(token).await {
            println!("{}", ui.apply(BOLD.apply_to("Existing token found!")));
            print_cli_authorized(&response.user.email, &ui);
            return Ok(());
        }
    }

    let login_url_config = base.repo_config()?.login_url().to_string();

    // We are passing a closure here, but it would be cleaner if we made a
    // turborepo-config crate and imported that into turborepo-auth.
    let set_token = |token: &str| -> Result<(), anyhow::Error> {
        Ok(base.user_config_mut()?.set_token(Some(token.to_string()))?)
    };

    auth_login(api_client, &ui, set_token, &login_url_config).await
}
