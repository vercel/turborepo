#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

#[cfg(not(test))]
use console::Style;
use console::StyledObject;
#[cfg(not(test))]
use dialoguer::FuzzySelect;
use dialoguer::{theme::ColorfulTheme, Confirm};
use dirs_next::home_dir;
#[cfg(test)]
use rand::Rng;
use thiserror::Error;
use turborepo_api_client::Client;
#[cfg(not(test))]
use turborepo_ui::CYAN;
use turborepo_ui::{BOLD, GREY, UNDERLINE};
use turborepo_vercel_api::{CachingStatus, Space, Team};

use crate::{
    cli::LinkTarget,
    commands::CommandBase,
    config,
    rewrite_json::{self, set_path, unset_path},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] config::Error),
    #[error("unable to find team {0}")]
    TeamNotFound(String),
    #[error("usage limit")]
    UsageLimit,
    #[error("spending paused")]
    SpendingPaused,
    #[error("could not find home directory.")]
    HomeDirectoryNotFound,
    #[error("User not found. Please login to Turborepo first by running {command}.")]
    TokenNotFound { command: StyledObject<&'static str> },
    #[error("canceled")]
    UserCanceled,
    #[error("could not get user information {0}")]
    UserNotFound(#[source] turborepo_api_client::Error),
    #[error("could not get teams information")]
    TeamsNotFound(#[source] turborepo_api_client::Error),
    #[error("could not get spaces information")]
    SpacesNotFound(#[source] turborepo_api_client::Error),
    #[error("could not get caching status")]
    CachingStatusNotFound(#[source] turborepo_api_client::Error),
    #[error("Failed to open browser. Please visit {0} to enable Remote Caching")]
    OpenBrowser(String, #[source] io::Error),
    #[error("please re-run `link` after enabling caching")]
    EnableCaching,
    #[error(
        "Could not persist selected space ({space_id}) to `experimentalSpaces.id` in turbo.json"
    )]
    WriteToTurboJson {
        space_id: String,
        #[source]
        error: io::Error,
    },
    #[error(transparent)]
    Rewrite(#[from] rewrite_json::RewriteError),
}

#[derive(Clone)]
pub(crate) enum SelectedTeam<'a> {
    User,
    Team(&'a Team),
}

#[derive(Clone)]
pub(crate) enum SelectedSpace<'a> {
    Space(&'a Space),
}

pub(crate) const REMOTE_CACHING_INFO: &str = "  Remote Caching shares your cached Turborepo task \
                                              outputs and logs across
  all your team’s Vercel projects. It also can share outputs
  with other services that enable Remote Caching, like CI/CD systems.
  This results in faster build times and deployments for your team.";
pub(crate) const REMOTE_CACHING_URL: &str =
    "https://turbo.build/repo/docs/core-concepts/remote-caching";
pub(crate) const SPACES_URL: &str = "https://vercel.com/docs/workflow-collaboration/vercel-spaces";

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
    api_client: &impl Client,
    team_id: &str,
    token: &str,
    selected_team: Option<SelectedTeam<'a>>,
) -> Result<(), Error> {
    let team_slug = selected_team.as_ref().and_then(|team| match team {
        SelectedTeam::Team(team) => Some(team.slug.as_str()),
        SelectedTeam::User => None,
    });

    let response = api_client
        .get_caching_status(token, team_id, team_slug)
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
                            .ok()
                            .flatten()
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
    modify_gitignore: bool,
    target: LinkTarget,
) -> Result<(), Error> {
    let homedir_path = home_dir().ok_or_else(|| Error::HomeDirectoryNotFound)?;
    let homedir = homedir_path.to_string_lossy();
    let repo_root_with_tilde = base.repo_root.to_string().replacen(&*homedir, "~", 1);
    let api_client = base.api_client()?;
    let token = base.config()?.token().ok_or_else(|| Error::TokenNotFound {
        command: base.ui.apply(BOLD.apply_to("`npx turbo login`")),
    })?;

    match target {
        LinkTarget::RemoteCache => {
            println!(
                ">>> Remote Caching

    {}
      For more info, see {}
      ",
                REMOTE_CACHING_INFO,
                base.ui.apply(UNDERLINE.apply_to(REMOTE_CACHING_URL))
            );

            if !should_link_remote_cache(base, &repo_root_with_tilde)? {
                return Err(Error::UserCanceled);
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
                .map_err(Error::TeamsNotFound)?;

            let selected_team = select_team(base, &teams_response.teams, user_display_name)?;

            let team_id = match selected_team {
                SelectedTeam::User => user_response.user.id.as_str(),
                SelectedTeam::Team(team) => team.id.as_str(),
            };

            verify_caching_enabled(&api_client, team_id, token, Some(selected_team.clone()))
                .await?;

            let local_config_path = base.local_config_path();
            let before = local_config_path
                .read_existing_to_string_or(Ok("{}"))
                .map_err(|e| config::Error::FailedToReadConfig {
                    config_path: local_config_path.clone(),
                    error: e,
                })?;

            let no_preexisting_id = unset_path(&before, &["teamid"], false)?.unwrap_or(before);
            let no_preexisting_slug =
                unset_path(&no_preexisting_id, &["teamslug"], false)?.unwrap_or(no_preexisting_id);

            let after = set_path(
                &no_preexisting_slug,
                &["teamId"],
                &format!("\"{}\"", team_id),
            )?;
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
                add_turbo_to_gitignore(base).map_err(|error| config::Error::FailedToSetConfig {
                    config_path: base.repo_root.join_component(".gitignore"),
                    error,
                })?;
            }

            println!(
                "
    {}  Turborepo CLI authorized for {}

    {}
        ",
                base.ui.rainbow(">>> Success!"),
                base.ui.apply(BOLD.apply_to(chosen_team_name)),
                GREY.apply_to("To disable Remote Caching, run `npx turbo unlink`")
            );
            Ok(())
        }
        LinkTarget::Spaces => {
            println!(
                ">>> Vercel Spaces (Beta)

      For more info, see {}
      ",
                base.ui.apply(UNDERLINE.apply_to(SPACES_URL))
            );

            if !should_link_spaces(base, &repo_root_with_tilde)? {
                return Err(Error::UserCanceled);
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
                .map_err(Error::TeamsNotFound)?;

            let selected_team = select_team(base, &teams_response.teams, user_display_name)?;

            let team_id = match selected_team {
                SelectedTeam::User => user_response.user.id.as_str(),
                SelectedTeam::Team(team) => team.id.as_str(),
            };

            let spaces_response = api_client
                .get_spaces(token, Some(team_id))
                .await
                .map_err(Error::SpacesNotFound)?;

            let selected_space = select_space(base, &spaces_response.spaces)?;

            // print result from selected_space
            let SelectedSpace::Space(space) = selected_space;

            add_space_id_to_turbo_json(base, &space.id)?;

            let local_config_path = base.local_config_path();
            let before = local_config_path
                .read_existing_to_string_or(Ok("{}"))
                .map_err(|error| config::Error::FailedToReadConfig {
                    config_path: local_config_path.clone(),
                    error,
                })?;

            let no_preexisting_id = unset_path(&before, &["teamid"], false)?.unwrap_or(before);
            let no_preexisting_slug =
                unset_path(&no_preexisting_id, &["teamslug"], false)?.unwrap_or(no_preexisting_id);

            let after = set_path(
                &no_preexisting_slug,
                &["teamId"],
                &format!("\"{}\"", team_id),
            )?;
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

            println!(
                "
    {} {} linked to {}

    {}
        ",
                base.ui.rainbow(">>> Success!"),
                base.ui.apply(BOLD.apply_to(&repo_root_with_tilde)),
                base.ui.apply(BOLD.apply_to(&space.name)),
                GREY.apply_to(
                    "To remove Spaces integration, run `npx turbo unlink --target spaces`"
                )
            );

            Ok(())
        }
    }
}

fn should_enable_caching() -> Result<bool, Error> {
    let theme = ColorfulTheme::default();

    Confirm::with_theme(&theme)
        .with_prompt(
            "Remote Caching was previously disabled for this team. Would you like to enable it \
             now?",
        )
        .default(true)
        .interact()
        .map_err(|_| Error::UserCanceled)
}

#[cfg(test)]
fn select_team<'a>(
    _: &CommandBase,
    teams: &'a [Team],
    _: &'a str,
) -> Result<SelectedTeam<'a>, Error> {
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..=(teams.len()));
    if idx == teams.len() {
        Ok(SelectedTeam::User)
    } else {
        Ok(SelectedTeam::Team(&teams[idx]))
    }
}

#[cfg(not(test))]
fn select_team<'a>(
    base: &CommandBase,
    teams: &'a [Team],
    user_display_name: &'a str,
) -> Result<SelectedTeam<'a>, Error> {
    let mut team_names = vec![user_display_name];
    team_names.extend(teams.iter().map(|team| team.name.as_str()));

    let theme = ColorfulTheme {
        active_item_style: Style::new().cyan().bold(),
        active_item_prefix: Style::new().cyan().bold().apply_to(">".to_string()),
        prompt_prefix: Style::new().dim().bold().apply_to("?".to_string()),
        values_style: Style::new().cyan(),
        ..ColorfulTheme::default()
    };

    let prompt = format!(
        "{}\n  {}",
        base.ui.apply(BOLD.apply_to(
            "Which Vercel scope (and Remote Cache) do you want associated with this Turborepo?",
        )),
        base.ui
            .apply(CYAN.apply_to("[Use arrows to move, type to filter]"))
    );

    let selection = FuzzySelect::with_theme(&theme)
        .with_prompt(prompt)
        .items(&team_names)
        .default(0)
        .interact()
        .map_err(|_| Error::UserCanceled)?;

    if selection == 0 {
        Ok(SelectedTeam::User)
    } else {
        Ok(SelectedTeam::Team(&teams[selection - 1]))
    }
}

