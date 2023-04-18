use std::fs::File;

use anyhow::{anyhow, Context, Result};

use crate::{
    cli::LinkTarget,
    commands::CommandBase,
    config::{SpacesJson, TurboJson},
    ui::GREY,
};

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

pub fn unlink(base: &mut CommandBase, target: Option<LinkTarget>) -> Result<()> {
    let target = target.unwrap_or(LinkTarget::RemoteCache);

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
    let turbo_json_path = base.repo_root.join("turbo.json");

    if !turbo_json_path.exists() {
        return Err(anyhow!("turbo.json not found."));
    }

    let turbo_json_file = File::open(&turbo_json_path)?;
    let mut turbo_json: TurboJson = serde_json::from_reader(turbo_json_file)?;
    let has_spaces_id = turbo_json
        .experimental_spaces
        .unwrap_or(SpacesJson {
            id: None,
            other: None,
        })
        .id
        .is_some();
    // remove the spaces config
    // TODO: in the future unlink should possible just remove the spaces id
    turbo_json.experimental_spaces = None;

    // write turbo_json back to file
    let config_file = File::create(&turbo_json_path)?;
    serde_json::to_writer_pretty(&config_file, &turbo_json)?;

    match has_spaces_id {
        true => Ok(UnlinkSpacesResult::Unlinked),
        false => Ok(UnlinkSpacesResult::NoSpacesFound),
    }
}
