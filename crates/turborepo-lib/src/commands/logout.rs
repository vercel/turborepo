use anyhow::Result;
use turborepo_auth::logout as auth_logout;

use crate::commands::CommandBase;

pub fn logout(base: &mut CommandBase) -> Result<()> {
    let ui = base.ui;

    // Passing a closure here while we figure out how to make turborepo-auth
    // crate manage its own configuration for the path to the token.
    let set_token =
        || -> Result<(), anyhow::Error> { Ok(base.user_config_mut()?.set_token(None)?) };

    auth_logout(&ui, set_token)
}