#[cfg(test)]
fn select_space<'a>(_: &CommandBase, spaces: &'a [Space]) -> Result<SelectedSpace<'a>, Error> {
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..spaces.len());
    Ok(SelectedSpace::Space(&spaces[idx]))
}

#[cfg(not(test))]
fn select_space<'a>(base: &CommandBase, spaces: &'a [Space]) -> Result<SelectedSpace<'a>, Error> {
    let space_names = spaces
        .iter()
        .map(|space| space.name.as_str())
        .collect::<Vec<_>>();

    let theme = ColorfulTheme {
        active_item_style: Style::new().cyan().bold(),
        active_item_prefix: Style::new().cyan().bold().apply_to(">".to_string()),
        prompt_prefix: Style::new().dim().bold().apply_to("?".to_string()),
        values_style: Style::new().cyan(),
        ..ColorfulTheme::default()
    };

    let prompt = format!(
        "{}\n  {}",
        base.ui.apply(
            BOLD.apply_to("Which Vercel space do you want associated with this Turborepo?",)
        ),
        base.ui
            .apply(CYAN.apply_to("[Use arrows to move, type to filter]"))
    );

    let selection = FuzzySelect::with_theme(&theme)
        .with_prompt(prompt)
        .items(&space_names)
        .default(0)
        .interact()
        .map_err(|_| Error::UserCanceled)?;

    Ok(SelectedSpace::Space(&spaces[selection]))
}

