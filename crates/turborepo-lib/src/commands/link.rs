#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    fs::{File, OpenOptions},
    io,
    io::{BufRead, Write},
};

use anyhow::{anyhow, Context, Result};
#[cfg(not(test))]
use console::Style;
#[cfg(not(test))]
use dialoguer::FuzzySelect;
use dialoguer::{theme::ColorfulTheme, Confirm};
use dirs_next::home_dir;
#[cfg(test)]
use rand::Rng;
use turbopath::RelativeSystemPathBuf;
use turborepo_api_client::{APIClient, CachingStatus, Space, Team};

#[cfg(not(test))]
use crate::ui::CYAN;
use crate::{
    cli::LinkTarget,
    commands::CommandBase,
    config::{SpacesJson, TurboJson},
    ui::{BOLD, GREY, UNDERLINE},
};

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
pub(crate) const SPACES_URL: &str = "https://vercel.com/docs/spaces";

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
    api_client: &APIClient,
    team_id: &str,
    token: &str,
    selected_team: Option<SelectedTeam<'a>>,
) -> Result<()> {
    let team_slug = selected_team.as_ref().and_then(|team| match team {
        SelectedTeam::Team(team) => Some(team.slug.as_str()),
        SelectedTeam::User => None,
    });
    let response = api_client
        .get_caching_status(token, team_id, team_slug)
        .await?;
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
                            .await?
                            .ok_or_else(|| anyhow!("unable to find team {}", team_id))?;
                        let url =
                            format!("https://vercel.com/teams/{}/settings/billing", team.slug);

                        enable_caching(&url)?;
                    }
                    _ => {}
                }
            }

            Ok(())
        }
        CachingStatus::OverLimit => Err(anyhow!("usage limit")),
        CachingStatus::Paused => Err(anyhow!("spending paused")),
        CachingStatus::Enabled => Ok(()),
    }
}

pub async fn link(
    base: &mut CommandBase,
    modify_gitignore: bool,
    target: LinkTarget,
) -> Result<()> {
    let homedir_path = home_dir().ok_or_else(|| anyhow!("could not find home directory."))?;
    let homedir = homedir_path.to_string_lossy();
    let repo_root_with_tilde = base.repo_root.to_string_lossy().replacen(&*homedir, "~", 1);
    let api_client = base.api_client()?;
    let token = base.user_config()?.token().ok_or_else(|| {
        anyhow!(
            "User not found. Please login to Turborepo first by running {}.",
            BOLD.apply_to("`npx turbo login`")
        )
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
                return Err(anyhow!("canceled"));
            }

            let user_response = api_client
                .get_user(token)
                .await
                .context("could not get user information")?;

            let user_display_name = user_response
                .user
                .name
                .as_deref()
                .unwrap_or(user_response.user.username.as_str());

            let teams_response = api_client
                .get_teams(token)
                .await
                .context("could not get team information")?;

            let selected_team = select_team(base, &teams_response.teams, user_display_name)?;

            let team_id = match selected_team {
                SelectedTeam::User => user_response.user.id.as_str(),
                SelectedTeam::Team(team) => team.id.as_str(),
            };

            verify_caching_enabled(&api_client, team_id, token, Some(selected_team.clone()))
                .await?;

            fs::create_dir_all(
                base.repo_root
                    .join_relative(RelativeSystemPathBuf::new(".turbo").expect("relative")),
            )
            .context("could not create .turbo directory")?;
            base.repo_config_mut()?
                .set_team_id(Some(team_id.to_string()))?;

            let chosen_team_name = match selected_team {
                SelectedTeam::User => user_display_name,
                SelectedTeam::Team(team) => team.name.as_str(),
            };

            if modify_gitignore {
                add_turbo_to_gitignore(base)?;
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
                return Err(anyhow!("canceled"));
            }

            let spaces_response = api_client
                .get_spaces(token, base.repo_config()?.team_id())
                .await
                .context("could not get spaces information")?;

            let selected_space = select_space(base, &spaces_response.spaces)?;

            // print result from selected_space
            let SelectedSpace::Space(space) = selected_space;

            add_space_id_to_turbo_json(base, &space.id).map_err(|err| {
                anyhow!(
                    "Could not persist selected space ({}) to `experimentalSpaces.id` in \
                     turbo.json {}",
                    space.id,
                    err
                )
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

#[cfg(test)]
fn select_team<'a>(_: &CommandBase, teams: &'a [Team], _: &'a str) -> Result<SelectedTeam<'a>> {
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
) -> Result<SelectedTeam<'a>> {
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
        .interact()?;

    if selection == 0 {
        Ok(SelectedTeam::User)
    } else {
        Ok(SelectedTeam::Team(&teams[selection - 1]))
    }
}

#[cfg(test)]
fn select_space<'a>(_: &CommandBase, spaces: &'a [Space]) -> Result<SelectedSpace<'a>> {
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..spaces.len());
    Ok(SelectedSpace::Space(&spaces[idx]))
}

#[cfg(not(test))]
fn select_space<'a>(base: &CommandBase, spaces: &'a [Space]) -> Result<SelectedSpace<'a>> {
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
        .interact()?;

    Ok(SelectedSpace::Space(&spaces[selection]))
}

#[cfg(test)]
fn should_link_remote_cache(_: &CommandBase, _: &str) -> Result<bool> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link_remote_cache(base: &CommandBase, location: &str) -> Result<bool> {
    let prompt = format!(
        "{}{} {}",
        base.ui.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.ui
            .apply(BOLD.apply_to("Would you like to enable Remote Caching for")),
        base.ui.apply(BOLD.apply_to(CYAN.apply_to(location)))
    );

    Ok(Confirm::new().with_prompt(prompt).interact()?)
}

