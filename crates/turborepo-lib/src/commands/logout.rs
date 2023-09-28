use anyhow::{anyhow, Result};
use turborepo_ui::GREY;

use crate::{commands::CommandBase, rewrite_json::unset_path};

pub fn logout(base: &mut CommandBase) -> Result<()> {
    let global_config_path = base.global_config_path()?;
    let before = global_config_path
        .read_or_default("{}".into())
        .map_err(|e| {
            anyhow!(
                "Encountered an IO error while attempting to read {}: {}",
                global_config_path,
                e
            )
        })?;
    let output = if let Some(after) = unset_path(&before, &["token"], true)? {
        global_config_path.create_with_contents(after)?;
        ">>> Logged out"
    } else {
        ">>> Not logged in"
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));
    Ok(())
}
