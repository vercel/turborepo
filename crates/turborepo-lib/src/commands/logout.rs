use anyhow::Result;
use turborepo_ui::GREY;

use crate::{commands::CommandBase, rewrite_json::unset_path};

pub fn logout(base: &mut CommandBase) -> Result<()> {
    let before = base.global_config_path()?.read_to_string()?;
    if let Some(after) = unset_path(&before, &["token"])? {
        base.global_config_path()?.create_with_contents(after)?;
    }

    println!("{}", base.ui.apply(GREY.apply_to(">>> Logged out")));
    Ok(())
}
