use anyhow::{anyhow, Context, Result};
use dialoguer::Confirm;
use dirs_next::home_dir;

use crate::{
    client::UserClient,
    commands::CommandBase,
    ui::{BOLD, CYAN, GREY, UNDERLINE},
};

pub async fn link(mut base: CommandBase) -> Result<()> {
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

    let api_client = base.api_client()?.ok_or_else(|| {
        anyhow!(
            "User not found. Please login to Turborepo first by running {}.",
            BOLD.apply_to("`npx turbo login`")
        )
    })?;

    let teams_response = api_client
        .get_teams()
        .await
        .context("could not get team information")?;

    let user_response = api_client
        .get_user()
        .await
        .context("could not get user information")?;

    let mut teams = if &user_response.user.name == "" {
        vec![user_response.user.username.as_str()]
    } else {
        vec![user_response.user.name.as_str()]
    };

    teams.extend(teams_response.teams.iter().map(|team| team.name.as_str()));
    println!("{}", teams.join("\n"));
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
