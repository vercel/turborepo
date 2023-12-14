use tracing::error;
use turborepo_auth::{logout as auth_logout, read_or_create_auth_file, AuthToken};

use crate::{cli::Error, commands::CommandBase};

pub async fn logout(base: &mut CommandBase) -> Result<(), Error> {
    let client = base.api_client()?;
    let auth_path = base.global_auth_path()?;
    let config_path = base.global_config_path()?;
    let mut auth_file = read_or_create_auth_file(&auth_path, &config_path, &client).await?;

    if auth_file.tokens().is_empty() {
        println!("No tokens to remove");
        return Ok(());
    }

    // Don't prompt when there's only one token to logout for.
    if auth_file.tokens().len() <= 1 {
        auth_file.tokens_mut().clear();
    } else {
        // Make a friendly display for the user to select from.
        let items = &auth_file
            .tokens()
            .iter()
            .map(|t| {
                let token = AuthToken {
                    api: t.0.to_string(),
                    token: t.1.to_string(),
                };
                token.friendly_api_display().to_string()
            })
            .collect::<Vec<_>>();

        let index = base
            .ui
            .display_selectable_items("Select api to log out of:", items)
            .unwrap();

        // Remove the token display from the api we're trying to remove.
        let api = items[index].split_whitespace().next().unwrap();

        if let Some(token) = auth_file.get_token(api) {
            println!("Removing token for {}", token.friendly_api_display());
            auth_file.remove(api);
        } else {
            // This should never happen, but just in case.
            return Err(Error::Auth(turborepo_auth::Error::FailedToGetToken));
        }
    }

    if let Err(err) = auth_file.write_to_disk(&auth_path) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(Error::Auth(err));
    }

    auth_logout(&base.ui);

    Ok(())
}
