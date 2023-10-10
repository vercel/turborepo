use anyhow::{anyhow, Error, Result};
use turborepo_auth::logout as auth_logout;

use crate::{commands::CommandBase, rewrite_json::unset_path};

pub fn logout(base: &mut CommandBase) -> Result<()> {
    let ui = base.ui;

    // Passing a closure here while we figure out how to make turborepo-auth
    // crate manage its own configuration for the path to the token.
    let set_token = || -> Result<(), Error> {
        let global_config_path = base.global_config_path()?;
        let before = global_config_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| {
                anyhow!(
                    "Encountered an IO error while attempting to read {}: {}",
                    global_config_path,
                    e
                )
            })?;

        if let Some(after) = unset_path(&before, &["token"], true)? {
            global_config_path
                .create_with_contents(after)
                .map_err(Error::from)
        } else {
            Ok(())
        }
    };

    auth_logout(&ui, set_token)
}
