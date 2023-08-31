use anyhow::{anyhow, Result};
use turborepo_ui::GREY;

use crate::{commands::CommandBase, rewrite_json::unset_path};

pub fn logout(base: &mut CommandBase) -> Result<()> {
    let before = base.global_config_path()?.read_to_string().or_else(|e| {
        if matches!(e.kind(), std::io::ErrorKind::NotFound) {
            Ok(String::from("{}"))
        } else {
            dbg!(e);
            Err(anyhow!("logout"))
        }
    })?;
    let output = if let Some(after) = unset_path(&before, &["token"], true)? {
        base.global_config_path()?.create_with_contents(after)?;
        ">>> Logged out"
    } else {
        ">>> Not logged in"
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));
    Ok(())
}
