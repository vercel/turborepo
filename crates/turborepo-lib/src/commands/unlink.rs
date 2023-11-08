use std::fs;

use turborepo_ui::GREY;

use crate::{
    cli,
    cli::{Error, LinkTarget},
    commands::CommandBase,
    config,
    rewrite_json::unset_path,
};

enum UnlinkSpacesResult {
    Unlinked,
    NoSpacesFound,
}

fn unlink_remote_caching(base: &mut CommandBase) -> Result<(), cli::Error> {
    let needs_disabling =
        base.config()?.team_id().is_some() || base.config()?.team_slug().is_some();

    let output = if needs_disabling {
        let local_config_path = base.local_config_path();

        let before = local_config_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|error| config::Error::FailedToReadConfig {
                config_path: local_config_path.clone(),
                error,
            })?;
        let no_id = unset_path(&before, &["teamid"], false)?.unwrap_or(before);
        let no_slug = unset_path(&no_id, &["teamslug"], false)?.unwrap_or(no_id);

        local_config_path
            .ensure_dir()
            .map_err(|error| config::Error::FailedToSetConfig {
                config_path: local_config_path.clone(),
                error,
            })?;

        local_config_path
            .create_with_contents(no_slug)
            .map_err(|error| config::Error::FailedToSetConfig {
                config_path: local_config_path.clone(),
                error,
            })?;

        "> Disabled Remote Caching"
    } else {
        "> No Remote Caching config found"
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));

    Ok(())
}

fn unlink_spaces(base: &mut CommandBase) -> Result<(), cli::Error> {
    let needs_disabling =
        base.config()?.team_id().is_some() || base.config()?.team_slug().is_some();

    if needs_disabling {
        let local_config_path = base.local_config_path();
        let before = local_config_path
            .read_existing_to_string_or(Ok("{}"))
            .map_err(|e| config::Error::FailedToReadConfig {
                config_path: local_config_path.clone(),
                error: e,
            })?;
        let no_id = unset_path(&before, &["teamid"], false)?.unwrap_or(before);
        let no_slug = unset_path(&no_id, &["teamslug"], false)?.unwrap_or(no_id);

        local_config_path
            .ensure_dir()
            .map_err(|e| config::Error::FailedToSetConfig {
                config_path: local_config_path.clone(),
                error: e,
            })?;

        local_config_path
            .create_with_contents(no_slug)
            .map_err(|e| config::Error::FailedToSetConfig {
                config_path: local_config_path.clone(),
                error: e,
            })?;
    }

    // Space config is _also_ in turbo.json.
    let result = remove_spaces_from_turbo_json(base)?;

    let output = match (needs_disabling, result) {
        (_, UnlinkSpacesResult::Unlinked) => "> Unlinked Spaces",
        (true, _) => "> Unlinked Spaces",
        (false, UnlinkSpacesResult::NoSpacesFound) => "> No Spaces config found",
    };

    println!("{}", base.ui.apply(GREY.apply_to(output)));

    Ok(())
}

pub fn unlink(base: &mut CommandBase, target: LinkTarget) -> Result<(), cli::Error> {
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

fn remove_spaces_from_turbo_json(base: &CommandBase) -> Result<UnlinkSpacesResult, Error> {
    let turbo_json_path = base.repo_root.join_component("turbo.json");
    let turbo_json =
        fs::read_to_string(&turbo_json_path).map_err(|e| config::Error::FailedToReadConfig {
            config_path: turbo_json_path.clone(),
            error: e,
        })?;

    let output = unset_path(&turbo_json, &["experimentalSpaces", "id"], true)?;
    if let Some(output) = output {
        fs::write(&turbo_json_path, output).map_err(|e| config::Error::FailedToSetConfig {
            config_path: turbo_json_path.clone(),
            error: e,
        })?;
        Ok(UnlinkSpacesResult::Unlinked)
    } else {
        Ok(UnlinkSpacesResult::NoSpacesFound)
    }
}
