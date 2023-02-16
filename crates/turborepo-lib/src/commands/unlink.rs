use anyhow::{Context, Result};

use crate::{commands::CommandBase, ui::GREY};

pub fn unlink(base: &mut CommandBase) -> Result<()> {
    base.delete_repo_config_file()
        .context("could not unlink. Something went wrong")?;

    println!(
        "{}",
        base.ui.apply(GREY.apply_to("> Disabled Remote Caching"))
    );

    Ok(())
}
