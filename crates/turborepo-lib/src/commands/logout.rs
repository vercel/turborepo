use anyhow::Result;
use log::error;

use crate::{
    config::{default_user_config_path, UserConfigLoader},
    ui::{GREY, UI},
};

pub fn logout(ui: UI) -> Result<()> {
    let mut config = UserConfigLoader::new(default_user_config_path()?).load()?;

    if let Err(err) = config.set_token(None) {
        error!("could not logout. Something went wrong: {}", err);
        return Err(err);
    }

    println!("{}", ui.apply(GREY.apply_to(">>> Logged out")));
    Ok(())
}