#[cfg(test)]
fn should_link_spaces(_: &CommandBase, _: &str) -> Result<bool> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link_spaces(base: &CommandBase, location: &str) -> Result<bool> {
    let prompt = format!(
        "{}{} {} {}",
        base.ui.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.ui.apply(BOLD.apply_to("Would you like to link")),
        base.ui.apply(BOLD.apply_to(CYAN.apply_to(location))),
        base.ui.apply(BOLD.apply_to("to Vercel Spaces")),
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
    Err(anyhow!("link after enabling caching"))
}

fn add_turbo_to_gitignore(base: &CommandBase) -> Result<()> {
    let gitignore_path = base
        .repo_root
        .join_relative(RelativeSystemPathBuf::new(".gitignore").expect("relative"));

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

fn add_space_id_to_turbo_json(base: &CommandBase, space_id: &str) -> Result<()> {
    let turbo_json_path = base
        .repo_root
        .join_relative(RelativeSystemPathBuf::new("turbo.json").expect("relative"));

    if !turbo_json_path.exists() {
        return Err(anyhow!("turbo.json not found."));
    }

    let turbo_json_file = File::open(&turbo_json_path)?;
    let mut turbo_json: TurboJson = serde_json::from_reader(turbo_json_file)?;
    match turbo_json.experimental_spaces {
        Some(mut spaces_config) => {
            spaces_config.id = Some(space_id.to_string());
            turbo_json.experimental_spaces = Some(spaces_config);
        }
        None => {
            turbo_json.experimental_spaces = Some(SpacesJson {
                id: Some(space_id.to_string()),
                other: None,
            });
        }
    }

    // write turbo_json back to file
    let config_file = File::create(&turbo_json_path)?;
    serde_json::to_writer_pretty(&config_file, &turbo_json)?;

    Ok(())
}

#[cfg(test)]
mod test {
    use std::fs;

    use tempfile::{NamedTempFile, TempDir};
    use tokio::sync::OnceCell;
    use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};
    use vercel_api_mock::start_test_server;

    use crate::{
        cli::LinkTarget,
        commands::{link, CommandBase},
        config::{ClientConfigLoader, RepoConfigLoader, TurboJson, UserConfigLoader},
        ui::UI,
        Args,
    };

    #[tokio::test]
    async fn test_link_remote_cache() {
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();
        let repo_config_file = NamedTempFile::new().unwrap();
        let repo_config_path = AbsoluteSystemPathBuf::new(repo_config_file.path()).unwrap();
        fs::write(
            repo_config_file.path(),
            r#"{ "apiurl": "http://localhost:3000" }"#,
        )
        .unwrap();

        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let mut base = CommandBase {
            repo_root: Default::default(),
            ui: UI::new(false),
            client_config: OnceCell::from(ClientConfigLoader::new().load().unwrap()),
            user_config: OnceCell::from(
                UserConfigLoader::new(user_config_file.path().to_path_buf())
                    .with_token(Some("token".to_string()))
                    .load()
                    .unwrap(),
            ),
            repo_config: OnceCell::from(
                RepoConfigLoader::new(repo_config_path)
                    .with_api(Some(format!("http://localhost:{}", port)))
                    .with_login(Some(format!("http://localhost:{}", port)))
                    .load()
                    .unwrap(),
            ),
            args: Args::default(),
            version: "",
        };

        link::link(&mut base, false, LinkTarget::RemoteCache)
            .await
            .unwrap();

        handle.abort();
        let team_id = base.repo_config().unwrap().team_id();
        assert!(
            team_id == Some(vercel_api_mock::EXPECTED_USER_ID)
                || team_id == Some(vercel_api_mock::EXPECTED_TEAM_ID)
        );
    }

    #[tokio::test]
    async fn test_link_spaces() {
        // user config
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();

        // repo config
        let repo_config_file = NamedTempFile::new().unwrap();
        let repo_config_path = AbsoluteSystemPathBuf::new(repo_config_file.path()).unwrap();
        fs::write(
            repo_config_file.path(),
            r#"{ "apiurl": "http://localhost:3000" }"#,
        )
        .unwrap();

        let port = port_scanner::request_open_port().unwrap();
        let handle = tokio::spawn(start_test_server(port));
        let mut base = CommandBase {
            repo_root: AbsoluteSystemPathBuf::new(TempDir::new().unwrap().into_path()).unwrap(),
            ui: UI::new(false),
            client_config: OnceCell::from(ClientConfigLoader::new().load().unwrap()),
            user_config: OnceCell::from(
                UserConfigLoader::new(user_config_file.path().to_path_buf())
                    .with_token(Some("token".to_string()))
                    .load()
                    .unwrap(),
            ),
            repo_config: OnceCell::from(
                RepoConfigLoader::new(repo_config_path)
                    .with_api(Some(format!("http://localhost:{}", port)))
                    .with_login(Some(format!("http://localhost:{}", port)))
                    .load()
                    .unwrap(),
            ),
            args: Args::default(),
            version: "",
        };

        // turbo config
        let turbo_json_file = base
            .repo_root
            .join_relative(RelativeSystemPathBuf::new("turbo.json").expect("relative"));

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
        let turbo_json_file = fs::File::open(&turbo_json_file).unwrap();
        let turbo_json: TurboJson = serde_json::from_reader(turbo_json_file).unwrap();
        assert_eq!(
            turbo_json.experimental_spaces.unwrap().id.unwrap(),
            vercel_api_mock::EXPECTED_SPACE_ID
        );
    }
}
