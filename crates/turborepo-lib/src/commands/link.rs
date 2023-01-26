use anyhow::{anyhow, Result};
use dialoguer::Confirm;
use dirs_next::home_dir;

use crate::{
    commands::CommandBase,
    ui::{BOLD, CYAN, GREY, UNDERLINE},
};

pub fn link(base: CommandBase) -> Result<()> {
    let homedir_path = home_dir().ok_or_else(|| anyhow!("could not find home directory."))?;
    let homedir = homedir_path.to_string_lossy();
    println!(">>> Remote Caching");
    println!();
    println!("  Remote Caching shares your cached Turborepo task outputs and logs across");
    println!("  all your team’s Vercel projects. It also can share outputs");
    println!("  with other services that enable Remote Caching, like CI/CD systems.");
    println!("  This results in faster build times and deployments for your team.");
    println!(
        "  For more info, see {}",
        UNDERLINE.apply_to("https://turbo.build/repo/docs/core-concepts/remote-caching")
    );
    println!();

    let repo_root_with_tilde = base.repo_root.to_string_lossy().replacen(&*homedir, "~", 1);

    if !should_link(&base, &repo_root_with_tilde)? {
        return Err(anyhow!("canceled"));
    }

    if base.user_config()?.token.is_none() {
        return Err(anyhow!(
            "User not found. Please login to Turborepo first by running {}.",
            BOLD.apply_to("`npx turbo login`")
        ));
    }
    let teams_response = base.api_client.get_teams().await?;
    Ok(())
}

fn should_link(base: &CommandBase, location: &str) -> Result<bool> {
    let prompt = format!(
        "{}{} {}",
        BOLD.apply_to(GREY.apply_to("? ")),
        BOLD.apply_to("Would you like to enable Remote Caching for"),
        base.ui.apply(BOLD.apply_to(CYAN.apply_to(location)))
    );

    Ok(Confirm::new().with_prompt(prompt).interact()?)
}
