use anyhow::Result;
use log::error;

use crate::{
    config::{default_user_config_path, UserConfig},
    ui::{GREY, UI},
};

pub fn logout(ui: UI) -> Result<()> {
    let mut config = UserConfig::load(
        &default_user_config_path()
            .context("Unable to find default config path")?,
        None,
    )?;

    if let Err(err) = config.set_token(None) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    ui.info(">>> Logged out", Some(&GREY))?;
    Ok(())
}
