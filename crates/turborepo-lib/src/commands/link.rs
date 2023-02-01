#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

use anyhow::{anyhow, Context, Result};
use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirm, Select};
use dirs_next::home_dir;

use crate::{
    client::{CachingStatus, Team, UserClient},
    commands::CommandBase,
    ui::{BOLD, CYAN, GREY, UNDERLINE},
};

enum SelectedTeam<'a> {
    User,
    Team(&'a Team),
}

pub async fn link(mut base: CommandBase, modify_gitignore: bool) -> Result<()> {
    let homedir_path = home_dir().ok_or_else(|| anyhow!("could not find home directory."))?;
    let homedir = homedir_path.to_string_lossy();
    println!(
        ">>> Remote Caching
  Remote Caching shares your cached Turborepo task outputs and logs across
  all your team’s Vercel projects. It also can share outputs
  with other services that enable Remote Caching, like CI/CD systems.
  This results in faster build times and deployments for your team.
  For more info, see {}
",
        base.ui.apply(
            UNDERLINE.apply_to("https://turbo.build/repo/docs/core-concepts/remote-caching")
        )
    );

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

    let user_display_name = user_response
        .user
        .name
        .as_deref()
        .unwrap_or(user_response.user.username.as_str());

    let selected_team = select_team(&teams_response.teams, user_display_name)?;

    let team_id = match selected_team {
        SelectedTeam::User => user_response.user.id.as_str(),
        SelectedTeam::Team(team) => team.id.as_str(),
    };
    let response = api_client.get_caching_status(team_id).await?;
    match response.status {
        CachingStatus::Disabled => {
            let should_enable = should_enable_caching()?;
            if should_enable {
                match selected_team {
                    SelectedTeam::Team(team) if team.is_owner() => {
                        let url =
                            format!("https://vercel.com/teams/{}/settings/billing", team.slug);

                        enable_caching(&url)?;
                    }
                    SelectedTeam::User => {
                        let url = "https://vercel.com/account/billing";

                        enable_caching(url)?;
                    }
                    _ => {}
                }
            }
        }
        CachingStatus::OverLimit => return Err(anyhow!("usage limit")),
        CachingStatus::Paused => return Err(anyhow!("spending paused")),
        CachingStatus::Enabled => {}
    }

    fs::create_dir_all(base.repo_root.join(".turbo"))
        .context("could not create .turbo directory")?;
    base.repo_config_mut()?
        .set_team_id(Some(team_id.to_string()))?;

    let chosen_team_name = match selected_team {
        SelectedTeam::User => user_display_name,
        SelectedTeam::Team(team) => team.name.as_str(),
    };

    if modify_gitignore {
        add_turbo_to_gitignore(&base)?;
    }

    println!(
        "
{}  Turborepo CLI authorized for {}

{}
    ",
        base.ui.rainbow(">>> Success!"),
        chosen_team_name,
        GREY.apply_to("To disable Remote Caching, run `npx turbo unlink`")
    );
    Ok(())
}

fn should_enable_caching() -> Result<bool> {
    let theme = ColorfulTheme::default();
    Ok(Confirm::with_theme(&theme)
        .with_prompt(
            "Remote Caching was previously disabled for this team. Would you like to enable it \
             now?",
        )
        .default(true)
        .interact()?)
}

fn select_team<'a>(teams: &'a [Team], user_display_name: &'a str) -> Result<SelectedTeam<'a>> {
    let mut team_names = vec![user_display_name];
    team_names.extend(teams.iter().map(|team| team.name.as_str()));

    let theme = ColorfulTheme {
        active_item_style: Style::new().cyan().bold(),
        active_item_prefix: Style::new().cyan().bold().apply_to(">".to_string()),
        ..ColorfulTheme::default()
    };
    let selection = Select::with_theme(&theme)
        .items(&team_names)
        .default(0)
        .interact()?;

    if selection == 0 {
        Ok(SelectedTeam::User)
    } else {
        Ok(SelectedTeam::Team(&teams[selection - 1]))
    }
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

fn enable_caching(url: &str) -> Result<()> {
    webbrowser::open(url).with_context(|| {
        format!(
            "Failed to open browser. Please visit {} to enable Remote Caching",
            url
        )
    })?;

    println!("Visit {} in your browser to enable Remote Caching", url);

    // We return an error no matter what
    return Err(anyhow!("link after enabling caching"));
}

fn add_turbo_to_gitignore(base: &CommandBase) -> Result<()> {
    let gitignore_path = base.repo_root.join(".gitignore");

    if !gitignore_path.exists() {
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, ".turbo")?;
    } else {
        let gitignore = File::open(&gitignore_path)?;
        let mut lines = io::BufReader::new(gitignore).lines();
        let has_turbo = lines.any(|line| line.map_or(false, |line| line.trim() == ".turbo"));
        if !has_turbo {
            let mut gitignore = OpenOptions::new()
                .read(true)
                .append(true)
                .open(&gitignore_path)?;
            writeln!(gitignore, ".turbo")?;
        }
    }

    Ok(())
}
