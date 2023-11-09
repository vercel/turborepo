use tracing::error;
use turborepo_auth::logout as auth_logout;

use crate::{cli::Error, commands::CommandBase, config, rewrite_json::unset_path};

pub fn logout(base: &mut CommandBase) -> Result<(), Error> {
    if let Err(err) = remove_token(base) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    auth_logout(&base.ui);

    Ok(())
}

fn remove_token(base: &mut CommandBase) -> Result<(), Error> {
    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|e| {
            Error::Config(config::Error::FailedToReadConfig {
                config_path: global_config_path.clone(),
                error: e,
            })
        })?;

    if let Some(after) = unset_path(&before, &["token"], true)? {
        global_config_path.create_with_contents(after).map_err(|e| {
            Error::Config(config::Error::FailedToSetConfig {
                config_path: global_config_path.clone(),
                error: e,
            })
        })
    } else {
        Ok(())
    }
}
