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
use dialoguer::Select;
use dialoguer::{theme::ColorfulTheme, Confirm};
use dirs_next::home_dir;
#[cfg(test)]
use rand::Rng;

#[cfg(not(test))]
use crate::ui::CYAN;
use crate::{
    client::{APIClient, CachingStatus, Team},
    commands::CommandBase,
    ui::{BOLD, GREY, UNDERLINE},
};

#[derive(Clone)]
pub(crate) enum SelectedTeam<'a> {
    User,
    Team(&'a Team),
}

pub(crate) const REMOTE_CACHING_INFO: &str = "  Remote Caching shares your cached Turborepo task \
                                              outputs and logs across
  all your team’s Vercel projects. It also can share outputs
  with other services that enable Remote Caching, like CI/CD systems.
  This results in faster build times and deployments for your team.";
pub(crate) const REMOTE_CACHING_URL: &str =
    "https://turbo.build/repo/docs/core-concepts/remote-caching";

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

pub async fn link(base: &mut CommandBase, modify_gitignore: bool) -> Result<()> {
    let homedir_path = home_dir().ok_or_else(|| anyhow!("could not find home directory."))?;
    let homedir = homedir_path.to_string_lossy();
    println!(
        ">>> Remote Caching
{}
  For more info, see {}",
        REMOTE_CACHING_INFO,
        base.ui.apply(UNDERLINE.apply_to(REMOTE_CACHING_URL))
    );

    let repo_root_with_tilde = base.repo_root.to_string_lossy().replacen(&*homedir, "~", 1);

    if !should_link(base, &repo_root_with_tilde)? {
        return Err(anyhow!("canceled"));
    }

    let api_client = base.api_client()?;
    let token = base.user_config()?.token().ok_or_else(|| {
        anyhow!(
            "User not found. Please login to Turborepo first by running {}.",
            BOLD.apply_to("`npx turbo login`")
        )
    })?;

    let teams_response = api_client
        .get_teams(token)
        .await
        .context("could not get team information")?;

    let user_response = api_client
        .get_user(token)
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

    verify_caching_enabled(&api_client, team_id, token, Some(selected_team.clone())).await?;

    fs::create_dir_all(base.repo_root.join(".turbo"))
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

#[cfg(test)]
fn select_team<'a>(teams: &'a [Team], _: &'a str) -> Result<SelectedTeam<'a>> {
    let mut rng = rand::thread_rng();
    let idx = rng.gen_range(0..=(teams.len()));
    if idx == teams.len() {
        Ok(SelectedTeam::User)
    } else {
        Ok(SelectedTeam::Team(&teams[idx]))
    }
}

#[cfg(not(test))]
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

#[cfg(test)]
fn should_link(_: &CommandBase, _: &str) -> Result<bool> {
    Ok(true)
}

#[cfg(not(test))]
fn should_link(base: &CommandBase, location: &str) -> Result<bool> {
    let prompt = format!(
        "{}{} {}",
        base.ui.apply(BOLD.apply_to(GREY.apply_to("? "))),
        base.ui
            .apply(BOLD.apply_to("Would you like to enable Remote Caching for")),
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

#[cfg(test)]
mod test {
    use std::{fs, net::SocketAddr};

    use anyhow::Result;
    use axum::{routing::get, Json, Router};
    use tempfile::NamedTempFile;
    use tokio::sync::OnceCell;

    use crate::{
        client::{
            CachingStatus, CachingStatusResponse, Membership, Role, Team, TeamsResponse, User,
            UserResponse,
        },
        commands::{link, CommandBase},
        config::{RepoConfigLoader, UserConfigLoader},
        ui::UI,
        Args,
    };

    const TEAM_ID: &str = "vercel";
    const USER_ID: &str = "my-user-id";

    #[tokio::test]
    async fn test_link() {
        let user_config_file = NamedTempFile::new().unwrap();
        fs::write(user_config_file.path(), r#"{ "token": "hello" }"#).unwrap();
        let repo_config_file = NamedTempFile::new().unwrap();
        fs::write(
            repo_config_file.path(),
            r#"{ "apiurl": "http://localhost:3000" }"#,
        )
        .unwrap();

        let handle = tokio::spawn(start_test_server());
        let mut base = CommandBase {
            repo_root: Default::default(),
            ui: UI::new(false),
            user_config: OnceCell::from(
                UserConfigLoader::new(user_config_file.path().to_path_buf())
                    .with_token(Some("token".to_string()))
                    .load()
                    .unwrap(),
            ),
            repo_config: OnceCell::from(
                RepoConfigLoader::new(repo_config_file.path().to_path_buf())
                    .with_api(Some("http://localhost:3000".to_string()))
                    .with_login(Some("http://localhost:3000".to_string()))
                    .load()
                    .unwrap(),
            ),
            args: Args::default(),
        };

        link::link(&mut base, false).await.unwrap();

        handle.abort();
        let team_id = base.repo_config().unwrap().team_id();
        assert!(team_id == Some(TEAM_ID) || team_id == Some(USER_ID));
    }

    async fn start_test_server() -> Result<()> {
        let app = Router::new()
            // `GET /` goes to `root`
            .route(
                "/v2/teams",
                get(|| async move {
                    Json(TeamsResponse {
                        teams: vec![Team {
                            id: TEAM_ID.to_string(),
                            slug: "vercel".to_string(),
                            name: "vercel".to_string(),
                            created_at: 0,
                            created: Default::default(),
                            membership: Membership::new(Role::Owner),
                        }],
                    })
                }),
            )
            .route(
                "/v2/user",
                get(|| async move {
                    Json(UserResponse {
                        user: User {
                            id: USER_ID.to_string(),
                            username: "my_username".to_string(),
                            email: "my_email".to_string(),
                            name: None,
                            created_at: Some(0),
                        },
                    })
                }),
            )
            .route(
                "/v8/artifacts/status",
                get(|| async {
                    Json(CachingStatusResponse {
                        status: CachingStatus::Enabled,
                    })
                }),
            );
        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Ok(axum_server::bind(addr)
            .serve(app.into_make_service())
            .await?)
    }
}
