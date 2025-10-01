#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

#[cfg(not(test))]
use console::Style;
use console::StyledObject;
use dialoguer::Confirm;
#[cfg(not(test))]
use dialoguer::FuzzySelect;
use dirs_next::home_dir;
#[cfg(test)]
use rand::Rng;
use thiserror::Error;
use turborepo_api_client::{CacheClient, Client};
#[cfg(not(test))]
use turborepo_ui::CYAN;
use turborepo_ui::{DialoguerTheme, BOLD, GREY};
use turborepo_vercel_api::{CachingStatus, Team};

use crate::{
    commands::CommandBase,
    config,
    gitignore::ensure_turbo_is_gitignored,
    rewrite_json::{self, set_path, unset_path},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error("usage limit")]
    UsageLimit,
    #[error("spending paused")]
    SpendingPaused,
    #[error("Could not find home directory.")]
    HomeDirectoryNotFound,
    #[error("User not found. Please login to Turborepo first by running {command}.")]
    TokenNotFound { command: StyledObject<&'static str> },
    // User decided to not link the remote cache
    #[error("Link cancelled.")]
    NotLinking,
    #[error("Canceled.")]
    UserCanceled(#[source] dialoguer::Error),
    #[error("Could not get user information: {0}")]
    UserNotFound(#[source] turborepo_api_client::Error),
    // We failed to fetch the team for whatever reason
    #[error("Could not get information for team: {1}")]
    TeamRequest(#[source] turborepo_api_client::Error, String),
    // We fetched the team, but it doesn't exist.
    #[error("Could not find team: {0}")]
    TeamNotFound(String),
    #[error("Could not get teams information.")]
    TeamsRequest(#[source] turborepo_api_client::Error),
    #[error("Could not get caching status.")]
    CachingStatusNotFound(#[source] turborepo_api_client::Error),
    #[error("Failed to open browser. Please visit {0} to enable Remote Caching")]
    OpenBrowser(String, #[source] io::Error),
    #[error("Please re-run `link` after enabling caching.")]
    EnableCaching,
    #[error(transparent)]
    Rewrite(#[from] rewrite_json::RewriteError),
}

#[derive(Clone)]
pub(crate) enum SelectedTeam<'a> {
    User,
    Team(&'a Team),
}

pub(crate) const REMOTE_CACHING_INFO: &str =
    "Remote Caching makes your caching multiplayer,\nsharing build outputs and logs between \
     developers and CI/CD systems.\n\nBuild and deploy faster.";
pub(crate) const REMOTE_CACHING_URL: &str =
    "https://turborepo.com/docs/core-concepts/remote-caching";

/// Verifies that caching status for a team is enabled, or prompts the user to
/// enable it.
///
/// # Arguments
///
/// * `team_id`: ID for team selected
/// * `token`: API token
/// * `selected_team`: The team selected
///
/// returns: Result<(), Error>
pub(crate) async fn verify_caching_enabled<'a>(
    api_client: &(impl Client + CacheClient),
    team_id: &str,
    token: &str,
    selected_team: Option<SelectedTeam<'a>>,
) -> Result<(), Error> {
    let team_slug = selected_team.as_ref().and_then(|team| match team {
        SelectedTeam::Team(team) => Some(team.slug.as_str()),
        SelectedTeam::User => None,
    });

    let response = api_client
        .get_caching_status(token, Some(team_id), team_slug)
        .await
        .map_err(Error::CachingStatusNotFound)?;

    match response.status {
        CachingStatus::Disabled => {
            let should_enable = should_enable_caching()?;
            if should_enable {
                match selected_team {
                    Some(SelectedTeam::Team(team)) if team.is_owner() => {
                        let url =
                            format!("https://vercel.com/teams/{}/settings/billing", team.slug);

                        enable_caching(&url)?;
                    }
                    Some(SelectedTeam::User) => {
                        let url = "https://vercel.com/account/billing";

                        enable_caching(url)?;
                    }
                    None => {
                        let team = api_client
                            .get_team(token, team_id)
                            .await
                            .map_err(|err| Error::TeamRequest(err, team_id.to_string()))?
                            .ok_or_else(|| Error::TeamNotFound(team_id.to_string()))?;
                        let url =
                            format!("https://vercel.com/teams/{}/settings/billing", team.slug);

                        enable_caching(&url)?;
                    }
                    _ => {}
                }
            }

            Ok(())
        }
        CachingStatus::OverLimit => Err(Error::UsageLimit),
        CachingStatus::Paused => Err(Error::SpendingPaused),
        CachingStatus::Enabled => Ok(()),
    }
}

pub async fn link(
    base: &mut CommandBase,
    scope: Option<String>,
    modify_gitignore: bool,
    yes: bool,
) -> Result<(), Error> {
    let homedir_path = home_dir().ok_or_else(|| Error::HomeDirectoryNotFound)?;
    let homedir = homedir_path.to_string_lossy();
    let repo_root_with_tilde = base.repo_root.to_string().replacen(&*homedir, "~", 1);
    let api_client = base.api_client()?;

    // Always try to get a valid token with automatic refresh if expired
    let token = match turborepo_auth::get_token_with_refresh().await {
        Ok(Some(refreshed_token)) => {
            // Store the refreshed token temporarily for this command
            Box::leak(refreshed_token.into_boxed_str())
        }
        Ok(None) | Err(_) => {
            // Fall back to the token from config/CLI if refresh logic didn't work
            base.opts()
                .api_client_opts
                .token
                .as_deref()
                .ok_or_else(|| Error::TokenNotFound {
                    command: base.color_config.apply(BOLD.apply_to("`npx turbo login`")),
                })?
        }
    };

    println!(
        "\n{}\n\n{}\n\nFor more information, visit: {}\n",
        base.color_config.rainbow(">>> Remote Caching"),
        REMOTE_CACHING_INFO,
        REMOTE_CACHING_URL
    );

    if !yes && !should_link_remote_cache(base, &repo_root_with_tilde)? {
        return Err(Error::NotLinking);
    }

    let user_response = api_client
        .get_user(token)
        .await
        .map_err(Error::UserNotFound)?;

    let user_display_name = user_response
        .user
        .name
        .as_deref()
        .unwrap_or(user_response.user.username.as_str());

    let teams_response = api_client
        .get_teams(token)
        .await
        .map_err(Error::TeamsRequest)?;

    let selected_team = if let Some(team_slug) = scope {
        SelectedTeam::Team(
            teams_response
                .teams
                .iter()
                .find(|team| team.slug == team_slug)
                .ok_or_else(|| Error::TeamNotFound(team_slug.to_string()))?,
        )
    } else {
        select_team(base, &teams_response.teams)?
    };

    let team_id = match selected_team {
        SelectedTeam::User => user_response.user.id.as_str(),
        SelectedTeam::Team(team) => team.id.as_str(),
    };

    verify_caching_enabled(&api_client, team_id, token, Some(selected_team.clone())).await?;

    let local_config_path = base.local_config_path();
    let before = local_config_path
        .read_existing_to_string()
        .map_err(|e| config::Error::FailedToReadConfig {
            config_path: local_config_path.clone(),
            error: e,
        })?
        .unwrap_or_else(|| String::from("{}"));

    let no_preexisting_id = unset_path(&before, &["teamid"], false)?.unwrap_or(before);
    let no_preexisting_slug =
        unset_path(&no_preexisting_id, &["teamslug"], false)?.unwrap_or(no_preexisting_id);

    let after = set_path(&no_preexisting_slug, &["teamId"], &format!("\"{team_id}\""))?;
    let local_config_path = base.local_config_path();
    local_config_path
        .ensure_dir()
        .map_err(|error| config::Error::FailedToSetConfig {
            config_path: local_config_path.clone(),
            error,
        })?;
    local_config_path
        .create_with_contents(after)
        .map_err(|error| config::Error::FailedToSetConfig {
            config_path: local_config_path.clone(),
            error,
        })?;

    let chosen_team_name = match selected_team {
        SelectedTeam::User => user_display_name,
        SelectedTeam::Team(team) => team.name.as_str(),
    };

    if modify_gitignore {
        ensure_turbo_is_gitignored(&base.repo_root).map_err(|error| {
            config::Error::FailedToSetConfig {
                config_path: base.repo_root.join_component(".gitignore"),
                error,
            }
        })?;
    }

    println!(
        "
    {}  Turborepo CLI authorized for {}

    {}
        ",
        base.color_config.rainbow(">>> Success!"),
        base.color_config.apply(BOLD.apply_to(chosen_team_name)),
        GREY.apply_to("To disable Remote Caching, run `npx turbo unlink`")
    );
    Ok(())
}

fn should_enable_caching() -> Result<bool, Error> {
    let theme = DialoguerTheme::default();

    Confirm::with_theme(&theme)
        .with_prompt(
            "Remote Caching was previously disabled for this team. Would you like to enable it \
             now?",
        )
        .default(true)
        .interact()
        .map_err(Error::UserCanceled)
}

#[cfg(test)]
fn select_team<'a>(_: &CommandBase, teams: &'a [Team]) -> Result<SelectedTeam<'a>, Error> {
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..(teams.len()));
    Ok(SelectedTeam::Team(&teams[idx]))
}

#[cfg(not(test))]
fn select_team<'a>(base: &CommandBase, teams: &'a [Team]) -> Result<SelectedTeam<'a>, Error> {
    let team_names = teams
        .iter()
        .map(|team| team.name.as_str())
        .collect::<Vec<_>>();

    let theme = DialoguerTheme {
        active_item_style: Style::new().cyan().bold(),
        active_item_prefix: Style::new().cyan().bold().apply_to(">".to_string()),
        prompt_prefix: Style::new().dim().bold().apply_to("?".to_string()),
        values_style: Style::new().cyan(),
        ..DialoguerTheme::default()
    };

    let prompt = format!(
        "{}\n  {}",
        base.color_config.apply(BOLD.apply_to(
            "Which Vercel scope (and Remote Cache) do you want associated with this Turborepo?",
        )),
        base.color_config
            .apply(CYAN.apply_to("[Use arrows to move, type to filter]"))
    );

    let selection = FuzzySelect::with_theme(&theme)
        .with_prompt(prompt)
        .items(&team_names)
        .default(0)
        .interact()
        .map_err(Error::UserCanceled)?;

    Ok(SelectedTeam::Team(&teams[selection]))
}

#[cfg(test)]
fn should_link_remote_cache(_: &CommandBase, _: &str) -> Result<bool, Error> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link_remote_cache(base: &CommandBase, location: &str) -> Result<bool, Error> {
    let prompt = format!(
        "{}{} {}{}",
        base.color_config.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.color_config
            .apply(BOLD.apply_to("Enable Vercel Remote Cache for")),
        base.color_config
            .apply(BOLD.apply_to(CYAN.apply_to(location))),
        base.color_config.apply(BOLD.apply_to(" ?"))
    );

    Confirm::new()
        .with_prompt(prompt)
        .interact()
        .map_err(Error::UserCanceled)
}

fn enable_caching(url: &str) -> Result<(), Error> {
    webbrowser::open(url).map_err(|err| Error::OpenBrowser(url.to_string(), err))?;

    println!("Visit {url} in your browser to enable Remote Caching");

    // We return an error no matter what
    Err(Error::EnableCaching)
}

fn add_turbo_to_gitignore(base: &CommandBase) -> Result<(), io::Error> {
    let gitignore_path = base.repo_root.join_component(".gitignore");

    if !gitignore_path.exists() {
        let mut gitignore = File::create(gitignore_path)?;
        #[cfg(unix)]
        gitignore.metadata()?.permissions().set_mode(0o0644);
        writeln!(gitignore, ".turbo")?;
    } else {
        let gitignore = File::open(&gitignore_path)?;
        let mut lines = io::BufReader::new(gitignore).lines();
        let has_turbo = lines.any(|line| line.is_ok_and(|line| line.trim() == ".turbo"));
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

#[cfg(test)]
mod test {
    use std::fs;

    use anyhow::Result;
    use tempfile::{NamedTempFile, TempDir};
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::ColorConfig;
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        commands::{link, CommandBase},
        config::TurborepoConfigBuilder,
        opts::Opts,
        Args,
    };

    #[tokio::test]
    async fn test_link_remote_cache() -> Result<()> {
        // user config
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();

        // repo
        let repo_root_tmp_dir = TempDir::new().unwrap();
        let handle = repo_root_tmp_dir.path();
        let repo_root = AbsoluteSystemPathBuf::try_from(handle).unwrap();
        repo_root
            .join_component("turbo.json")
            .create_with_contents("{}")
            .unwrap();
        repo_root
            .join_component("package.json")
            .create_with_contents("{}")
            .unwrap();

        let repo_config_path = repo_root.join_components(&[".turbo", "config.json"]);
        repo_config_path.ensure_dir().unwrap();
        repo_config_path
            .create_with_contents(r#"{ "apiurl": "http://localhost:3000" }"#)
            .unwrap();

        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let override_global_config_path =
            AbsoluteSystemPathBuf::try_from(user_config_file.path().to_path_buf())?;

        let config = TurborepoConfigBuilder::new(&repo_root)
            .with_global_config_path(override_global_config_path.clone())
            .with_api_url(Some(format!("http://localhost:{}", port)))
            .with_login_url(Some(format!("http://localhost:{}", port)))
            .with_token(Some("token".to_string()))
            .build()?;

        let mut base = CommandBase::from_opts(
            Opts::new(&repo_root, &Args::default(), config)?,
            repo_root.clone(),
            "1.0.0",
            ColorConfig::new(false),
        );

        link::link(&mut base, None, false, false).await?;

        handle.abort();

        // read the config
        let updated_config = TurborepoConfigBuilder::new(&base.repo_root)
            .with_global_config_path(override_global_config_path)
            .build()?;
        let team_id = updated_config.team_id();

        assert!(
            team_id == Some(turborepo_vercel_api_mock::EXPECTED_USER_ID)
                || team_id == Some(turborepo_vercel_api_mock::EXPECTED_TEAM_ID)
        );

        Ok(())
    }
}
