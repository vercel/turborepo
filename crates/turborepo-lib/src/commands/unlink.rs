use std::fs;

use anyhow::{Context, Result};
use turborepo_ui::GREY;

use crate::{cli::LinkTarget, commands::CommandBase, rewrite_json};

enum UnlinkSpacesResult {
    Unlinked,
    NoSpacesFound,
}

fn unlink_remote_caching(base: &mut CommandBase) -> Result<()> {
    let repo_config: &mut crate::config::RepoConfig = base.repo_config_mut()?;
    let needs_disabling = repo_config.team_id().is_some() || repo_config.team_slug().is_some();

    let output = if needs_disabling {
        repo_config.set_team_id(None)?;
        "> Disabled Remote Caching"
    } else {
        "> No Remote Caching config found"
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));

    Ok(())
}

fn unlink_spaces(base: &mut CommandBase) -> Result<()> {
    let repo_config: &mut crate::config::RepoConfig = base.repo_config_mut()?;
    let needs_disabling =
        repo_config.space_team_id().is_some() || repo_config.space_team_slug().is_some();

    if needs_disabling {
        repo_config.set_space_team_id(None)?;
    }

    // Space config is _also_ in turbo.json.
    let result =
        remove_spaces_from_turbo_json(base).context("could not unlink. Something went wrong")?;

    let output = match (needs_disabling, result) {
        (_, UnlinkSpacesResult::Unlinked) => "> Unlinked Spaces",
        (true, _) => "> Unlinked Spaces",
        (false, UnlinkSpacesResult::NoSpacesFound) => "> No Spaces config found",
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));

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