#[cfg(test)]
fn should_link_remote_cache(_: &CommandBase, _: &str) -> Result<bool, Error> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link_remote_cache(base: &CommandBase, location: &str) -> Result<bool, Error> {
    let prompt = format!(
        "{}{} {}",
        base.ui.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.ui
            .apply(BOLD.apply_to("Would you like to enable Remote Caching for")),
        base.ui.apply(BOLD.apply_to(CYAN.apply_to(location)))
    );

    Confirm::new()
        .with_prompt(prompt)
        .interact()
        .map_err(|_| Error::UserCanceled)
}

#[cfg(test)]
fn should_link_spaces(_: &CommandBase, _: &str) -> Result<bool, Error> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link_spaces(base: &CommandBase, location: &str) -> Result<bool, Error> {
    let prompt = format!(
        "{}{} {} {}",
        base.ui.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.ui.apply(BOLD.apply_to("Would you like to link")),
        base.ui.apply(BOLD.apply_to(CYAN.apply_to(location))),
        base.ui.apply(BOLD.apply_to("to Vercel Spaces")),
    );

    Confirm::new()
        .with_prompt(prompt)
        .interact()
        .map_err(|_| Error::UserCanceled)
}

fn enable_caching(url: &str) -> Result<(), Error> {
    webbrowser::open(url).map_err(|err| Error::OpenBrowser(url.to_string(), err))?;

    println!("Visit {} in your browser to enable Remote Caching", url);

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

fn add_space_id_to_turbo_json(base: &CommandBase, space_id: &str) -> Result<(), Error> {
    let turbo_json_path = base.repo_root.join_component("turbo.json");
    let turbo_json = turbo_json_path
        .read_existing_to_string_or(Ok("{}"))
        .map_err(|error| config::Error::FailedToReadConfig {
            config_path: turbo_json_path.clone(),
            error,
        })?;

    let space_id_json_value = format!("\"{}\"", space_id);

    let output = rewrite_json::set_path(
        &turbo_json,
        &["experimentalSpaces", "id"],
        &space_id_json_value,
    )?;

    fs::write(turbo_json_path, output).map_err(|error| Error::WriteToTurboJson {
        space_id: space_id.to_string(),
        error,
    })?;

    Ok(())
}

#[cfg(test)]
mod test {
    use std::{cell::OnceCell, fs};

    use anyhow::Result;
    use tempfile::{NamedTempFile, TempDir};
    use turbopath::AbsoluteSystemPathBuf;
    use turborepo_ui::UI;
    use turborepo_vercel_api_mock::start_test_server;

    use crate::{
        cli::LinkTarget,
        commands::{link, CommandBase},
        config::{RawTurboJSON, TurborepoConfigBuilder},
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
        let mut base = CommandBase {
            global_config_path: Some(
                AbsoluteSystemPathBuf::try_from(user_config_file.path().to_path_buf()).unwrap(),
            ),
            repo_root: repo_root.clone(),
            ui: UI::new(false),
            config: OnceCell::new(),
            args: Args::default(),
            version: "",
        };
        base.config
            .set(
                TurborepoConfigBuilder::new(&base)
                    .with_api_url(Some(format!("http://localhost:{}", port)))
                    .with_login_url(Some(format!("http://localhost:{}", port)))
                    .with_token(Some("token".to_string()))
                    .build()
                    .unwrap(),
            )
            .unwrap();

        link::link(&mut base, false, LinkTarget::RemoteCache)
            .await
            .unwrap();

        handle.abort();

        // read the config
        let updated_config = TurborepoConfigBuilder::new(&base).build().unwrap();
        let team_id = updated_config.team_id();

        assert!(
            team_id == Some(turborepo_vercel_api_mock::EXPECTED_USER_ID)
                || team_id == Some(turborepo_vercel_api_mock::EXPECTED_TEAM_ID)
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_link_spaces() {
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
        let mut base = CommandBase {
            global_config_path: Some(
                AbsoluteSystemPathBuf::try_from(user_config_file.path().to_path_buf()).unwrap(),
            ),
            repo_root: repo_root.clone(),
            ui: UI::new(false),
            config: OnceCell::new(),
            args: Args::default(),
            version: "",
        };
        base.config
            .set(
                TurborepoConfigBuilder::new(&base)
                    .with_api_url(Some(format!("http://localhost:{}", port)))
                    .with_login_url(Some(format!("http://localhost:{}", port)))
                    .with_token(Some("token".to_string()))
                    .build()
                    .unwrap(),
            )
            .unwrap();

        // turbo config
        let turbo_json_file = base.repo_root.join_component("turbo.json");

        fs::write(
            turbo_json_file.as_path(),
            r#"{ "globalEnv": [], "pipeline": {} }"#,
        )
        .unwrap();

        link::link(&mut base, false, LinkTarget::Spaces)
            .await
            .unwrap();

        handle.abort();

        // verify space id is added to turbo.json
        let turbo_json_contents = fs::read_to_string(&turbo_json_file).unwrap();
        let turbo_json: RawTurboJSON = serde_json::from_str(&turbo_json_contents).unwrap();
        assert_eq!(
            turbo_json.experimental_spaces.unwrap().id.unwrap(),
            turborepo_vercel_api_mock::EXPECTED_SPACE_ID
        );
    }
}
