use turborepo_ui::GREY;

use crate::{cli, commands::CommandBase, config, rewrite_json::unset_path};

fn unlink_remote_caching(base: &mut CommandBase) -> Result<(), cli::Error> {
    let needs_disabling = base.opts.api_client_opts.team_id.is_some()
        || base.opts.api_client_opts.team_slug.is_some();

    let output = if needs_disabling {
        let local_config_path = base.local_config_path();

        let before = local_config_path
            .read_existing_to_string()
            .map_err(|error| config::Error::FailedToReadConfig {
                config_path: local_config_path.clone(),
                error,
            })?
            .unwrap_or_else(|| String::from("{}"));
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

    println!("{}", base.color_config.apply(GREY.apply_to(output)));

    Ok(())
}

pub fn unlink(base: &mut CommandBase) -> Result<(), cli::Error> {
    unlink_remote_caching(base)?;
    Ok(())
}
