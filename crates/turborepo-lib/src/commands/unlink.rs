use std::fs;

use anyhow::{Context, Result};
use turborepo_ui::GREY;

use crate::{cli::LinkTarget, commands::CommandBase, rewrite_json};

enum UnlinkSpacesResult {
    Unlinked,
    NoSpacesFound,
}

fn unlink_remote_caching(base: &mut CommandBase) -> Result<()> {
    base.delete_repo_config_file()
        .context("could not unlink. Something went wrong")?;

    println!(
        "{}",
        base.ui.apply(GREY.apply_to("> Disabled Remote Caching"))
    );

    Ok(())
}

fn unlink_spaces(base: &mut CommandBase) -> Result<()> {
    let result =
        remove_spaces_from_turbo_json(base).context("could not unlink. Something went wrong")?;

    match result {
        UnlinkSpacesResult::Unlinked => {
            println!("{}", base.ui.apply(GREY.apply_to("> Unlinked Spaces")));
        }
        UnlinkSpacesResult::NoSpacesFound => {
            println!(
                "{}",
                base.ui.apply(GREY.apply_to("> No Spaces config found"))
            );
        }
    }

    Ok(())
}

pub fn unlink(base: &mut CommandBase, target: LinkTarget) -> Result<()> {
    match target {
        LinkTarget::RemoteCache => {
            unlink_remote_caching(base)?;
        }
        LinkTarget::Spaces => {
            unlink_spaces(base)?;
        }
    }
    Ok(())
}

fn remove_spaces_from_turbo_json(base: &CommandBase) -> Result<UnlinkSpacesResult> {
    let turbo_json_path = base.repo_root.join_component("turbo.json");
    let turbo_json = fs::read_to_string(&turbo_json_path)?;

    let output = rewrite_json::unset_path(&turbo_json, &["experimentalSpaces", "id"])?;
    if let Some(output) = output {
        fs::write(turbo_json_path, output)?;
        Ok(UnlinkSpacesResult::Unlinked)
    } else {
        Ok(UnlinkSpacesResult::NoSpacesFound)
    }
}
